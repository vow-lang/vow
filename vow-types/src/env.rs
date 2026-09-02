use crate::types::Ty;
use std::collections::{BTreeSet, BinaryHeap, HashMap, HashSet};
use vow_syntax::ast::{Effect, Type as AstType};
use vow_syntax::span::Span;

/// Per-binding mutability metadata, tracked in parallel with the type scopes.
/// `mutated` records whether the binding was the target of a whole-binding
/// assignment `x = e` (the only thing `mut` governs under Scope A).
#[derive(Debug, Clone, Copy)]
struct MutInfo {
    is_mut: bool,
    mutated: bool,
    span: Span,
}

/// Result of attempting to mark a binding as mutated via `mark_mutated`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MutStatus {
    /// No binding with that name is in scope (leave to undefined-var handling).
    NotFound,
    /// Found and declared `mut` — the assignment is allowed.
    Mutable,
    /// Found but not declared `mut` — the assignment is an error.
    Immutable,
}

/// Signature of a function or method known to the type checker.
#[derive(Debug, Clone)]
pub struct FnSig {
    pub params: Vec<Ty>,
    pub return_ty: Ty,
    pub effects: BTreeSet<Effect>,
}

/// Information about a user-defined struct type.
#[derive(Debug, Clone)]
pub struct StructInfo {
    /// Field name → resolved type, in declaration order.
    pub fields: Vec<(String, Ty)>,
    /// Whether declared with `linear struct`.
    pub is_linear: bool,
}

/// Information about a user-defined enum type.
#[derive(Debug, Clone)]
pub struct EnumInfo {
    pub variants: Vec<VariantInfo>,
}

/// One variant of an enum.
#[derive(Debug, Clone)]
pub struct VariantInfo {
    pub name: String,
    pub kind: VariantKind,
}

/// The payload shape of an enum variant.
#[derive(Debug, Clone)]
pub enum VariantKind {
    Unit,
    Tuple(Vec<Ty>),
    Struct(Vec<(String, Ty)>),
}

/// Picks the lex-smallest `max_names` keys via a bounded max-heap so the hint path can't be inflated by adversarial input.
fn sorted_capped_keys<V>(
    map: &HashMap<String, V>,
    max_names: usize,
    max_len: usize,
) -> Vec<String> {
    if max_names == 0 {
        return Vec::new();
    }
    let mut heap: BinaryHeap<&String> = BinaryHeap::with_capacity(max_names);
    for key in map.keys() {
        if key.len() > max_len {
            continue;
        }
        if heap.len() < max_names {
            heap.push(key);
        } else if let Some(&top) = heap.peek()
            && key < top
        {
            heap.pop();
            heap.push(key);
        }
    }
    let mut result: Vec<String> = heap.into_iter().cloned().collect();
    result.sort_unstable();
    result
}

/// Scope-based type environment.
///
/// Maintains a stack of lexical scopes for variable bindings. Top-level definitions
/// (functions, structs, enums) are stored separately and are always visible.
pub struct TypeEnv {
    scopes: Vec<HashMap<String, Ty>>,
    /// Mutability metadata, maintained in lockstep with `scopes` (same scope
    /// depth). A per-scope `Vec` rather than a map so same-scope shadows
    /// (`let mut x = 1; let x = 2;`) keep one record per declaration — matching
    /// the self-hosted checker's flat per-declaration vectors — so the first
    /// declaration's unused `mut` is still reported. `lookup`/`all_var_names`
    /// stay on `scopes` and are untouched.
    mut_info: Vec<Vec<(String, MutInfo)>>,
    fn_sigs: HashMap<String, FnSig>,
    /// Names registered from an `extern "C"` block. Calling one is rejected
    /// with `UnsupportedFeature`: neither compiler lowers extern-declared
    /// calls to IR yet, so silently falling through to the generic unresolved-
    /// call path would use a fabricated signature instead of the declared one.
    extern_fn_names: HashSet<String>,
    struct_defs: HashMap<String, StructInfo>,
    enum_defs: HashMap<String, EnumInfo>,
    type_aliases: HashMap<String, Ty>,
}

impl Default for TypeEnv {
    fn default() -> Self {
        Self::new()
    }
}

/// Construct the builtin `Vec<T>` type used by container-returning builtins.
fn vec_ty(elem: Ty) -> Ty {
    Ty::Applied(Box::new(Ty::Struct("Vec".to_string())), vec![elem])
}

/// Construct the builtin `Option<T>` type used by fallible builtins.
fn option_ty(elem: Ty) -> Ty {
    Ty::Applied(Box::new(Ty::Enum("Option".to_string())), vec![elem])
}

/// Signatures of every builtin free function known to the type checker.
///
/// This is the single source of truth consumed by [`TypeEnv::new`]. Holding the
/// builtin surface as data - rather than ~70 imperative `define_fn` calls -
/// makes it enumerable and directly testable, and matches the compact
/// one-line-per-builtin form the self-hosted checker already uses in
/// `compiler/env.vow`.
fn builtin_free_fn_signatures() -> Vec<(String, FnSig)> {
    fn def(name: &str, params: Vec<Ty>, return_ty: Ty, effects: &[Effect]) -> (String, FnSig) {
        (
            name.to_string(),
            FnSig {
                params,
                return_ty,
                effects: effects.iter().cloned().collect(),
            },
        )
    }

    let mut sigs = vec![
        def("print_str", vec![Ty::Str], Ty::Unit, &[Effect::IO]),
        def("print_i64", vec![Ty::I64], Ty::Unit, &[Effect::IO]),
        def("print_u64", vec![Ty::U64], Ty::Unit, &[Effect::IO]),
        def("debug_str", vec![Ty::Str], Ty::Unit, &[]),
        def("debug_i64", vec![Ty::I64], Ty::Unit, &[]),
        def("debug_u64", vec![Ty::U64], Ty::Unit, &[]),
        def("fs_read", vec![Ty::Str], Ty::Str, &[Effect::Read]),
        def("fs_open", vec![Ty::Str], Ty::I64, &[Effect::Read]),
        def("fs_read_line", vec![Ty::I64], Ty::Str, &[Effect::Read]),
        def("fs_status", vec![Ty::I64], Ty::I64, &[Effect::Read]),
        def("fs_close", vec![Ty::I64], Ty::I64, &[Effect::Read]),
        def(
            "fs_write",
            vec![Ty::Str, Ty::Str],
            Ty::I64,
            &[Effect::Write],
        ),
        def("fs_exists", vec![Ty::Str], Ty::I64, &[Effect::Read]),
        def("fs_mkdir", vec![Ty::Str], Ty::I64, &[Effect::IO]),
        def(
            "fs_listdir",
            vec![Ty::Str],
            vec_ty(Ty::Str),
            &[Effect::Read],
        ),
        def("fs_remove", vec![Ty::Str], Ty::I64, &[Effect::IO]),
        def("fs_remove_dir", vec![Ty::Str], Ty::I64, &[Effect::IO]),
        def("fs_is_dir", vec![Ty::Str], Ty::I64, &[Effect::Read]),
        def("fs_is_symlink", vec![Ty::Str], Ty::I64, &[Effect::Read]),
        def("fs_rename", vec![Ty::Str, Ty::Str], Ty::I64, &[Effect::IO]),
        def(
            "string_substr",
            vec![Ty::Str, Ty::I64, Ty::I64],
            Ty::Str,
            &[],
        ),
        def("string_split", vec![Ty::Str, Ty::Str], vec_ty(Ty::Str), &[]),
        def("string_starts_with", vec![Ty::Str, Ty::Str], Ty::I64, &[]),
        def("string_ends_with", vec![Ty::Str, Ty::Str], Ty::I64, &[]),
        def(
            "string_matches_literal_at",
            vec![Ty::Str, Ty::I64, Ty::Str],
            Ty::I64,
            &[],
        ),
        def("string_trim", vec![Ty::Str], Ty::Str, &[]),
        def("string_to_upper", vec![Ty::Str], Ty::Str, &[]),
        def("string_to_lower", vec![Ty::Str], Ty::Str, &[]),
        def(
            "string_replace",
            vec![Ty::Str, Ty::Str, Ty::Str],
            Ty::Str,
            &[],
        ),
        def("string_join", vec![vec_ty(Ty::Str), Ty::Str], Ty::Str, &[]),
        def("parse_i64", vec![Ty::Str], option_ty(Ty::I64), &[]),
        def("int_to_string", vec![Ty::I64], Ty::Str, &[]),
        def("uint_to_string", vec![Ty::U64], Ty::Str, &[]),
        def("i64_to_string", vec![Ty::I64], Ty::Str, &[]),
        def("vec_sort", vec![vec_ty(Ty::I64)], vec_ty(Ty::I64), &[]),
        def("time_unix", vec![], Ty::I64, &[Effect::IO]),
        def("time_unix_ms", vec![], Ty::I64, &[Effect::IO]),
        def("time_micros", vec![], Ty::I64, &[Effect::IO]),
        def("proc_sample", vec![], Ty::Str, &[Effect::IO]),
        def(
            "gzip_write_file",
            vec![Ty::Str, Ty::Str],
            Ty::I64,
            &[Effect::IO],
        ),
        def("num_cpus", vec![], Ty::I64, &[Effect::IO]),
        def("memory_root_arena_bytes", vec![], Ty::U64, &[Effect::IO]),
        def("memory_peak_bytes", vec![], Ty::U64, &[Effect::IO]),
        def(
            "memory_alloc_count_since_start",
            vec![],
            Ty::U64,
            &[Effect::IO],
        ),
        def("hex_encode", vec![vec_ty(Ty::U8)], Ty::Str, &[]),
        def("hex_decode", vec![Ty::Str], vec_ty(Ty::U8), &[]),
        def("eprintln_str", vec![Ty::Str], Ty::Unit, &[Effect::IO]),
        def("args", vec![], vec_ty(Ty::Str), &[Effect::Read]),
        def("stdin_read", vec![], Ty::Str, &[Effect::Read]),
        def("stdin_read_line", vec![], Ty::Str, &[Effect::Read]),
        def("stdin_ready", vec![], Ty::Bool, &[Effect::Read]),
        def("process_exit", vec![Ty::I64], Ty::Never, &[Effect::IO]),
        def(
            "process_run",
            vec![Ty::Str, vec_ty(Ty::Str)],
            Ty::I64,
            &[Effect::IO],
        ),
        def("process_get_stdout", vec![], Ty::Str, &[Effect::IO]),
        def("process_get_stderr", vec![], Ty::Str, &[Effect::IO]),
        def(
            "process_start",
            vec![Ty::Str, vec_ty(Ty::Str)],
            Ty::I64,
            &[Effect::IO],
        ),
        def("process_wait", vec![Ty::I64], Ty::I64, &[Effect::IO]),
        def(
            "process_wait_timeout",
            vec![Ty::I64, Ty::I64],
            Ty::I64,
            &[Effect::IO],
        ),
        def(
            "process_poll_wait",
            vec![Ty::I64, Ty::I64],
            Ty::I64,
            &[Effect::IO],
        ),
        def("process_kill", vec![Ty::I64], Ty::I64, &[Effect::IO]),
        def("process_stdout_for", vec![Ty::I64], Ty::Str, &[Effect::IO]),
        def("process_stderr_for", vec![Ty::I64], Ty::Str, &[Effect::IO]),
        def(
            "__vow_clif_create",
            vec![Ty::I64, Ty::I64],
            Ty::I64,
            &[Effect::IO],
        ),
        def(
            "__vow_clif_add_string",
            vec![Ty::I64, Ty::Str],
            Ty::Unit,
            &[Effect::IO],
        ),
        def(
            "__vow_clif_declare_extern",
            vec![Ty::I64, Ty::Str],
            Ty::Unit,
            &[Effect::IO],
        ),
        def(
            "__vow_clif_declare_function",
            vec![
                Ty::I64,
                Ty::I64,
                Ty::Str,
                vec_ty(Ty::I64),
                Ty::I64,
                Ty::I64,
                Ty::I64,
                Ty::I64,
                vec_ty(Ty::I64),
            ],
            Ty::Unit,
            &[Effect::IO],
        ),
        def(
            "__vow_clif_fn_begin",
            vec![Ty::I64, Ty::I64, Ty::I64, vec_ty(Ty::I64)],
            Ty::I64,
            &[Effect::IO],
        ),
        def("__vow_clif_fn_block", vec![Ty::I64], Ty::I64, &[Effect::IO]),
        def(
            "__vow_clif_fn_inst",
            vec![
                Ty::I64,
                Ty::I64,
                Ty::I64,
                Ty::I64,
                Ty::I64,
                Ty::I64,
                Ty::I64,
                Ty::Str,
                vec_ty(Ty::I64),
                Ty::I64,
            ],
            Ty::I64,
            &[Effect::IO],
        ),
        def(
            "__vow_clif_fn_vow",
            vec![Ty::I64, Ty::I64, Ty::Str, vec_ty(Ty::I64), vec_ty(Ty::Str)],
            Ty::I64,
            &[Effect::IO],
        ),
        def("__vow_clif_fn_end", vec![Ty::I64], Ty::I64, &[Effect::IO]),
        def(
            "__vow_clif_finish",
            vec![Ty::I64, Ty::Str],
            Ty::I64,
            &[Effect::IO],
        ),
        def(
            "__vow_clif_link",
            vec![Ty::Str, Ty::Str],
            Ty::I64,
            &[Effect::IO],
        ),
        def("__vow_clif_destroy", vec![Ty::I64], Ty::Unit, &[Effect::IO]),
    ];

    // Byte-narrowing conversions: `<source>_to_u8_<mode>` for every wider int,
    // preserved as a generated family rather than one row per combination.
    sigs.push(def("parse_u8", vec![Ty::Str], option_ty(Ty::U8), &[]));
    for (source_name, source_ty) in [
        ("i16", Ty::I16),
        ("i32", Ty::I32),
        ("i64", Ty::I64),
        ("i128", Ty::I128),
        ("u16", Ty::U16),
        ("u32", Ty::U32),
        ("u64", Ty::U64),
        ("u128", Ty::U128),
    ] {
        for (mode, return_ty) in [
            ("try", option_ty(Ty::U8)),
            ("wrap", Ty::U8),
            ("sat", Ty::U8),
        ] {
            sigs.push(def(
                &format!("{source_name}_to_u8_{mode}"),
                vec![source_ty.clone()],
                return_ty,
                &[],
            ));
        }
    }
    for name in ["add_sat_u8", "sub_sat_u8", "mul_sat_u8"] {
        sigs.push(def(name, vec![Ty::U8, Ty::U8], Ty::U8, &[]));
    }

    // 32-bit narrowing conversions: `<source>_to_i32_<mode>`.
    sigs.push(def("parse_i32", vec![Ty::Str], option_ty(Ty::I32), &[]));
    for (source_name, source_ty) in [("i64", Ty::I64), ("u32", Ty::U32), ("u64", Ty::U64)] {
        for (mode, return_ty) in [
            ("try", option_ty(Ty::I32)),
            ("wrap", Ty::I32),
            ("sat", Ty::I32),
        ] {
            sigs.push(def(
                &format!("{source_name}_to_i32_{mode}"),
                vec![source_ty.clone()],
                return_ty,
                &[],
            ));
        }
    }

    // Remaining narrow-int targets: `<source>_to_<target>_<mode>` for i8, i16,
    // u16, and u32, each with a `parse_<target>` entry.
    for (target_name, target_ty, sources) in [
        (
            "i8",
            Ty::I8,
            vec![
                ("i16", Ty::I16),
                ("u16", Ty::U16),
                ("i32", Ty::I32),
                ("u32", Ty::U32),
                ("i64", Ty::I64),
                ("u64", Ty::U64),
            ],
        ),
        (
            "i16",
            Ty::I16,
            vec![
                ("i32", Ty::I32),
                ("u32", Ty::U32),
                ("i64", Ty::I64),
                ("u64", Ty::U64),
            ],
        ),
        (
            "u16",
            Ty::U16,
            vec![
                ("i32", Ty::I32),
                ("u32", Ty::U32),
                ("i64", Ty::I64),
                ("u64", Ty::U64),
            ],
        ),
        ("u32", Ty::U32, vec![("i64", Ty::I64), ("u64", Ty::U64)]),
    ] {
        sigs.push(def(
            &format!("parse_{target_name}"),
            vec![Ty::Str],
            option_ty(target_ty.clone()),
            &[],
        ));
        for (source_name, source_ty) in sources {
            for (mode, return_ty) in [
                ("try", option_ty(target_ty.clone())),
                ("wrap", target_ty.clone()),
                ("sat", target_ty.clone()),
            ] {
                sigs.push(def(
                    &format!("{source_name}_to_{target_name}_{mode}"),
                    vec![source_ty.clone()],
                    return_ty,
                    &[],
                ));
            }
        }
    }

    sigs
}

impl TypeEnv {
    pub fn new() -> Self {
        let mut env = Self {
            scopes: vec![HashMap::new()],
            mut_info: vec![Vec::new()],
            fn_sigs: HashMap::new(),
            extern_fn_names: HashSet::new(),
            struct_defs: HashMap::new(),
            enum_defs: HashMap::new(),
            type_aliases: HashMap::new(),
        };
        for (name, sig) in builtin_free_fn_signatures() {
            env.define_fn(name, sig);
        }
        env
    }

    pub fn push_scope(&mut self) {
        self.scopes.push(HashMap::new());
        self.mut_info.push(Vec::new());
    }

    /// Pop the innermost scope, returning any `mut` bindings in it that were
    /// never reassigned (unused `mut`), sorted by source position so the caller
    /// emits `UnusedMut` deterministically.
    pub fn pop_scope(&mut self) -> Vec<(String, Span)> {
        assert!(self.scopes.len() > 1, "cannot pop the last scope");
        self.scopes.pop();
        let info = self.mut_info.pop().expect("mut_info parallels scopes");
        let mut unused: Vec<(String, Span)> = info
            .into_iter()
            .filter(|(_, m)| m.is_mut && !m.mutated)
            .map(|(name, m)| (name, m.span))
            .collect();
        unused.sort_by_key(|(_, span)| span.start);
        unused
    }

    /// Define an immutable binding (parameters, `result`, loop/match binders,
    /// and tests). Equivalent to `define_mut(name, ty, false, _)`.
    pub fn define(&mut self, name: &str, ty: Ty) {
        self.define_mut(name, ty, false, Span::new(0, 0));
    }

    /// Define a binding carrying its declared mutability and declaration span.
    /// Only the `let`-binding path passes `is_mut = true`.
    pub fn define_mut(&mut self, name: &str, ty: Ty, is_mut: bool, span: Span) {
        self.scopes
            .last_mut()
            .expect("at least one scope must exist")
            .insert(name.to_string(), ty);
        self.mut_info
            .last_mut()
            .expect("mut_info parallels scopes")
            .push((
                name.to_string(),
                MutInfo {
                    is_mut,
                    mutated: false,
                    span,
                },
            ));
    }

    /// Record a whole-binding assignment `name = e` against the nearest binding
    /// in scope, returning whether that binding exists and is mutable. Marks the
    /// binding as mutated so it is not later flagged as unused `mut`.
    pub fn mark_mutated(&mut self, name: &str) -> MutStatus {
        for scope in self.mut_info.iter_mut().rev() {
            // The most-recent matching declaration is the visible binding
            // (handles same-scope shadowing); reassignment marks that one used.
            if let Some((_, info)) = scope.iter_mut().rev().find(|(n, _)| n == name) {
                info.mutated = true;
                return if info.is_mut {
                    MutStatus::Mutable
                } else {
                    MutStatus::Immutable
                };
            }
        }
        MutStatus::NotFound
    }

    pub fn lookup(&self, name: &str) -> Option<&Ty> {
        for scope in self.scopes.iter().rev() {
            if let Some(ty) = scope.get(name) {
                return Some(ty);
            }
        }
        None
    }

    pub fn define_fn(&mut self, name: impl Into<String>, sig: FnSig) {
        self.fn_sigs.insert(name.into(), sig);
    }

    pub fn lookup_fn(&self, name: &str) -> Option<&FnSig> {
        self.fn_sigs.get(name)
    }

    pub fn mark_extern_fn(&mut self, name: impl Into<String>) {
        self.extern_fn_names.insert(name.into());
    }

    pub fn is_extern_fn(&self, name: &str) -> bool {
        self.extern_fn_names.contains(name)
    }

    pub fn define_struct(&mut self, name: impl Into<String>, info: StructInfo) {
        self.struct_defs.insert(name.into(), info);
    }

    pub fn lookup_struct(&self, name: &str) -> Option<&StructInfo> {
        self.struct_defs.get(name)
    }

    pub fn define_enum(&mut self, name: impl Into<String>, info: EnumInfo) {
        self.enum_defs.insert(name.into(), info);
    }

    pub fn lookup_enum(&self, name: &str) -> Option<&EnumInfo> {
        self.enum_defs.get(name)
    }

    pub fn define_alias(&mut self, name: impl Into<String>, ty: Ty) {
        self.type_aliases.insert(name.into(), ty);
    }

    pub fn all_var_names(&self, max_names: usize, max_len: usize) -> Vec<String> {
        // Inner-scope priority: each scope contributes via a bounded max-heap so the hint path stays O(max_names) memory.
        if max_names == 0 {
            return Vec::new();
        }
        let mut names: Vec<String> = Vec::with_capacity(max_names);
        let mut seen: HashSet<&str> = HashSet::with_capacity(max_names);
        for scope in self.scopes.iter().rev() {
            if names.len() >= max_names {
                break;
            }
            let remaining = max_names - names.len();
            let mut heap: BinaryHeap<&String> = BinaryHeap::with_capacity(remaining);
            for key in scope.keys() {
                if key.len() > max_len {
                    continue;
                }
                if seen.contains(key.as_str()) {
                    continue;
                }
                if heap.len() < remaining {
                    heap.push(key);
                } else if let Some(&top) = heap.peek()
                    && key < top
                {
                    heap.pop();
                    heap.push(key);
                }
            }
            let mut scope_names: Vec<&String> = heap.into_iter().collect();
            scope_names.sort_unstable();
            for key in scope_names {
                seen.insert(key.as_str());
                names.push(key.clone());
                if names.len() >= max_names {
                    break;
                }
            }
        }
        names
    }

    pub fn all_fn_names(&self, max_names: usize, max_len: usize) -> Vec<String> {
        sorted_capped_keys(&self.fn_sigs, max_names, max_len)
    }

    pub fn all_struct_names(&self, max_names: usize, max_len: usize) -> Vec<String> {
        sorted_capped_keys(&self.struct_defs, max_names, max_len)
    }

    pub fn resolve(&self, ast_ty: &AstType) -> Result<Ty, String> {
        match ast_ty {
            AstType::Named { name, .. } => {
                if let Some(ty) = Ty::from_primitive_name(name) {
                    return Ok(ty);
                }
                if self.lookup_struct(name).is_some() {
                    return Ok(Ty::Struct(name.clone()));
                }
                if self.lookup_enum(name).is_some() {
                    return Ok(Ty::Enum(name.clone()));
                }
                if let Some(ty) = self.type_aliases.get(name) {
                    return Ok(ty.clone());
                }
                Err(format!("unknown type: {name}"))
            }
            AstType::Generic { name, args, .. } => {
                // Builtin generic types
                match name.as_str() {
                    "Option" => {
                        if args.len() != 1 {
                            return Err("Option requires one type argument".to_string());
                        }
                        let arg = &args[0];
                        let t = self.resolve(arg)?;
                        return Ok(Ty::Applied(
                            Box::new(Ty::Enum("Option".to_string())),
                            vec![t],
                        ));
                    }
                    "Result" => {
                        if args.len() != 2 {
                            return Err("Result requires two type arguments".to_string());
                        }
                        let t = &args[0];
                        let e = &args[1];
                        return Ok(Ty::Applied(
                            Box::new(Ty::Enum("Result".to_string())),
                            vec![self.resolve(t)?, self.resolve(e)?],
                        ));
                    }
                    "Vec" => {
                        let arg = args
                            .first()
                            .ok_or_else(|| "Vec requires one type argument".to_string())?;
                        let t = self.resolve(arg)?;
                        return Ok(Ty::Applied(
                            Box::new(Ty::Struct("Vec".to_string())),
                            vec![t],
                        ));
                    }
                    "HashMap" => {
                        let k = args
                            .first()
                            .ok_or_else(|| "HashMap requires two type arguments".to_string())?;
                        let v = args
                            .get(1)
                            .ok_or_else(|| "HashMap requires two type arguments".to_string())?;
                        return Ok(Ty::Applied(
                            Box::new(Ty::Struct("HashMap".to_string())),
                            vec![self.resolve(k)?, self.resolve(v)?],
                        ));
                    }
                    "BTreeMap" => {
                        if args.len() != 2 {
                            return Err("BTreeMap requires two type arguments".to_string());
                        }
                        return Ok(Ty::Applied(
                            Box::new(Ty::Struct("BTreeMap".to_string())),
                            vec![self.resolve(&args[0])?, self.resolve(&args[1])?],
                        ));
                    }
                    _ => {}
                }
                let base = self.resolve(&AstType::Named {
                    name: name.clone(),
                    span: vow_syntax::span::Span::new(0, 0),
                })?;
                let resolved_args = args
                    .iter()
                    .map(|a| self.resolve(a))
                    .collect::<Result<Vec<_>, _>>()?;
                Ok(Ty::Applied(Box::new(base), resolved_args))
            }
            AstType::Reference { inner, .. } => Ok(Ty::Reference(Box::new(self.resolve(inner)?))),
            AstType::Tuple { elems, .. } => {
                let resolved = elems
                    .iter()
                    .map(|e| self.resolve(e))
                    .collect::<Result<Vec<_>, _>>()?;
                Ok(Ty::Tuple(resolved))
            }
            AstType::Unit { .. } => Ok(Ty::Unit),
            AstType::Never { .. } => Ok(Ty::Never),
            AstType::Slice { inner, .. } => Ok(Ty::Applied(
                Box::new(Ty::Struct("Slice".to_string())),
                vec![self.resolve(inner)?],
            )),
            // Inline refinement types `{ r: T || pred }` carry a predicate that
            // no later pass consumes — resolving to `base` would silently drop it,
            // letting the function verify as if the predicate were absent (a false
            // proof). Fail closed until the predicate is forwarded to a real
            // obligation. Parameter `where` clauses are unaffected: they live in
            // `Param.refinement`, not in the type, and become `requires`.
            AstType::Refinement { .. } => Err(
                "refinement-type predicate `{ x: T || pred }` is not supported in \
                 type position (it would be silently unverified); express the \
                 constraint with a `where` clause on the parameter or a \
                 `requires`/`ensures` contract"
                    .to_string(),
            ),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeSet;
    use vow_syntax::span::Span;

    fn dummy_span() -> Span {
        Span::new(0, 0)
    }

    fn builtin_signature_snapshot() -> String {
        let env = TypeEnv::new();
        let mut lines: Vec<String> = env
            .fn_sigs
            .iter()
            .map(|(name, sig)| {
                let params = sig
                    .params
                    .iter()
                    .map(|t| format!("{t:?}"))
                    .collect::<Vec<_>>()
                    .join(", ");
                let effects = sig
                    .effects
                    .iter()
                    .map(|e| format!("{e:?}"))
                    .collect::<Vec<_>>()
                    .join(", ");
                format!("{name}({params}) -> {:?} [{effects}]", sig.return_ty)
            })
            .collect();
        lines.sort();
        lines.join("\n")
    }

    // Golden snapshot of every builtin free-function signature registered by
    // `TypeEnv::new`. Captured from behavior prior to the table refactor, so it
    // is an independent source of truth: any dropped, added, or drifted builtin
    // fails here with the offending line named.
    const EXPECTED_BUILTIN_SIGNATURES: &str = r#"
__vow_clif_add_string(I64, Str) -> Unit [IO]
__vow_clif_create(I64, I64) -> I64 [IO]
__vow_clif_declare_extern(I64, Str) -> Unit [IO]
__vow_clif_declare_function(I64, I64, Str, Applied(Struct("Vec"), [I64]), I64, I64, I64, I64, Applied(Struct("Vec"), [I64])) -> Unit [IO]
__vow_clif_destroy(I64) -> Unit [IO]
__vow_clif_finish(I64, Str) -> I64 [IO]
__vow_clif_fn_begin(I64, I64, I64, Applied(Struct("Vec"), [I64])) -> I64 [IO]
__vow_clif_fn_block(I64) -> I64 [IO]
__vow_clif_fn_end(I64) -> I64 [IO]
__vow_clif_fn_inst(I64, I64, I64, I64, I64, I64, I64, Str, Applied(Struct("Vec"), [I64]), I64) -> I64 [IO]
__vow_clif_fn_vow(I64, I64, Str, Applied(Struct("Vec"), [I64]), Applied(Struct("Vec"), [Str])) -> I64 [IO]
__vow_clif_link(Str, Str) -> I64 [IO]
add_sat_u8(U8, U8) -> U8 []
args() -> Applied(Struct("Vec"), [Str]) [Read]
debug_i64(I64) -> Unit []
debug_str(Str) -> Unit []
debug_u64(U64) -> Unit []
eprintln_str(Str) -> Unit [IO]
fs_close(I64) -> I64 [Read]
fs_exists(Str) -> I64 [Read]
fs_is_dir(Str) -> I64 [Read]
fs_is_symlink(Str) -> I64 [Read]
fs_listdir(Str) -> Applied(Struct("Vec"), [Str]) [Read]
fs_mkdir(Str) -> I64 [IO]
fs_open(Str) -> I64 [Read]
fs_read(Str) -> Str [Read]
fs_read_line(I64) -> Str [Read]
fs_remove(Str) -> I64 [IO]
fs_remove_dir(Str) -> I64 [IO]
fs_rename(Str, Str) -> I64 [IO]
fs_status(I64) -> I64 [Read]
fs_write(Str, Str) -> I64 [Write]
gzip_write_file(Str, Str) -> I64 [IO]
hex_decode(Str) -> Applied(Struct("Vec"), [U8]) []
hex_encode(Applied(Struct("Vec"), [U8])) -> Str []
i128_to_u8_sat(I128) -> U8 []
i128_to_u8_try(I128) -> Applied(Enum("Option"), [U8]) []
i128_to_u8_wrap(I128) -> U8 []
i16_to_i8_sat(I16) -> I8 []
i16_to_i8_try(I16) -> Applied(Enum("Option"), [I8]) []
i16_to_i8_wrap(I16) -> I8 []
i16_to_u8_sat(I16) -> U8 []
i16_to_u8_try(I16) -> Applied(Enum("Option"), [U8]) []
i16_to_u8_wrap(I16) -> U8 []
i32_to_i16_sat(I32) -> I16 []
i32_to_i16_try(I32) -> Applied(Enum("Option"), [I16]) []
i32_to_i16_wrap(I32) -> I16 []
i32_to_i8_sat(I32) -> I8 []
i32_to_i8_try(I32) -> Applied(Enum("Option"), [I8]) []
i32_to_i8_wrap(I32) -> I8 []
i32_to_u16_sat(I32) -> U16 []
i32_to_u16_try(I32) -> Applied(Enum("Option"), [U16]) []
i32_to_u16_wrap(I32) -> U16 []
i32_to_u8_sat(I32) -> U8 []
i32_to_u8_try(I32) -> Applied(Enum("Option"), [U8]) []
i32_to_u8_wrap(I32) -> U8 []
i64_to_i16_sat(I64) -> I16 []
i64_to_i16_try(I64) -> Applied(Enum("Option"), [I16]) []
i64_to_i16_wrap(I64) -> I16 []
i64_to_i32_sat(I64) -> I32 []
i64_to_i32_try(I64) -> Applied(Enum("Option"), [I32]) []
i64_to_i32_wrap(I64) -> I32 []
i64_to_i8_sat(I64) -> I8 []
i64_to_i8_try(I64) -> Applied(Enum("Option"), [I8]) []
i64_to_i8_wrap(I64) -> I8 []
i64_to_string(I64) -> Str []
i64_to_u16_sat(I64) -> U16 []
i64_to_u16_try(I64) -> Applied(Enum("Option"), [U16]) []
i64_to_u16_wrap(I64) -> U16 []
i64_to_u32_sat(I64) -> U32 []
i64_to_u32_try(I64) -> Applied(Enum("Option"), [U32]) []
i64_to_u32_wrap(I64) -> U32 []
i64_to_u8_sat(I64) -> U8 []
i64_to_u8_try(I64) -> Applied(Enum("Option"), [U8]) []
i64_to_u8_wrap(I64) -> U8 []
int_to_string(I64) -> Str []
memory_alloc_count_since_start() -> U64 [IO]
memory_peak_bytes() -> U64 [IO]
memory_root_arena_bytes() -> U64 [IO]
mul_sat_u8(U8, U8) -> U8 []
num_cpus() -> I64 [IO]
parse_i16(Str) -> Applied(Enum("Option"), [I16]) []
parse_i32(Str) -> Applied(Enum("Option"), [I32]) []
parse_i64(Str) -> Applied(Enum("Option"), [I64]) []
parse_i8(Str) -> Applied(Enum("Option"), [I8]) []
parse_u16(Str) -> Applied(Enum("Option"), [U16]) []
parse_u32(Str) -> Applied(Enum("Option"), [U32]) []
parse_u8(Str) -> Applied(Enum("Option"), [U8]) []
print_i64(I64) -> Unit [IO]
print_str(Str) -> Unit [IO]
print_u64(U64) -> Unit [IO]
proc_sample() -> Str [IO]
process_exit(I64) -> Never [IO]
process_get_stderr() -> Str [IO]
process_get_stdout() -> Str [IO]
process_kill(I64) -> I64 [IO]
process_poll_wait(I64, I64) -> I64 [IO]
process_run(Str, Applied(Struct("Vec"), [Str])) -> I64 [IO]
process_start(Str, Applied(Struct("Vec"), [Str])) -> I64 [IO]
process_stderr_for(I64) -> Str [IO]
process_stdout_for(I64) -> Str [IO]
process_wait(I64) -> I64 [IO]
process_wait_timeout(I64, I64) -> I64 [IO]
stdin_read() -> Str [Read]
stdin_read_line() -> Str [Read]
stdin_ready() -> Bool [Read]
string_ends_with(Str, Str) -> I64 []
string_join(Applied(Struct("Vec"), [Str]), Str) -> Str []
string_matches_literal_at(Str, I64, Str) -> I64 []
string_replace(Str, Str, Str) -> Str []
string_split(Str, Str) -> Applied(Struct("Vec"), [Str]) []
string_starts_with(Str, Str) -> I64 []
string_substr(Str, I64, I64) -> Str []
string_to_lower(Str) -> Str []
string_to_upper(Str) -> Str []
string_trim(Str) -> Str []
sub_sat_u8(U8, U8) -> U8 []
time_micros() -> I64 [IO]
time_unix() -> I64 [IO]
time_unix_ms() -> I64 [IO]
u128_to_u8_sat(U128) -> U8 []
u128_to_u8_try(U128) -> Applied(Enum("Option"), [U8]) []
u128_to_u8_wrap(U128) -> U8 []
u16_to_i8_sat(U16) -> I8 []
u16_to_i8_try(U16) -> Applied(Enum("Option"), [I8]) []
u16_to_i8_wrap(U16) -> I8 []
u16_to_u8_sat(U16) -> U8 []
u16_to_u8_try(U16) -> Applied(Enum("Option"), [U8]) []
u16_to_u8_wrap(U16) -> U8 []
u32_to_i16_sat(U32) -> I16 []
u32_to_i16_try(U32) -> Applied(Enum("Option"), [I16]) []
u32_to_i16_wrap(U32) -> I16 []
u32_to_i32_sat(U32) -> I32 []
u32_to_i32_try(U32) -> Applied(Enum("Option"), [I32]) []
u32_to_i32_wrap(U32) -> I32 []
u32_to_i8_sat(U32) -> I8 []
u32_to_i8_try(U32) -> Applied(Enum("Option"), [I8]) []
u32_to_i8_wrap(U32) -> I8 []
u32_to_u16_sat(U32) -> U16 []
u32_to_u16_try(U32) -> Applied(Enum("Option"), [U16]) []
u32_to_u16_wrap(U32) -> U16 []
u32_to_u8_sat(U32) -> U8 []
u32_to_u8_try(U32) -> Applied(Enum("Option"), [U8]) []
u32_to_u8_wrap(U32) -> U8 []
u64_to_i16_sat(U64) -> I16 []
u64_to_i16_try(U64) -> Applied(Enum("Option"), [I16]) []
u64_to_i16_wrap(U64) -> I16 []
u64_to_i32_sat(U64) -> I32 []
u64_to_i32_try(U64) -> Applied(Enum("Option"), [I32]) []
u64_to_i32_wrap(U64) -> I32 []
u64_to_i8_sat(U64) -> I8 []
u64_to_i8_try(U64) -> Applied(Enum("Option"), [I8]) []
u64_to_i8_wrap(U64) -> I8 []
u64_to_u16_sat(U64) -> U16 []
u64_to_u16_try(U64) -> Applied(Enum("Option"), [U16]) []
u64_to_u16_wrap(U64) -> U16 []
u64_to_u32_sat(U64) -> U32 []
u64_to_u32_try(U64) -> Applied(Enum("Option"), [U32]) []
u64_to_u32_wrap(U64) -> U32 []
u64_to_u8_sat(U64) -> U8 []
u64_to_u8_try(U64) -> Applied(Enum("Option"), [U8]) []
u64_to_u8_wrap(U64) -> U8 []
uint_to_string(U64) -> Str []
vec_sort(Applied(Struct("Vec"), [I64])) -> Applied(Struct("Vec"), [I64]) []
"#;

    #[test]
    fn builtin_signatures_snapshot() {
        assert_eq!(
            builtin_signature_snapshot(),
            EXPECTED_BUILTIN_SIGNATURES.trim()
        );
    }

    #[test]
    fn builtin_signatures_match_spec_examples() {
        // Signatures asserted here are sourced independently from the builtin
        // reference table in docs/spec/grammar.md, not from the code under test.
        let env = TypeEnv::new();
        let vec_of = |elem: Ty| Ty::Applied(Box::new(Ty::Struct("Vec".to_string())), vec![elem]);
        let option_of =
            |elem: Ty| Ty::Applied(Box::new(Ty::Enum("Option".to_string())), vec![elem]);
        let pure: BTreeSet<Effect> = BTreeSet::new();
        let io: BTreeSet<Effect> = [Effect::IO].into_iter().collect();
        let write: BTreeSet<Effect> = [Effect::Write].into_iter().collect();

        let cases: &[(&str, Vec<Ty>, Ty, &BTreeSet<Effect>)] = &[
            ("print_str", vec![Ty::Str], Ty::Unit, &io),
            ("debug_str", vec![Ty::Str], Ty::Unit, &pure),
            ("fs_write", vec![Ty::Str, Ty::Str], Ty::I64, &write),
            (
                "string_join",
                vec![vec_of(Ty::Str), Ty::Str],
                Ty::Str,
                &pure,
            ),
            ("parse_i64", vec![Ty::Str], option_of(Ty::I64), &pure),
            ("parse_u8", vec![Ty::Str], option_of(Ty::U8), &pure),
            ("i64_to_u8_try", vec![Ty::I64], option_of(Ty::U8), &pure),
        ];

        for (name, params, return_ty, effects) in cases {
            let sig = env
                .lookup_fn(name)
                .unwrap_or_else(|| panic!("builtin `{name}` is not registered"));
            assert_eq!(&sig.params, params, "params mismatch for `{name}`");
            assert_eq!(
                &sig.return_ty, return_ty,
                "return type mismatch for `{name}`"
            );
            assert_eq!(&sig.effects, *effects, "effects mismatch for `{name}`");
        }
    }

    #[test]
    fn scope_push_pop_and_lookup() {
        let mut env = TypeEnv::new();
        env.define("x", Ty::I32);
        assert_eq!(env.lookup("x"), Some(&Ty::I32));

        env.push_scope();
        assert_eq!(env.lookup("x"), Some(&Ty::I32));
        env.define("y", Ty::Bool);
        assert_eq!(env.lookup("y"), Some(&Ty::Bool));

        env.pop_scope();
        assert_eq!(env.lookup("y"), None);
        assert_eq!(env.lookup("x"), Some(&Ty::I32));
    }

    #[test]
    fn variable_shadowing() {
        let mut env = TypeEnv::new();
        env.define("x", Ty::I32);

        env.push_scope();
        env.define("x", Ty::Bool);
        assert_eq!(env.lookup("x"), Some(&Ty::Bool));

        env.pop_scope();
        assert_eq!(env.lookup("x"), Some(&Ty::I32));
    }

    #[test]
    fn same_scope_shadow_preserves_first_unused_mut() {
        // `let mut x = 1; let x = 2;` — the first binding is a `mut` that is
        // never reassigned, so it must still be reported as unused even though
        // a same-scope shadow reuses the name. Regression: a name-keyed map
        // dropped the first record and diverged from the self-hosted checker's
        // flat per-declaration vectors.
        let mut env = TypeEnv::new();
        env.push_scope();
        env.define_mut("x", Ty::I32, true, Span::new(0, 1));
        env.define_mut("x", Ty::I32, false, Span::new(10, 1));
        let unused = env.pop_scope();
        assert_eq!(unused, vec![("x".to_string(), Span::new(0, 1))]);
    }

    #[test]
    fn lookup_returns_innermost_binding() {
        let mut env = TypeEnv::new();
        env.define("v", Ty::U8);
        env.push_scope();
        env.define("v", Ty::F64);
        env.push_scope();
        assert_eq!(env.lookup("v"), Some(&Ty::F64));
    }

    #[test]
    fn lookup_fn_returns_registered_signature() {
        let mut env = TypeEnv::new();
        let sig = FnSig {
            params: vec![Ty::I32, Ty::I32],
            return_ty: Ty::I32,
            effects: BTreeSet::new(),
        };
        env.define_fn("add", sig.clone());
        let found = env.lookup_fn("add").unwrap();
        assert_eq!(found.params, sig.params);
        assert_eq!(found.return_ty, sig.return_ty);
    }

    #[test]
    fn resolve_primitive_i32() {
        let env = TypeEnv::new();
        let ast_ty = AstType::Named {
            name: "i32".to_string(),
            span: dummy_span(),
        };
        assert_eq!(env.resolve(&ast_ty), Ok(Ty::I32));
    }

    #[test]
    fn resolve_primitive_bool() {
        let env = TypeEnv::new();
        let ast_ty = AstType::Named {
            name: "bool".to_string(),
            span: dummy_span(),
        };
        assert_eq!(env.resolve(&ast_ty), Ok(Ty::Bool));
    }

    #[test]
    fn resolve_registered_struct() {
        let mut env = TypeEnv::new();
        env.define_struct(
            "Foo",
            StructInfo {
                fields: vec![],
                is_linear: false,
            },
        );
        let ast_ty = AstType::Named {
            name: "Foo".to_string(),
            span: dummy_span(),
        };
        assert_eq!(env.resolve(&ast_ty), Ok(Ty::Struct("Foo".to_string())));
    }

    #[test]
    fn resolve_returns_err_for_unknown() {
        let env = TypeEnv::new();
        let ast_ty = AstType::Named {
            name: "Unknown".to_string(),
            span: dummy_span(),
        };
        assert!(env.resolve(&ast_ty).is_err());
    }

    #[test]
    fn resolve_refinement_predicate_fails_closed() {
        use vow_syntax::ast::{Expr, ExprKind, Lit};
        let env = TypeEnv::new();
        // { r: i64 || true } — resolving to the base i64 would silently drop the
        // predicate and verify as if it were absent. resolve() must fail instead.
        let ast_ty = AstType::Refinement {
            binding: "r".to_string(),
            base: Box::new(AstType::Named {
                name: "i64".to_string(),
                span: dummy_span(),
            }),
            predicate: Box::new(Expr {
                kind: ExprKind::Lit(Lit::Bool(true)),
                span: dummy_span(),
            }),
            span: dummy_span(),
        };
        assert!(env.resolve(&ast_ty).is_err());
    }

    #[test]
    fn resolve_reference() {
        let env = TypeEnv::new();
        let ast_ty = AstType::Reference {
            inner: Box::new(AstType::Named {
                name: "i32".to_string(),
                span: dummy_span(),
            }),
            span: dummy_span(),
        };
        assert_eq!(env.resolve(&ast_ty), Ok(Ty::Reference(Box::new(Ty::I32))));
    }

    #[test]
    fn resolve_tuple() {
        let env = TypeEnv::new();
        let ast_ty = AstType::Tuple {
            elems: vec![
                AstType::Named {
                    name: "i32".to_string(),
                    span: dummy_span(),
                },
                AstType::Named {
                    name: "bool".to_string(),
                    span: dummy_span(),
                },
            ],
            span: dummy_span(),
        };
        assert_eq!(env.resolve(&ast_ty), Ok(Ty::Tuple(vec![Ty::I32, Ty::Bool])));
    }

    #[test]
    fn resolve_unit_and_never() {
        let env = TypeEnv::new();
        assert_eq!(
            env.resolve(&AstType::Unit { span: dummy_span() }),
            Ok(Ty::Unit)
        );
        assert_eq!(
            env.resolve(&AstType::Never { span: dummy_span() }),
            Ok(Ty::Never)
        );
    }

    #[test]
    fn resolve_option_generic() {
        let env = TypeEnv::new();
        let ast_ty = AstType::Generic {
            name: "Option".to_string(),
            args: vec![AstType::Named {
                name: "i64".to_string(),
                span: dummy_span(),
            }],
            span: dummy_span(),
        };
        assert_eq!(
            env.resolve(&ast_ty),
            Ok(Ty::Applied(
                Box::new(Ty::Enum("Option".to_string())),
                vec![Ty::I64]
            ))
        );
    }

    #[test]
    fn resolve_option_generic_rejects_wrong_arity() {
        let env = TypeEnv::new();
        let no_args = AstType::Generic {
            name: "Option".to_string(),
            args: vec![],
            span: dummy_span(),
        };
        assert_eq!(
            env.resolve(&no_args),
            Err("Option requires one type argument".to_string())
        );

        let too_many_args = AstType::Generic {
            name: "Option".to_string(),
            args: vec![
                AstType::Named {
                    name: "i64".to_string(),
                    span: dummy_span(),
                },
                AstType::Named {
                    name: "bool".to_string(),
                    span: dummy_span(),
                },
            ],
            span: dummy_span(),
        };
        assert_eq!(
            env.resolve(&too_many_args),
            Err("Option requires one type argument".to_string())
        );
    }

    #[test]
    fn resolve_result_generic() {
        let env = TypeEnv::new();
        let ast_ty = AstType::Generic {
            name: "Result".to_string(),
            args: vec![
                AstType::Named {
                    name: "i64".to_string(),
                    span: dummy_span(),
                },
                AstType::Named {
                    name: "bool".to_string(),
                    span: dummy_span(),
                },
            ],
            span: dummy_span(),
        };
        assert_eq!(
            env.resolve(&ast_ty),
            Ok(Ty::Applied(
                Box::new(Ty::Enum("Result".to_string())),
                vec![Ty::I64, Ty::Bool]
            ))
        );
    }

    #[test]
    fn resolve_result_generic_rejects_wrong_arity() {
        let env = TypeEnv::new();
        let one_arg = AstType::Generic {
            name: "Result".to_string(),
            args: vec![AstType::Named {
                name: "i64".to_string(),
                span: dummy_span(),
            }],
            span: dummy_span(),
        };
        assert_eq!(
            env.resolve(&one_arg),
            Err("Result requires two type arguments".to_string())
        );

        let too_many_args = AstType::Generic {
            name: "Result".to_string(),
            args: vec![
                AstType::Named {
                    name: "i64".to_string(),
                    span: dummy_span(),
                },
                AstType::Named {
                    name: "bool".to_string(),
                    span: dummy_span(),
                },
                AstType::Named {
                    name: "i32".to_string(),
                    span: dummy_span(),
                },
            ],
            span: dummy_span(),
        };
        assert_eq!(
            env.resolve(&too_many_args),
            Err("Result requires two type arguments".to_string())
        );
    }

    #[test]
    fn resolve_vec_generic() {
        let env = TypeEnv::new();
        let ast_ty = AstType::Generic {
            name: "Vec".to_string(),
            args: vec![AstType::Named {
                name: "i32".to_string(),
                span: dummy_span(),
            }],
            span: dummy_span(),
        };
        assert_eq!(
            env.resolve(&ast_ty),
            Ok(Ty::Applied(
                Box::new(Ty::Struct("Vec".to_string())),
                vec![Ty::I32]
            ))
        );
    }
}
