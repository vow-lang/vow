use std::collections::HashSet;
use std::path::{Path, PathBuf};

use vow_diag::{Blame, Diagnostic, ErrorCode, Severity, SourceLocation};
use vow_syntax::ast::Module;

pub(crate) struct ModuleGraph {
    /// Modules in dependency-first order; root is last.
    pub modules: Vec<(PathBuf, Module)>,
}

/// Load the module graph for `root`, optionally overriding the directory used
/// to resolve `use` declarations.
///
/// When `module_root` is `None`, `use` paths resolve relative to `root.parent()`
/// — the default for `vow build`/`verify`. When `Some`, the supplied directory
/// is used as the resolution base for the root file *and* every transitively
/// loaded dependency. `vowc test` uses this so a test at
/// `compiler/tests/test_region.vow` can `use region;` and resolve against
/// `compiler/region.vow` rather than the non-existent `compiler/tests/region.vow`.
pub(crate) fn load_modules_with_root(
    root: &Path,
    module_root: Option<&Path>,
    root_ast: &Module,
) -> Result<ModuleGraph, Vec<Diagnostic>> {
    let root_dir = module_root.unwrap_or_else(|| root.parent().unwrap_or(root));
    let mut modules: Vec<(PathBuf, Module)> = Vec::new();
    let mut visited: HashSet<PathBuf> = HashSet::new();
    let mut errors: Vec<Diagnostic> = Vec::new();

    visited.insert(root.to_path_buf());
    load_deps(root_dir, root_ast, &mut modules, &mut visited, &mut errors);
    modules.push((root.to_path_buf(), root_ast.clone()));

    if !errors.is_empty() {
        Err(errors)
    } else {
        Ok(ModuleGraph { modules })
    }
}

fn load_deps(
    root_dir: &Path,
    module: &Module,
    modules: &mut Vec<(PathBuf, Module)>,
    visited: &mut HashSet<PathBuf>,
    errors: &mut Vec<Diagnostic>,
) {
    for use_decl in &module.uses {
        let vow_path = resolve_use(root_dir, &use_decl.path);
        let decl_path = vow_path.with_extension("vow.d");
        let file_path = if decl_path.exists() {
            decl_path
        } else {
            vow_path
        };
        if !visited.insert(file_path.clone()) {
            continue;
        }
        match std::fs::read_to_string(&file_path) {
            Ok(src) => {
                let file_str = file_path.to_string_lossy();
                let (dep_ast, diags) = vow_syntax::parser::parse_module(&src, &file_str);
                let has_error = diags.iter().any(|d| d.severity == Severity::Error);
                if has_error {
                    errors.extend(diags);
                } else {
                    load_deps(root_dir, &dep_ast, modules, visited, errors);
                    modules.push((file_path, dep_ast));
                }
            }
            Err(e) => {
                errors.push(Diagnostic {
                    severity: Severity::Error,
                    code: ErrorCode::IoError,
                    message: format!("cannot load module `{}`: {e}", use_decl.path.join(".")),
                    primary: SourceLocation {
                        file: use_decl.path.join("."),
                        byte_offset: use_decl.span.start,
                        byte_len: use_decl.span.len,
                    },
                    secondary: vec![],
                    blame: Blame::None,
                    hints: vec![],
                });
            }
        }
    }
}

fn resolve_use(root_dir: &Path, path: &[String]) -> PathBuf {
    let mut result = root_dir.to_path_buf();
    for component in path {
        result = result.join(component);
    }
    result.with_extension("vow")
}

/// Merge all modules into a single Module for unified type-checking and lowering.
/// All items from dependency modules are visible as if declared in the root.
///
/// Returns the merged module plus a parallel `Vec<String>` of source-file
/// paths — `item_files[i]` is the originating file path of `items[i]`.
/// This per-item provenance is consumed by `vow_ir::lower_module` to set
/// `Function.source_file` so region diagnostics label the right file under
/// multi-module compilation (#254).
pub(crate) fn merge_modules(graph: ModuleGraph) -> (Module, Vec<String>) {
    let (_, root_module) = graph
        .modules
        .last()
        .cloned()
        .unwrap_or_else(|| panic!("empty module graph"));

    let mut all_items = Vec::new();
    let mut item_files: Vec<String> = Vec::new();
    for (path, module) in &graph.modules {
        let path_str = path.to_string_lossy().into_owned();
        for item in &module.items {
            all_items.push(item.clone());
            item_files.push(path_str.clone());
        }
    }

    let merged = Module {
        name: root_module.name,
        uses: vec![],
        items: all_items,
        span: root_module.span,
    };
    (merged, item_files)
}
