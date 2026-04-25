use std::path::{Path, PathBuf};
use std::sync::Arc;

use vow_diag::{CollectingEmitter, Diagnostic, DiagnosticEmitter, Severity};
use vow_syntax::ast::Module;

use crate::module_loader;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum FrontendGoal {
    MergedAst,
    LoweredIr,
}

#[derive(Debug, Clone)]
pub(crate) struct DependencyManifest {
    source_files: Vec<PathBuf>,
}

impl DependencyManifest {
    pub(crate) fn from_paths(source_files: Vec<PathBuf>) -> Self {
        Self { source_files }
    }

    pub(crate) fn paths(&self) -> &[PathBuf] {
        &self.source_files
    }
}

#[derive(Debug)]
pub(crate) struct FrontendBundle {
    diagnostics: Vec<Diagnostic>,
    module: Module,
    deps: DependencyManifest,
    ir: Option<Arc<vow_ir::Module>>,
}

impl FrontendBundle {
    pub(crate) fn diagnostics(&self) -> &[Diagnostic] {
        &self.diagnostics
    }

    pub(crate) fn module(&self) -> &Module {
        &self.module
    }

    pub(crate) fn dependencies(&self) -> &DependencyManifest {
        &self.deps
    }

    pub(crate) fn ir(&self) -> Option<&Arc<vow_ir::Module>> {
        self.ir.as_ref()
    }
}

#[derive(Debug)]
pub(crate) enum FrontendError {
    Io(String),
    Diagnostics {
        stage: FrontendStage,
        diagnostics: Vec<Diagnostic>,
    },
}

impl FrontendError {
    pub(crate) fn diagnostics(&self) -> &[Diagnostic] {
        match self {
            Self::Io(_) => &[],
            Self::Diagnostics { diagnostics, .. } => diagnostics,
        }
    }

    pub(crate) fn into_diagnostics(self) -> Vec<Diagnostic> {
        match self {
            Self::Io(_) => vec![],
            Self::Diagnostics { diagnostics, .. } => diagnostics,
        }
    }

    pub(crate) fn failure_message(&self) -> &str {
        match self {
            Self::Io(message) => message.as_str(),
            Self::Diagnostics { stage, .. } => stage.failure_message(),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum FrontendStage {
    Parse,
    ModuleLoad,
    TypeCheck,
    RegionInference,
}

impl FrontendStage {
    fn failure_message(self) -> &'static str {
        match self {
            Self::Parse => "parse error",
            Self::ModuleLoad => "module load error",
            Self::TypeCheck => "type error",
            Self::RegionInference => "region error",
        }
    }
}

struct NullEmitter;

impl DiagnosticEmitter for NullEmitter {
    fn emit(&mut self, _: &Diagnostic) {}

    fn finish(&mut self) {}
}

pub(crate) fn prepare_frontend(
    source: &Path,
    goal: FrontendGoal,
) -> Result<FrontendBundle, FrontendError> {
    let src = std::fs::read_to_string(source).map_err(|e| FrontendError::Io(e.to_string()))?;
    let file_str = source.to_string_lossy();

    let mut diagnostics = Vec::new();
    let (root_ast, parse_diags) = vow_syntax::parser::parse_module(&src, &file_str);
    let parse_failed = parse_diags.iter().any(|d| d.severity == Severity::Error);
    diagnostics.extend(parse_diags);
    if parse_failed {
        return Err(FrontendError::Diagnostics {
            stage: FrontendStage::Parse,
            diagnostics,
        });
    }

    let graph = match module_loader::load_modules(source, &root_ast) {
        Ok(graph) => graph,
        Err(diags) => {
            diagnostics.extend(diags);
            return Err(FrontendError::Diagnostics {
                stage: FrontendStage::ModuleLoad,
                diagnostics,
            });
        }
    };

    let deps = DependencyManifest::from_paths(
        graph.modules.iter().map(|(path, _)| path.clone()).collect(),
    );
    let ast = module_loader::merge_modules(graph);

    let mut null_emit = NullEmitter;
    let mut collecting_emit = CollectingEmitter::new(&mut null_emit);
    let mut checker =
        vow_types::check::Checker::new(source.to_string_lossy().to_string(), &mut collecting_emit);
    checker.check_module(&ast);
    let has_errors = checker.has_errors();
    let string_exprs = if matches!(goal, FrontendGoal::LoweredIr) && !has_errors {
        Some(checker.into_string_exprs())
    } else {
        drop(checker);
        None
    };
    diagnostics.extend(collecting_emit.into_diagnostics());
    if has_errors {
        return Err(FrontendError::Diagnostics {
            stage: FrontendStage::TypeCheck,
            diagnostics,
        });
    }

    let ir = match goal {
        FrontendGoal::MergedAst => None,
        FrontendGoal::LoweredIr => {
            let string_exprs = string_exprs.expect("LoweredIr goal must preserve string exprs");
            let mut module = vow_ir::lower_module(&ast, &source.to_string_lossy(), &string_exprs);
            // Track lower-warning count so region inference does not see them
            // as its own (and the post-pass error check below only reacts to
            // newly-added Severity::Error diagnostics from infer_regions).
            let lower_warn_count = module.warnings.len();
            // Phase 3: region inference (arena-per-scope). Runs after type/
            // effect/linear checks (above) and before any consumer of region
            // metadata. Pushes any RegionConflict diagnostics into
            // `module.warnings`.
            vow_ir::infer_regions(&mut module);
            diagnostics.extend(module.warnings.iter().cloned());
            // If region inference emitted any errors, fail compilation here so
            // the build pipeline reports CompileFailed (spec §4.4 — rejection,
            // not over-approximation).
            let region_has_errors = module
                .warnings
                .iter()
                .skip(lower_warn_count)
                .any(|d| d.severity == Severity::Error);
            if region_has_errors {
                return Err(FrontendError::Diagnostics {
                    stage: FrontendStage::RegionInference,
                    diagnostics,
                });
            }
            // Phase 4 / S3: insert RegionOpen / RegionClose markers around
            // every block whose region is non-empty (spec §3.5).
            vow_ir::insert_region_markers(&mut module);
            Some(Arc::new(module))
        }
    };

    Ok(FrontendBundle {
        diagnostics,
        module: ast,
        deps,
        ir,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    fn write_vow(dir: &TempDir, name: &str, src: &str) -> PathBuf {
        let path = dir.path().join(name);
        std::fs::write(&path, src).unwrap();
        path
    }

    #[test]
    fn merged_ast_goal_merges_dependency_items() {
        let dir = TempDir::new().unwrap();
        write_vow(&dir, "lib.vow", "module Lib\nfn helper() -> i64 { 0 }");
        let root = write_vow(
            &dir,
            "main.vow",
            "module Main\nuse lib\nfn main_fn() -> i64 { helper() }",
        );

        let bundle = prepare_frontend(&root, FrontendGoal::MergedAst).unwrap();

        assert_eq!(bundle.module().name, "Main");
        assert!(bundle.module().uses.is_empty());
        assert_eq!(bundle.module().items.len(), 2);
        assert!(bundle.ir().is_none());
        assert_eq!(bundle.dependencies().paths().len(), 2);
        assert!(
            bundle
                .dependencies()
                .paths()
                .iter()
                .any(|path| path.ends_with("lib.vow"))
        );
    }

    #[test]
    fn lowered_ir_goal_returns_ir_and_dependency_manifest() {
        let dir = TempDir::new().unwrap();
        write_vow(&dir, "lib.vow", "module Lib\nfn helper() -> i64 { 0 }");
        let root = write_vow(
            &dir,
            "main.vow",
            "module Main\nuse lib\nfn main_fn() -> i64 { helper() }",
        );

        let bundle = prepare_frontend(&root, FrontendGoal::LoweredIr).unwrap();

        assert!(bundle.diagnostics().is_empty());
        assert!(bundle.ir().is_some());
        assert_eq!(bundle.dependencies().paths().len(), 2);
        assert!(
            bundle
                .dependencies()
                .paths()
                .iter()
                .any(|path| path.ends_with("main.vow"))
        );
    }

    #[test]
    fn missing_import_reports_module_load_error() {
        let dir = TempDir::new().unwrap();
        let root = write_vow(
            &dir,
            "main.vow",
            "module Main\nuse missing\nfn main_fn() -> i64 { 0 }",
        );

        let error = prepare_frontend(&root, FrontendGoal::MergedAst).unwrap_err();

        assert_eq!(error.failure_message(), "module load error");
        assert!(
            error
                .diagnostics()
                .iter()
                .any(|diag| diag.message.contains("cannot load module `missing`"))
        );
    }

    #[test]
    fn merged_ast_and_lowered_ir_share_typecheck_rules() {
        let dir = TempDir::new().unwrap();
        write_vow(&dir, "lib.vow", "module Lib\nfn helper() -> i64 { 0 }");
        let root = write_vow(
            &dir,
            "main.vow",
            "module Main\nuse lib\nfn main_fn() -> i64 { true }",
        );

        let merged = prepare_frontend(&root, FrontendGoal::MergedAst).unwrap_err();
        let lowered = prepare_frontend(&root, FrontendGoal::LoweredIr).unwrap_err();

        assert_eq!(merged.failure_message(), "type error");
        assert_eq!(lowered.failure_message(), "type error");
        assert_eq!(merged.diagnostics().len(), lowered.diagnostics().len());
        assert_eq!(
            merged.diagnostics()[0].message,
            lowered.diagnostics()[0].message
        );
    }
}
