use std::collections::HashSet;
use std::path::{Path, PathBuf};

use vow_diag::{Blame, Diagnostic, ErrorCode, Severity, SourceLocation};
use vow_syntax::ast::Module;

pub struct ModuleGraph {
    /// Modules in dependency-first order; root is last.
    pub modules: Vec<(PathBuf, Module)>,
}

pub fn load_modules(root: &Path, root_ast: &Module) -> Result<ModuleGraph, Vec<Diagnostic>> {
    let root_dir = root.parent().unwrap_or(root);
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
                    code: ErrorCode::TypeMismatch,
                    message: format!("cannot load module `{}`: {e}", use_decl.path.join(".")),
                    primary: SourceLocation {
                        file: use_decl.path.join("."),
                        byte_offset: use_decl.span.start,
                        byte_len: use_decl.span.len,
                    },
                    secondary: vec![],
                    blame: Blame::None,
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
    fn load_modules_missing_import_returns_error() {
        let dir = TempDir::new().unwrap();
        let src = "module Main\nuse nonexistent\nfn f() -> i32 { 0 }";
        let path = write_vow(&dir, "main.vow", src);
        let (ast, diags) = vow_syntax::parser::parse_module(src, &path.to_string_lossy());
        assert!(diags.is_empty());
        let result = load_modules(&path, &ast);
        assert!(result.is_err(), "should fail when import not found");
        let errors = result.err().unwrap();
        assert!(!errors.is_empty());
        assert!(errors[0].message.contains("nonexistent"));
    }

    #[test]
    fn load_modules_duplicate_import_not_loaded_twice() {
        let dir = TempDir::new().unwrap();
        write_vow(&dir, "lib.vow", "module Lib\nfn helper() -> i32 { 0 }");
        let src = "module Main\nuse lib\nuse lib\nfn f() -> i32 { 0 }";
        let path = write_vow(&dir, "main.vow", src);
        let (ast, diags) = vow_syntax::parser::parse_module(src, &path.to_string_lossy());
        assert!(diags.is_empty());
        let result = load_modules(&path, &ast);
        assert!(result.is_ok(), "should succeed with duplicate import");
        let graph = result.unwrap();
        let lib_count = graph
            .modules
            .iter()
            .filter(|(p, _)| p.ends_with("lib.vow"))
            .count();
        assert_eq!(lib_count, 1, "lib should only appear once");
    }

    #[test]
    fn merge_modules_combines_items() {
        let dir = TempDir::new().unwrap();
        write_vow(&dir, "lib.vow", "module Lib\nfn helper() -> i32 { 0 }");
        let src = "module Main\nuse lib\nfn main_fn() -> i32 { 0 }";
        let path = write_vow(&dir, "main.vow", src);
        let (ast, _) = vow_syntax::parser::parse_module(src, &path.to_string_lossy());
        let graph = load_modules(&path, &ast).unwrap();
        let merged = merge_modules(graph);
        assert_eq!(merged.name, "Main");
        assert_eq!(merged.items.len(), 2, "should have helper + main_fn");
        assert!(merged.uses.is_empty(), "merged module has no uses");
    }
}

/// Merge all modules into a single Module for unified type-checking and lowering.
/// All items from dependency modules are visible as if declared in the root.
pub fn merge_modules(graph: ModuleGraph) -> Module {
    let (_, root_module) = graph
        .modules
        .last()
        .cloned()
        .unwrap_or_else(|| panic!("empty module graph"));

    let mut all_items = Vec::new();
    for (_, module) in &graph.modules {
        all_items.extend(module.items.clone());
    }

    Module {
        name: root_module.name,
        uses: vec![],
        items: all_items,
        span: root_module.span,
    }
}
