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

    #[cfg(test)]
    pub(crate) fn dependencies(&self) -> &DependencyManifest {
        &self.deps
    }

    pub(crate) fn ir(&self) -> Option<&Arc<vow_ir::Module>> {
        self.ir.as_ref()
    }

    // Consume the bundle, dropping the AST `Module` field, and return only
    // the parts the build pipeline still needs after lowering. Lets callers
    // free the largest leftover frontend allocation right after IR
    // extraction instead of carrying it through codegen + verify. See #178.
    pub(crate) fn into_parts(
        self,
    ) -> (
        Vec<Diagnostic>,
        Option<Arc<vow_ir::Module>>,
        DependencyManifest,
    ) {
        (self.diagnostics, self.ir, self.deps)
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
    fn try_emit(&mut self, _: &Diagnostic) -> std::io::Result<()> {
        Ok(())
    }

    fn try_finish(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

pub(crate) fn prepare_frontend(
    source: &Path,
    goal: FrontendGoal,
) -> Result<FrontendBundle, FrontendError> {
    prepare_frontend_with_root(source, None, goal)
}

/// Same as `prepare_frontend`, but resolves `use` declarations against
/// `module_root` instead of the entry file's parent directory.
///
/// Used by `vowc test` so a test at `compiler/tests/test_region.vow` can
/// `use region;` and resolve against `compiler/region.vow`.
pub(crate) fn prepare_frontend_with_root(
    source: &Path,
    module_root: Option<&Path>,
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

    let graph = match module_loader::load_modules_with_root(source, module_root, &root_ast) {
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
    let (ast, item_files) = module_loader::merge_modules(graph);

    let mut null_emit = NullEmitter;
    let mut collecting_emit = CollectingEmitter::new(&mut null_emit);
    let mut checker =
        vow_types::check::Checker::new(source.to_string_lossy().to_string(), &mut collecting_emit);
    checker.check_module(&ast, &item_files);
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
            let mut module = vow_ir::lower_module(&ast, &item_files, &string_exprs);
            // Track lower-warning count so region inference does not see them
            // as its own (and the post-pass error check below only reacts to
            // newly-added Severity::Error diagnostics from infer_regions).
            let lower_warn_count = module.warnings.len();
            // Phase 3: region inference (arena-per-scope). Runs after type/
            // effect/linear checks (above) and before any consumer of region
            // metadata. Pushes any RegionConflict diagnostics into
            // `module.warnings`. Diagnostic file labels come from each
            // `Function.source_file` (set by `lower_module`), not a single
            // shared root path — see #254.
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
    use vow_diag::ErrorCode;

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
    fn module_root_override_resolves_use_against_sibling_directory() {
        // Mirrors the `vowc test compiler/` use case: a test file in a
        // subdirectory imports a module that lives one level up. Default
        // resolution (entry parent) would fail; module_root override should
        // succeed.
        let dir = TempDir::new().unwrap();
        write_vow(&dir, "lib.vow", "module Lib\nfn helper() -> i64 { 0 }");
        let tests_dir = dir.path().join("tests");
        std::fs::create_dir(&tests_dir).unwrap();
        let test_path = tests_dir.join("test_lib.vow");
        std::fs::write(
            &test_path,
            "module TestLib\nuse lib\nfn main() -> i64 { helper() }",
        )
        .unwrap();

        // Without override: should fail to find `lib` in tests/.
        let default = prepare_frontend(&test_path, FrontendGoal::MergedAst).unwrap_err();
        assert_eq!(default.failure_message(), "module load error");

        // With override: resolves against `dir.path()` and finds lib.vow.
        let bundle =
            prepare_frontend_with_root(&test_path, Some(dir.path()), FrontendGoal::MergedAst)
                .unwrap();
        assert_eq!(bundle.module().name, "TestLib");
        assert_eq!(bundle.dependencies().paths().len(), 2);
        assert!(
            bundle
                .dependencies()
                .paths()
                .iter()
                .any(|p| p.ends_with("lib.vow"))
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

    #[test]
    fn typecheck_error_in_dependency_module_reports_dep_file_and_valid_offset() {
        // Regression for issue 520: TypeMismatch diagnostics on items merged
        // from a `use`d module used to be labelled with the root file path and
        // an offset that belonged to a *different* file's coordinate space,
        // landing in random bytes (often comments) of the root source.
        //
        // The mismatch site here is `fn bad() -> i32 { true }` in lib.vow,
        // which triggers `check.rs:550` — span = fn_def.body.span, file-local
        // to lib.vow. The diagnostic must now report lib.vow as the file and
        // an offset that falls inside lib.vow's actual source (the body of
        // `bad`, which contains `true`).
        let dir = TempDir::new().unwrap();
        let lib_src = "module Lib\nfn bad() -> i32 { true }\n";
        write_vow(&dir, "lib.vow", lib_src);
        let root = write_vow(
            &dir,
            "main.vow",
            "module Main\nuse lib\nfn main_fn() -> i32 { bad() }\n",
        );

        let error = prepare_frontend(&root, FrontendGoal::MergedAst).unwrap_err();
        let diag = error
            .diagnostics()
            .iter()
            .find(|d| d.severity == Severity::Error && d.code == ErrorCode::TypeMismatch)
            .expect("expected at least one TypeMismatch diagnostic");

        assert!(
            diag.primary.file.ends_with("lib.vow"),
            "diagnostic file should be lib.vow but is `{}`",
            diag.primary.file
        );

        let offset = diag.primary.byte_offset as usize;
        let len = diag.primary.byte_len as usize;
        assert!(
            offset + len <= lib_src.len(),
            "span {offset}..{} exceeds lib.vow source length {}",
            offset + len,
            lib_src.len()
        );
        let slice = std::str::from_utf8(&lib_src.as_bytes()[offset..offset + len])
            .expect("span should slice on UTF-8 boundaries");
        assert!(
            slice.contains("true"),
            "span text `{slice}` should contain the offending body token `true`"
        );
    }
}
