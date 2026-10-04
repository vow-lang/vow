use std::collections::HashSet;
use std::path::{Component, Path, PathBuf};

use vow_diag::{Blame, Diagnostic, ErrorCode, Severity, SourceLocation};
use vow_syntax::ast::{Item, Module, UseDecl};

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
        let file_path = module_file_for_use(root_dir, &use_decl.path, |p| p.exists());
        if !visited.insert(file_path.clone()) {
            continue;
        }
        match load_dep_module(root_dir, use_decl, &file_path) {
            Ok((final_path, dep_ast)) => {
                load_deps(root_dir, &dep_ast, modules, visited, errors);
                modules.push((final_path, dep_ast));
            }
            Err(diags) => errors.extend(diags),
        }
    }
}

/// Load the dependency `module_file_for_use` chose for `use_decl`. When
/// `file_path` is a `.vow.d` stub that parses but carries a declaration with
/// a `vow` block the verifier cannot enforce from a bodyless signature, fall
/// back to the sibling `.vow` source instead (#1472) — but only when that
/// source actually exists. A library shipping only a stub keeps today's
/// behavior unchanged: the call becomes non-modelable downstream, never
/// falsely `Verified`. A stub that fails to parse is reported as-is and never
/// triggers the source fallback: parse robustness of `.vow.d` stubs is a
/// separate concern from #1472's contract-hiding bug, and neither
/// `docs/spec/grammar.md` nor `docs/spec/stdlib.md` documents a parse-failure
/// fallback.
fn load_dep_module(
    root_dir: &Path,
    use_decl: &UseDecl,
    file_path: &Path,
) -> Result<(PathBuf, Module), Vec<Diagnostic>> {
    let vow_path = resolve_use(root_dir, &use_decl.path);
    if file_path == vow_path {
        return read_and_parse(file_path, use_decl);
    }

    let stub_result = read_and_parse(file_path, use_decl);
    if let Ok((_, module)) = &stub_result
        && module_has_unenforceable_contract(module)
        && vow_path.exists()
    {
        return read_and_parse(&vow_path, use_decl);
    }
    stub_result
}

fn read_and_parse(
    file_path: &Path,
    use_decl: &UseDecl,
) -> Result<(PathBuf, Module), Vec<Diagnostic>> {
    match std::fs::read_to_string(file_path) {
        Ok(src) => {
            let file_str = file_path.to_string_lossy();
            let (ast, diags) = vow_syntax::parser::parse_module(&src, &file_str);
            if diags.iter().any(|d| d.severity == Severity::Error) {
                Err(diags)
            } else {
                Ok((file_path.to_path_buf(), ast))
            }
        }
        Err(e) => Err(vec![Diagnostic {
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
        }]),
    }
}

/// True when `module` contains a declaration-only function (top-level or an
/// `impl` method) carrying a `vow` block. Such a function has no body for the
/// verifier to check a call site against — see #1472. Visibility is
/// deliberately ignored: the self-hosted AST has no visibility slot at all,
/// so a visibility-aware predicate here would be an unreproducible parity
/// divergence from the self-hosted loader.
fn module_has_unenforceable_contract(module: &Module) -> bool {
    let fn_is_unenforceable = |f: &vow_syntax::ast::FnDef| f.is_declaration && f.vow.is_some();
    module.items.iter().any(|item| match item {
        Item::Fn(f) => fn_is_unenforceable(f),
        Item::Impl(i) => i.methods.iter().any(fn_is_unenforceable),
        _ => false,
    })
}

/// Resolve a dotted `use` path to the file to load, relative to `root_dir`,
/// preferring a sibling `.vow.d` declaration stub over the full `.vow` source
/// when one exists. This choice is not final: `load_dep_module` may still
/// override it and load the `.vow` source instead when the stub it names
/// carries a contract the verifier cannot enforce (#1472).
///
/// `decl_exists` is the on-disk existence check for the derived `.vow.d` path,
/// injected so the resolution decision is testable without touching the
/// filesystem. The production caller passes `|p| p.exists()`.
fn module_file_for_use(
    root_dir: &Path,
    path: &[String],
    decl_exists: impl Fn(&Path) -> bool,
) -> PathBuf {
    let vow_path = resolve_use(root_dir, path);
    let decl_path = vow_path.with_extension("vow.d");
    if decl_exists(&decl_path) {
        decl_path
    } else {
        vow_path
    }
}

/// Infer the module root for a single entry file that was given without an
/// explicit `--module-root`: the nearest directory, starting at the entry's own
/// directory and walking up through its ancestors, against which every direct
/// `use` of the entry resolves (to a `.vow` or `.vow.d` file).
///
/// The walk stops after the first directory containing `.git` (the repository
/// root), at a `..` component (whose parent is unknown without canonicalizing),
/// or when the path runs out; a relative entry path ends at `.`. Returns
/// `None` when the entry's own directory already works, when the entry has no
/// `use` declarations, or when no directory resolves them all — in each case the
/// default resolution (the entry's parent directory) applies unchanged.
/// `exists` is injected so the rule is testable without touching the filesystem.
pub(crate) fn infer_module_root(
    entry: &Path,
    uses: &[Vec<String>],
    exists: impl Fn(&Path) -> bool,
) -> Option<PathBuf> {
    if uses.is_empty() {
        return None;
    }
    let resolves = |dir: &Path| {
        uses.iter()
            .all(|u| exists(&module_file_for_use(dir, u, &exists)))
    };
    let mut first = true;
    for dir in entry.parent()?.ancestors() {
        let dir = if dir.as_os_str().is_empty() {
            Path::new(".")
        } else {
            dir
        };
        if resolves(dir) {
            return if first { None } else { Some(dir.to_path_buf()) };
        }
        if exists(&dir.join(".git")) || dir.components().next_back() == Some(Component::ParentDir) {
            return None;
        }
        first = false;
    }
    None
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

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::RefCell;

    fn comps(parts: &[&str]) -> Vec<String> {
        parts.iter().map(|s| s.to_string()).collect()
    }

    fn parse_ok(src: &str, file: &str) -> Module {
        let (module, diags) = vow_syntax::parser::parse_module(src, file);
        assert!(
            diags.is_empty(),
            "unexpected diagnostics for {file}: {diags:?}"
        );
        module
    }

    #[test]
    fn module_has_unenforceable_contract_detects_declaration_with_requires() {
        let m = parse_ok(
            "module M pub fn f(x: i64) -> i64 vow { requires: x > 0 };",
            "m.vow.d",
        );
        assert!(module_has_unenforceable_contract(&m));
    }

    #[test]
    fn module_has_unenforceable_contract_false_for_contract_free_declaration() {
        let m = parse_ok("module M pub fn f(x: i64) -> i64;", "m.vow.d");
        assert!(!module_has_unenforceable_contract(&m));
    }

    #[test]
    fn module_has_unenforceable_contract_false_for_non_declaration_with_vow() {
        let m = parse_ok(
            "module M pub fn f(x: i64) -> i64 vow { requires: x > 0 } { x }",
            "m.vow",
        );
        assert!(!module_has_unenforceable_contract(&m));
    }

    #[test]
    fn module_has_unenforceable_contract_detects_impl_method_declaration() {
        // Built directly rather than parsed: `parse_impl` does not yet accept
        // `;`-terminated method declarations at all (a separate, pre-existing
        // gap independent of #1472), so there is no source text today that
        // would round-trip into this shape.
        use vow_syntax::ast::{
            BinOp, Expr, ExprKind, FnDef, ImplBlock, Item, Lit, Type, Visibility, VowBlock,
            VowClause,
        };
        use vow_syntax::span::Span;

        let z = || Span::new(0, 0);
        let requires = VowClause::Requires {
            expr: Expr {
                kind: ExprKind::BinaryOp {
                    op: BinOp::Gt,
                    lhs: Box::new(Expr {
                        kind: ExprKind::Ident("x".to_string()),
                        span: z(),
                    }),
                    rhs: Box::new(Expr {
                        kind: ExprKind::Lit(Lit::Int(0)),
                        span: z(),
                    }),
                },
                span: z(),
            },
            span: z(),
        };
        let method = FnDef {
            vis: Visibility::Private,
            name: "f".to_string(),
            params: vec![],
            return_ty: Type::Named {
                name: "i64".to_string(),
                span: z(),
            },
            effects: vec![],
            vow: Some(VowBlock {
                clauses: vec![requires],
                span: z(),
            }),
            body: vow_syntax::ast::Block {
                stmts: vec![],
                trailing_expr: None,
                span: z(),
            },
            span: z(),
            is_declaration: true,
        };
        let m = Module {
            name: "M".to_string(),
            uses: vec![],
            items: vec![Item::Impl(ImplBlock {
                trait_name: None,
                self_ty: Type::Named {
                    name: "Foo".to_string(),
                    span: z(),
                },
                methods: vec![method],
                span: z(),
            })],
            span: z(),
        };
        assert!(module_has_unenforceable_contract(&m));
    }

    #[test]
    fn single_component_without_decl_resolves_to_vow_source() {
        let file = module_file_for_use(Path::new("/proj"), &comps(&["region"]), |_| false);
        assert_eq!(file, PathBuf::from("/proj/region.vow"));
    }

    #[test]
    fn single_component_with_decl_prefers_decl_stub() {
        let file = module_file_for_use(Path::new("/proj"), &comps(&["region"]), |_| true);
        assert_eq!(file, PathBuf::from("/proj/region.vow.d"));
    }

    #[test]
    fn multi_component_path_nests_directories() {
        let file = module_file_for_use(Path::new("/proj"), &comps(&["a", "b"]), |_| false);
        assert_eq!(file, PathBuf::from("/proj/a/b.vow"));
    }

    #[test]
    fn multi_component_path_with_decl_prefers_nested_decl_stub() {
        let file = module_file_for_use(Path::new("/proj"), &comps(&["a", "b"]), |_| true);
        assert_eq!(file, PathBuf::from("/proj/a/b.vow.d"));
    }

    #[test]
    fn existence_is_queried_with_the_decl_path() {
        let seen: RefCell<Option<PathBuf>> = RefCell::new(None);
        let _ = module_file_for_use(Path::new("/proj"), &comps(&["region"]), |p| {
            *seen.borrow_mut() = Some(p.to_path_buf());
            false
        });
        assert_eq!(seen.into_inner(), Some(PathBuf::from("/proj/region.vow.d")));
    }

    #[test]
    fn infer_root_climbs_to_the_ancestor_that_resolves_every_use() {
        let exists = |p: &Path| {
            p == Path::new("proj/compiler/region.vow")
                || p == Path::new("proj/compiler/tests/builders.vow")
        };
        let uses = vec![comps(&["region"]), comps(&["tests", "builders"])];
        let root = infer_module_root(Path::new("proj/compiler/tests/test_x.vow"), &uses, exists);
        assert_eq!(root, Some(PathBuf::from("proj/compiler")));
    }

    #[test]
    fn infer_root_is_none_when_the_entry_directory_already_resolves() {
        let exists = |p: &Path| p == Path::new("proj/sub/dep.vow");
        let uses = vec![comps(&["dep"])];
        assert_eq!(
            infer_module_root(Path::new("proj/sub/test_x.vow"), &uses, exists),
            None
        );
    }

    #[test]
    fn infer_root_is_none_without_uses_or_without_a_match() {
        assert_eq!(
            infer_module_root(Path::new("a/b/test_x.vow"), &[], |_| true),
            None
        );
        assert_eq!(
            infer_module_root(Path::new("a/b/test_x.vow"), &[comps(&["nope"])], |_| false),
            None
        );
    }

    #[test]
    fn infer_root_accepts_decl_stubs_and_ends_at_the_current_directory() {
        let exists = |p: &Path| p == Path::new("./dep.vow.d");
        let uses = vec![comps(&["dep"])];
        assert_eq!(
            infer_module_root(Path::new("tests/test_x.vow"), &uses, exists),
            Some(PathBuf::from("."))
        );
    }

    #[test]
    fn infer_root_never_walks_past_a_parent_component_into_the_current_directory() {
        let uses = vec![comps(&["dep"])];
        let cwd_only = |p: &Path| p == Path::new("./dep.vow");
        assert_eq!(
            infer_module_root(Path::new("../other/tests/test_x.vow"), &uses, cwd_only),
            None
        );
        let sibling = |p: &Path| p == Path::new("../other/dep.vow");
        assert_eq!(
            infer_module_root(Path::new("../other/tests/test_x.vow"), &uses, sibling),
            Some(PathBuf::from("../other"))
        );
    }

    #[test]
    fn infer_root_stops_at_the_repository_root() {
        let exists = |p: &Path| p == Path::new("/repo/.git") || p == Path::new("/dep.vow");
        let uses = vec![comps(&["dep"])];
        assert_eq!(
            infer_module_root(Path::new("/repo/tests/test_x.vow"), &uses, exists),
            None
        );
    }

    fn parse(src: &str, file: &str) -> Module {
        let (module, diags) = vow_syntax::parser::parse_module(src, file);
        assert!(
            diags.is_empty(),
            "unexpected diagnostics for {file}: {diags:?}"
        );
        module
    }

    #[test]
    fn merge_concatenates_items_dependency_first_with_aligned_provenance() {
        let dep = parse("module Dep fn helper() -> i64 { 0 }", "dep.vow");
        let root = parse(
            "module Root use dep fn a() -> i64 { 0 } fn b() -> i64 { 1 }",
            "main.vow",
        );
        let graph = ModuleGraph {
            modules: vec![
                (PathBuf::from("dep.vow"), dep),
                (PathBuf::from("main.vow"), root),
            ],
        };

        let (merged, item_files) = merge_modules(graph);

        assert_eq!(merged.items.len(), 3);
        assert_eq!(item_files.len(), merged.items.len());
        assert_eq!(item_files, vec!["dep.vow", "main.vow", "main.vow"]);
    }

    #[test]
    fn merge_takes_root_name_and_span_and_clears_uses() {
        let dep = parse("module Dep fn helper() -> i64 { 0 }", "dep.vow");
        let root = parse("module Root use dep fn a() -> i64 { 0 }", "main.vow");
        let root_span = root.span;
        let graph = ModuleGraph {
            modules: vec![
                (PathBuf::from("dep.vow"), dep),
                (PathBuf::from("main.vow"), root),
            ],
        };

        let (merged, _) = merge_modules(graph);

        assert_eq!(merged.name, "Root");
        assert_eq!(merged.span, root_span);
        assert!(merged.uses.is_empty());
    }

    #[test]
    fn merge_of_single_module_attributes_every_item_to_that_file() {
        let root = parse(
            "module Solo fn a() -> i64 { 0 } fn b() -> i64 { 1 }",
            "solo.vow",
        );
        let graph = ModuleGraph {
            modules: vec![(PathBuf::from("solo.vow"), root)],
        };

        let (merged, item_files) = merge_modules(graph);

        assert_eq!(merged.items.len(), 2);
        assert_eq!(item_files, vec!["solo.vow", "solo.vow"]);
    }

    fn fn_item_is_declaration(module: &Module, name: &str) -> Option<bool> {
        module.items.iter().find_map(|item| match item {
            Item::Fn(f) if f.name == name => Some(f.is_declaration),
            _ => None,
        })
    }

    fn load_graph_for(dir: &Path, main_name: &str) -> Result<ModuleGraph, Vec<Diagnostic>> {
        let main_path = dir.join(main_name);
        let src = std::fs::read_to_string(&main_path).unwrap();
        let root_ast = parse_ok(&src, main_name);
        load_modules_with_root(&main_path, None, &root_ast)
    }

    #[test]
    fn load_deps_falls_back_to_source_when_stub_has_unenforceable_contract() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(
            dir.path().join("dep.vow"),
            "module Dep\npub fn f(x: i64) -> i64 vow { requires: x > 0 } {\n    x\n}\n",
        )
        .unwrap();
        std::fs::write(
            dir.path().join("dep.vow.d"),
            "module Dep\npub fn f(x: i64) -> i64 vow { requires: x > 0 };\n",
        )
        .unwrap();
        std::fs::write(
            dir.path().join("main.vow"),
            "module Main\nuse dep\nfn g() -> i64 { 0 }\n",
        )
        .unwrap();

        let graph = load_graph_for(dir.path(), "main.vow").expect("load should succeed");
        let dep_module = &graph
            .modules
            .iter()
            .find(|(path, _)| path.ends_with("dep.vow") || path.ends_with("dep.vow.d"))
            .expect("dep module present")
            .1;
        assert_eq!(
            fn_item_is_declaration(dep_module, "f"),
            Some(false),
            "must fall back to the .vow source, not the contract-bearing stub"
        );
    }

    #[test]
    fn load_deps_still_prefers_contract_free_stub() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(
            dir.path().join("dep.vow"),
            // Deliberately ill-typed, mirroring tests/multi/decl_stub_preference:
            // the loader must never touch this body while the stub is preferred.
            "module Dep\nfn source_only() -> i64 {\n    true\n}\n",
        )
        .unwrap();
        std::fs::write(
            dir.path().join("dep.vow.d"),
            "module Dep\npub fn declaration_only() -> i64;\n",
        )
        .unwrap();
        std::fs::write(
            dir.path().join("main.vow"),
            "module Main\nuse dep\nfn g() -> i64 { 0 }\n",
        )
        .unwrap();

        let graph = load_graph_for(dir.path(), "main.vow").expect("load should succeed");
        let dep_module = &graph
            .modules
            .iter()
            .find(|(path, _)| path.ends_with("dep.vow") || path.ends_with("dep.vow.d"))
            .expect("dep module present")
            .1;
        assert_eq!(
            fn_item_is_declaration(dep_module, "declaration_only"),
            Some(true),
            "contract-free stub must still be preferred over source"
        );
    }

    #[test]
    fn load_deps_does_not_fall_back_when_stub_fails_to_parse() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(
            dir.path().join("dep.vow"),
            "module Dep\npub fn f(x: i64) -> i64 {\n    x\n}\n",
        )
        .unwrap();
        std::fs::write(
            dir.path().join("dep.vow.d"),
            // Deliberately malformed: an unclosed parameter list. The source
            // fallback is scoped to #1472's contract-hiding bug, not stub
            // parse robustness, so a broken stub must surface its own parse
            // error rather than silently falling back to the valid sibling.
            "module Dep\npub fn f(x: i64 -> i64;\n",
        )
        .unwrap();
        std::fs::write(
            dir.path().join("main.vow"),
            "module Main\nuse dep\nfn g() -> i64 { 0 }\n",
        )
        .unwrap();

        let result = load_graph_for(dir.path(), "main.vow");
        assert!(
            result.is_err(),
            "a stub that fails to parse must not fall back to a valid sibling .vow source"
        );
    }
}
