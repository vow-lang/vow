use crate::types::Ty;
use std::collections::{BTreeSet, BinaryHeap, HashMap, HashSet};
use vow_syntax::ast::{Effect, Type as AstType};

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

/// Pick the lexicographically smallest `max_names` keys from a `HashMap`.
///
/// Used on the "did you mean" hint path. Memory and work are bounded by
/// `max_names`, not by the map size: we keep a max-heap capped at
/// `max_names` and replace the largest entry whenever a smaller key is
/// seen. That way an adversarial source with millions of definitions
/// cannot inflate the diagnostic path beyond O(N · log max_names) time
/// and O(max_names) memory, while still producing a deterministic
/// candidate subset.
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
        } else if let Some(&top) = heap.peek() {
            if key < top {
                heap.pop();
                heap.push(key);
            }
        }
    }
    let mut result: Vec<String> = heap.into_iter().map(String::clone).collect();
    result.sort_unstable();
    result
}

/// Scope-based type environment.
///
/// Maintains a stack of lexical scopes for variable bindings. Top-level definitions
/// (functions, structs, enums) are stored separately and are always visible.
pub struct TypeEnv {
    // `HashMap` is used for the lookup tables because the type checker hits
    // them on every variable reference, function call, and struct access —
    // O(1) is the right complexity for those hot paths. The "did you mean"
    // hint helpers (`all_var_names` / `all_fn_names` / `all_struct_names`)
    // sort keys on demand so the truncated candidate subset stays
    // deterministic even though the underlying iteration order is not.
    scopes: Vec<HashMap<String, Ty>>,
    fn_sigs: HashMap<String, FnSig>,
    struct_defs: HashMap<String, StructInfo>,
    enum_defs: HashMap<String, EnumInfo>,
    type_aliases: HashMap<String, Ty>,
}

impl Default for TypeEnv {
    fn default() -> Self {
        Self::new()
    }
}

impl TypeEnv {
    pub fn new() -> Self {
        let mut env = Self {
            scopes: vec![HashMap::new()],
            fn_sigs: HashMap::new(),
            struct_defs: HashMap::new(),
            enum_defs: HashMap::new(),
            type_aliases: HashMap::new(),
        };
        env.define_fn(
            "print_str",
            FnSig {
                params: vec![Ty::Str],
                return_ty: Ty::Unit,
                effects: [Effect::IO].into_iter().collect(),
            },
        );
        env.define_fn(
            "print_i64",
            FnSig {
                params: vec![Ty::I64],
                return_ty: Ty::Unit,
                effects: [Effect::IO].into_iter().collect(),
            },
        );
        env.define_fn(
            "print_u64",
            FnSig {
                params: vec![Ty::U64],
                return_ty: Ty::Unit,
                effects: [Effect::IO].into_iter().collect(),
            },
        );
        env.define_fn(
            "debug_str",
            FnSig {
                params: vec![Ty::Str],
                return_ty: Ty::Unit,
                effects: BTreeSet::new(),
            },
        );
        env.define_fn(
            "debug_i64",
            FnSig {
                params: vec![Ty::I64],
                return_ty: Ty::Unit,
                effects: BTreeSet::new(),
            },
        );
        env.define_fn(
            "debug_u64",
            FnSig {
                params: vec![Ty::U64],
                return_ty: Ty::Unit,
                effects: BTreeSet::new(),
            },
        );
        env.define_fn(
            "fs_read",
            FnSig {
                params: vec![Ty::Str],
                return_ty: Ty::Str,
                effects: [Effect::Read].into_iter().collect(),
            },
        );
        env.define_fn(
            "fs_write",
            FnSig {
                params: vec![Ty::Str, Ty::Str],
                return_ty: Ty::I64,
                effects: [Effect::Write].into_iter().collect(),
            },
        );
        env.define_fn(
            "fs_exists",
            FnSig {
                params: vec![Ty::Str],
                return_ty: Ty::I64,
                effects: [Effect::Read].into_iter().collect(),
            },
        );
        env.define_fn(
            "fs_mkdir",
            FnSig {
                params: vec![Ty::Str],
                return_ty: Ty::I64,
                effects: [Effect::IO].into_iter().collect(),
            },
        );
        env.define_fn(
            "fs_listdir",
            FnSig {
                params: vec![Ty::Str],
                return_ty: Ty::Applied(Box::new(Ty::Struct("Vec".to_string())), vec![Ty::Str]),
                effects: [Effect::Read].into_iter().collect(),
            },
        );
        env.define_fn(
            "fs_remove",
            FnSig {
                params: vec![Ty::Str],
                return_ty: Ty::I64,
                effects: [Effect::IO].into_iter().collect(),
            },
        );
        env.define_fn(
            "fs_remove_dir",
            FnSig {
                params: vec![Ty::Str],
                return_ty: Ty::I64,
                effects: [Effect::IO].into_iter().collect(),
            },
        );
        env.define_fn(
            "fs_is_dir",
            FnSig {
                params: vec![Ty::Str],
                return_ty: Ty::I64,
                effects: [Effect::Read].into_iter().collect(),
            },
        );
        env.define_fn(
            "fs_rename",
            FnSig {
                params: vec![Ty::Str, Ty::Str],
                return_ty: Ty::I64,
                effects: [Effect::IO].into_iter().collect(),
            },
        );
        env.define_fn(
            "string_substr",
            FnSig {
                params: vec![Ty::Str, Ty::I64, Ty::I64],
                return_ty: Ty::Str,
                effects: BTreeSet::new(),
            },
        );
        env.define_fn(
            "string_split",
            FnSig {
                params: vec![Ty::Str, Ty::Str],
                return_ty: Ty::Applied(Box::new(Ty::Struct("Vec".to_string())), vec![Ty::Str]),
                effects: BTreeSet::new(),
            },
        );
        env.define_fn(
            "string_starts_with",
            FnSig {
                params: vec![Ty::Str, Ty::Str],
                return_ty: Ty::I64,
                effects: BTreeSet::new(),
            },
        );
        env.define_fn(
            "string_ends_with",
            FnSig {
                params: vec![Ty::Str, Ty::Str],
                return_ty: Ty::I64,
                effects: BTreeSet::new(),
            },
        );
        env.define_fn(
            "string_trim",
            FnSig {
                params: vec![Ty::Str],
                return_ty: Ty::Str,
                effects: BTreeSet::new(),
            },
        );
        env.define_fn(
            "string_to_upper",
            FnSig {
                params: vec![Ty::Str],
                return_ty: Ty::Str,
                effects: BTreeSet::new(),
            },
        );
        env.define_fn(
            "string_to_lower",
            FnSig {
                params: vec![Ty::Str],
                return_ty: Ty::Str,
                effects: BTreeSet::new(),
            },
        );
        env.define_fn(
            "string_replace",
            FnSig {
                params: vec![Ty::Str, Ty::Str, Ty::Str],
                return_ty: Ty::Str,
                effects: BTreeSet::new(),
            },
        );
        env.define_fn(
            "string_join",
            FnSig {
                params: vec![
                    Ty::Applied(Box::new(Ty::Struct("Vec".to_string())), vec![Ty::Str]),
                    Ty::Str,
                ],
                return_ty: Ty::Str,
                effects: BTreeSet::new(),
            },
        );
        env.define_fn(
            "parse_i64",
            FnSig {
                params: vec![Ty::Str],
                return_ty: Ty::I64,
                effects: BTreeSet::new(),
            },
        );
        env.define_fn(
            "i64_to_string",
            FnSig {
                params: vec![Ty::I64],
                return_ty: Ty::Str,
                effects: BTreeSet::new(),
            },
        );
        env.define_fn(
            "vec_sort",
            FnSig {
                params: vec![Ty::Applied(
                    Box::new(Ty::Struct("Vec".to_string())),
                    vec![Ty::I64],
                )],
                return_ty: Ty::Applied(Box::new(Ty::Struct("Vec".to_string())), vec![Ty::I64]),
                effects: BTreeSet::new(),
            },
        );
        env.define_fn(
            "time_unix",
            FnSig {
                params: vec![],
                return_ty: Ty::I64,
                effects: [Effect::IO].into_iter().collect(),
            },
        );
        env.define_fn(
            "time_unix_ms",
            FnSig {
                params: vec![],
                return_ty: Ty::I64,
                effects: [Effect::IO].into_iter().collect(),
            },
        );
        env.define_fn(
            "num_cpus",
            FnSig {
                params: vec![],
                return_ty: Ty::I64,
                effects: [Effect::IO].into_iter().collect(),
            },
        );
        env.define_fn(
            "hex_encode",
            FnSig {
                params: vec![Ty::Applied(
                    Box::new(Ty::Struct("Vec".to_string())),
                    vec![Ty::U8],
                )],
                return_ty: Ty::Str,
                effects: BTreeSet::new(),
            },
        );
        env.define_fn(
            "hex_decode",
            FnSig {
                params: vec![Ty::Str],
                return_ty: Ty::Applied(Box::new(Ty::Struct("Vec".to_string())), vec![Ty::U8]),
                effects: BTreeSet::new(),
            },
        );
        env.define_fn(
            "eprintln_str",
            FnSig {
                params: vec![Ty::Str],
                return_ty: Ty::Unit,
                effects: [Effect::IO].into_iter().collect(),
            },
        );
        env.define_fn(
            "args",
            FnSig {
                params: vec![],
                return_ty: Ty::Applied(Box::new(Ty::Struct("Vec".to_string())), vec![Ty::Str]),
                effects: [Effect::Read].into_iter().collect(),
            },
        );
        env.define_fn(
            "stdin_read",
            FnSig {
                params: vec![],
                return_ty: Ty::Str,
                effects: [Effect::Read].into_iter().collect(),
            },
        );
        env.define_fn(
            "stdin_read_line",
            FnSig {
                params: vec![],
                return_ty: Ty::Str,
                effects: [Effect::Read].into_iter().collect(),
            },
        );
        env.define_fn(
            "stdin_ready",
            FnSig {
                params: vec![],
                return_ty: Ty::Bool,
                effects: [Effect::Read].into_iter().collect(),
            },
        );
        env.define_fn(
            "process_exit",
            FnSig {
                params: vec![Ty::I64],
                return_ty: Ty::Never,
                effects: [Effect::IO].into_iter().collect(),
            },
        );

        env.define_fn(
            "process_run",
            FnSig {
                params: vec![
                    Ty::Str,
                    Ty::Applied(Box::new(Ty::Struct("Vec".to_string())), vec![Ty::Str]),
                ],
                return_ty: Ty::I64,
                effects: [Effect::IO].into_iter().collect(),
            },
        );
        env.define_fn(
            "process_get_stdout",
            FnSig {
                params: vec![],
                return_ty: Ty::Str,
                effects: [Effect::IO].into_iter().collect(),
            },
        );
        env.define_fn(
            "process_get_stderr",
            FnSig {
                params: vec![],
                return_ty: Ty::Str,
                effects: [Effect::IO].into_iter().collect(),
            },
        );

        env.define_fn(
            "process_start",
            FnSig {
                params: vec![
                    Ty::Str,
                    Ty::Applied(Box::new(Ty::Struct("Vec".to_string())), vec![Ty::Str]),
                ],
                return_ty: Ty::I64,
                effects: [Effect::IO].into_iter().collect(),
            },
        );
        env.define_fn(
            "process_wait",
            FnSig {
                params: vec![Ty::I64],
                return_ty: Ty::I64,
                effects: [Effect::IO].into_iter().collect(),
            },
        );
        env.define_fn(
            "process_wait_timeout",
            FnSig {
                params: vec![Ty::I64, Ty::I64],
                return_ty: Ty::I64,
                effects: [Effect::IO].into_iter().collect(),
            },
        );
        env.define_fn(
            "process_kill",
            FnSig {
                params: vec![Ty::I64],
                return_ty: Ty::I64,
                effects: [Effect::IO].into_iter().collect(),
            },
        );
        env.define_fn(
            "process_stdout_for",
            FnSig {
                params: vec![Ty::I64],
                return_ty: Ty::Str,
                effects: [Effect::IO].into_iter().collect(),
            },
        );
        env.define_fn(
            "process_stderr_for",
            FnSig {
                params: vec![Ty::I64],
                return_ty: Ty::Str,
                effects: [Effect::IO].into_iter().collect(),
            },
        );

        // Cranelift shim FFI functions (used by the self-hosted compiler's clif.vow)
        env.define_fn(
            "__vow_clif_create",
            FnSig {
                params: vec![Ty::I64, Ty::I64],
                return_ty: Ty::I64,
                effects: [Effect::IO].into_iter().collect(),
            },
        );
        env.define_fn(
            "__vow_clif_add_string",
            FnSig {
                params: vec![Ty::I64, Ty::Str],
                return_ty: Ty::Unit,
                effects: [Effect::IO].into_iter().collect(),
            },
        );
        env.define_fn(
            "__vow_clif_declare_extern",
            FnSig {
                params: vec![Ty::I64, Ty::Str],
                return_ty: Ty::Unit,
                effects: [Effect::IO].into_iter().collect(),
            },
        );
        env.define_fn(
            "__vow_clif_declare_function",
            FnSig {
                params: vec![
                    Ty::I64,
                    Ty::I64,
                    Ty::Str,
                    Ty::Applied(Box::new(Ty::Struct("Vec".to_string())), vec![Ty::I64]),
                    Ty::I64,
                    Ty::I64,
                    Ty::I64,
                ],
                return_ty: Ty::Unit,
                effects: [Effect::IO].into_iter().collect(),
            },
        );
        // Incremental per-function Cranelift FFI. Replaces the monolithic
        // __vow_clif_compile_function; the self-hosted clif.vow now streams
        // blocks/instructions/vow entries into shim-owned scratch buffers
        // that are reused across functions.
        env.define_fn(
            "__vow_clif_fn_begin",
            FnSig {
                params: vec![
                    Ty::I64,
                    Ty::I64,
                    Ty::I64,
                    Ty::Applied(Box::new(Ty::Struct("Vec".to_string())), vec![Ty::I64]),
                ],
                return_ty: Ty::I64,
                effects: [Effect::IO].into_iter().collect(),
            },
        );
        env.define_fn(
            "__vow_clif_fn_block",
            FnSig {
                params: vec![Ty::I64],
                return_ty: Ty::I64,
                effects: [Effect::IO].into_iter().collect(),
            },
        );
        env.define_fn(
            "__vow_clif_fn_inst",
            FnSig {
                params: vec![
                    Ty::I64,
                    Ty::I64,
                    Ty::I64,
                    Ty::I64,
                    Ty::I64,
                    Ty::I64,
                    Ty::I64,
                    Ty::Str,
                    Ty::Applied(Box::new(Ty::Struct("Vec".to_string())), vec![Ty::I64]),
                    Ty::I64,
                ],
                return_ty: Ty::I64,
                effects: [Effect::IO].into_iter().collect(),
            },
        );
        env.define_fn(
            "__vow_clif_fn_vow",
            FnSig {
                params: vec![
                    Ty::I64,
                    Ty::I64,
                    Ty::Str,
                    Ty::Applied(Box::new(Ty::Struct("Vec".to_string())), vec![Ty::I64]),
                    Ty::Applied(Box::new(Ty::Struct("Vec".to_string())), vec![Ty::Str]),
                ],
                return_ty: Ty::I64,
                effects: [Effect::IO].into_iter().collect(),
            },
        );
        env.define_fn(
            "__vow_clif_fn_end",
            FnSig {
                params: vec![Ty::I64],
                return_ty: Ty::I64,
                effects: [Effect::IO].into_iter().collect(),
            },
        );
        env.define_fn(
            "__vow_clif_finish",
            FnSig {
                params: vec![Ty::I64, Ty::Str],
                return_ty: Ty::I64,
                effects: [Effect::IO].into_iter().collect(),
            },
        );
        env.define_fn(
            "__vow_clif_link",
            FnSig {
                params: vec![Ty::Str, Ty::Str],
                return_ty: Ty::I64,
                effects: [Effect::IO].into_iter().collect(),
            },
        );
        env.define_fn(
            "__vow_clif_destroy",
            FnSig {
                params: vec![Ty::I64],
                return_ty: Ty::Unit,
                effects: [Effect::IO].into_iter().collect(),
            },
        );

        env
    }

    pub fn push_scope(&mut self) {
        self.scopes.push(HashMap::new());
    }

    pub fn pop_scope(&mut self) {
        assert!(self.scopes.len() > 1, "cannot pop the last scope");
        self.scopes.pop();
    }

    pub fn define(&mut self, name: &str, ty: Ty) {
        self.scopes
            .last_mut()
            .expect("at least one scope must exist")
            .insert(name.to_string(), ty);
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
        // Walk scopes inner→outer so shadowing inner-scope bindings win over
        // outer-scope leftovers under the cap. Each scope's contribution is
        // selected with a bounded max-heap (capacity = `remaining`), so the
        // hint path uses O(max_names) memory and O(N · log max_names) time
        // independent of how many bindings the program declares.
        if max_names == 0 {
            return Vec::new();
        }
        let mut names: Vec<String> = Vec::with_capacity(max_names.min(32));
        let mut seen: HashSet<&str> = HashSet::with_capacity(max_names.min(32));
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
                } else if let Some(&top) = heap.peek() {
                    if key < top {
                        heap.pop();
                        heap.push(key);
                    }
                }
            }
            let mut scope_names: Vec<&String> = heap.into_iter().collect();
            scope_names.sort_unstable();
            for key in scope_names {
                if seen.insert(key.as_str()) {
                    names.push(key.clone());
                    if names.len() >= max_names {
                        break;
                    }
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
                        let arg = args
                            .first()
                            .ok_or_else(|| "Option requires one type argument".to_string())?;
                        let t = self.resolve(arg)?;
                        return Ok(Ty::Applied(
                            Box::new(Ty::Enum("Option".to_string())),
                            vec![t],
                        ));
                    }
                    "Result" => {
                        let t = args
                            .first()
                            .ok_or_else(|| "Result requires two type arguments".to_string())?;
                        let e = args
                            .get(1)
                            .ok_or_else(|| "Result requires two type arguments".to_string())?;
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
            AstType::Refinement { base, .. } => self.resolve(base),
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
