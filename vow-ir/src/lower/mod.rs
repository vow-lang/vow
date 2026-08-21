pub mod vow;

use std::collections::{HashMap, HashSet};
use std::rc::Rc;

use vow_diag::Blame;
use vow_syntax::ast::{
    BinOp, Block, Effect, Expr, ExprKind, FnDef, Item, Lit, Module as AstModule, PatKind, Stmt,
    Type as AstType, UnOp, VariantKind, VowBlock, loop_break_values,
};
use vow_syntax::span::Span;
pub use vow_types::check::{
    PatternAggregateInfo, PatternAggregateMap, PatternScalarType, StringExprSet,
};

use crate::types::{
    BasicBlock, BlockId, EnumLayout, FieldLayout, FuncId, Function, Inst, InstData, InstId,
    IntegerType, Module, Opcode, RegionId, RegionSummary, StructLayout, Ty, VariantLayout,
    VowEntry, VowId,
};

fn vow_debug_builtin_to_runtime(name: &str) -> Option<(&'static str, Ty)> {
    match name {
        "debug_str" => Some(("__vow_debug_str", Ty::Unit)),
        "debug_i64" => Some(("__vow_debug_i64", Ty::Unit)),
        "debug_u64" => Some(("__vow_debug_u64", Ty::Unit)),
        _ => None,
    }
}

fn vow_static_builtin_to_runtime(name: &str) -> Option<(&'static str, Ty)> {
    match name {
        "print_str" => Some(("__vow_string_print", Ty::Unit)),
        "print_i64" => Some(("__vow_print_i64", Ty::Unit)),
        "print_u64" => Some(("__vow_print_u64", Ty::Unit)),
        "eprintln_str" => Some(("__vow_eprintln_str", Ty::Unit)),
        "fs_read" => Some(("__vow_fs_read", Ty::Ptr)),
        "fs_open" => Some(("__vow_fs_open", Ty::I64)),
        "fs_read_line" => Some(("__vow_fs_read_line", Ty::Ptr)),
        "fs_status" => Some(("__vow_fs_status", Ty::I64)),
        "fs_close" => Some(("__vow_fs_close", Ty::I64)),
        "fs_write" => Some(("__vow_fs_write", Ty::I64)),
        "fs_exists" => Some(("__vow_fs_exists", Ty::I64)),
        "fs_mkdir" => Some(("__vow_fs_mkdir", Ty::I64)),
        "fs_listdir" => Some(("__vow_fs_listdir", Ty::Ptr)),
        "fs_remove" => Some(("__vow_fs_remove", Ty::I64)),
        "fs_remove_dir" => Some(("__vow_fs_remove_dir", Ty::I64)),
        "fs_is_dir" => Some(("__vow_fs_is_dir", Ty::I64)),
        "fs_is_symlink" => Some(("__vow_fs_is_symlink", Ty::I64)),
        "fs_rename" => Some(("__vow_fs_rename", Ty::I64)),
        "string_substr" => Some(("__vow_string_substr", Ty::Ptr)),
        "string_split" => Some(("__vow_string_split", Ty::Ptr)),
        "string_starts_with" => Some(("__vow_string_starts_with", Ty::I64)),
        "string_ends_with" => Some(("__vow_string_ends_with", Ty::I64)),
        "string_trim" => Some(("__vow_string_trim", Ty::Ptr)),
        "string_to_upper" => Some(("__vow_string_to_upper", Ty::Ptr)),
        "string_to_lower" => Some(("__vow_string_to_lower", Ty::Ptr)),
        "string_replace" => Some(("__vow_string_replace", Ty::Ptr)),
        "string_join" => Some(("__vow_string_join", Ty::Ptr)),
        "parse_i64" => Some(("__vow_string_parse_i64_opt", Ty::Ptr)),
        "parse_u8" => Some(("__vow_string_parse_u8_opt", Ty::Ptr)),
        "parse_i8" => Some(("__vow_string_parse_i8_opt", Ty::Ptr)),
        "parse_i16" => Some(("__vow_string_parse_i16_opt", Ty::Ptr)),
        "parse_u16" => Some(("__vow_string_parse_u16_opt", Ty::Ptr)),
        "parse_u32" => Some(("__vow_string_parse_u32_opt", Ty::Ptr)),
        "i16_to_u8_try" => Some(("__vow_i16_to_u8_try", Ty::Ptr)),
        "i16_to_u8_wrap" => Some(("__vow_i16_to_u8_wrap", Ty::U8)),
        "i16_to_u8_sat" => Some(("__vow_i16_to_u8_sat", Ty::U8)),
        "i32_to_u8_try" => Some(("__vow_i32_to_u8_try", Ty::Ptr)),
        "i32_to_u8_wrap" => Some(("__vow_i32_to_u8_wrap", Ty::U8)),
        "i32_to_u8_sat" => Some(("__vow_i32_to_u8_sat", Ty::U8)),
        "i64_to_u8_try" => Some(("__vow_i64_to_u8_try", Ty::Ptr)),
        "i64_to_u8_wrap" => Some(("__vow_i64_to_u8_wrap", Ty::U8)),
        "i64_to_u8_sat" => Some(("__vow_i64_to_u8_sat", Ty::U8)),
        "i128_to_u8_try" => Some(("__vow_i128_to_u8_try", Ty::Ptr)),
        "i128_to_u8_wrap" => Some(("__vow_i128_to_u8_wrap", Ty::U8)),
        "i128_to_u8_sat" => Some(("__vow_i128_to_u8_sat", Ty::U8)),
        "u16_to_u8_try" => Some(("__vow_u16_to_u8_try", Ty::Ptr)),
        "u16_to_u8_wrap" => Some(("__vow_u16_to_u8_wrap", Ty::U8)),
        "u16_to_u8_sat" => Some(("__vow_u16_to_u8_sat", Ty::U8)),
        "u32_to_u8_try" => Some(("__vow_u32_to_u8_try", Ty::Ptr)),
        "u32_to_u8_wrap" => Some(("__vow_u32_to_u8_wrap", Ty::U8)),
        "u32_to_u8_sat" => Some(("__vow_u32_to_u8_sat", Ty::U8)),
        "u64_to_u8_try" => Some(("__vow_u64_to_u8_try", Ty::Ptr)),
        "u64_to_u8_wrap" => Some(("__vow_u64_to_u8_wrap", Ty::U8)),
        "u64_to_u8_sat" => Some(("__vow_u64_to_u8_sat", Ty::U8)),
        "u128_to_u8_try" => Some(("__vow_u128_to_u8_try", Ty::Ptr)),
        "u128_to_u8_wrap" => Some(("__vow_u128_to_u8_wrap", Ty::U8)),
        "u128_to_u8_sat" => Some(("__vow_u128_to_u8_sat", Ty::U8)),
        "add_sat_u8" => Some(("__vow_add_sat_u8", Ty::U8)),
        "sub_sat_u8" => Some(("__vow_sub_sat_u8", Ty::U8)),
        "mul_sat_u8" => Some(("__vow_mul_sat_u8", Ty::U8)),
        "parse_i32" => Some(("__vow_string_parse_i32_opt", Ty::Ptr)),
        "i64_to_i32_try" => Some(("__vow_i64_to_i32_try", Ty::Ptr)),
        "i64_to_i32_wrap" => Some(("__vow_i64_to_i32_wrap", Ty::I32)),
        "i64_to_i32_sat" => Some(("__vow_i64_to_i32_sat", Ty::I32)),
        "u32_to_i32_try" => Some(("__vow_u32_to_i32_try", Ty::Ptr)),
        "u32_to_i32_wrap" => Some(("__vow_u32_to_i32_wrap", Ty::I32)),
        "u32_to_i32_sat" => Some(("__vow_u32_to_i32_sat", Ty::I32)),
        "u64_to_i32_try" => Some(("__vow_u64_to_i32_try", Ty::Ptr)),
        "u64_to_i32_wrap" => Some(("__vow_u64_to_i32_wrap", Ty::I32)),
        "u64_to_i32_sat" => Some(("__vow_u64_to_i32_sat", Ty::I32)),
        "int_to_string" | "i64_to_string" => Some(("__vow_string_from_i64", Ty::Ptr)),
        "uint_to_string" => Some(("__vow_string_from_u64", Ty::Ptr)),
        "vec_sort" => Some(("__vow_vec_sort", Ty::Ptr)),
        "time_unix" => Some(("__vow_time_unix", Ty::I64)),
        "time_unix_ms" => Some(("__vow_time_unix_ms", Ty::I64)),
        "num_cpus" => Some(("__vow_num_cpus", Ty::I64)),
        "memory_root_arena_bytes" => Some(("__vow_memory_root_arena_bytes", Ty::U64)),
        "memory_peak_bytes" => Some(("__vow_memory_peak_bytes", Ty::U64)),
        "memory_alloc_count_since_start" => Some(("__vow_memory_alloc_count_since_start", Ty::U64)),
        "time_micros" => Some(("__vow_time_micros", Ty::I64)),
        "proc_sample" => Some(("__vow_proc_sample", Ty::Ptr)),
        "gzip_write_file" => Some(("__vow_gzip_write_file", Ty::I64)),
        "hex_encode" => Some(("__vow_hex_encode", Ty::Ptr)),
        "hex_decode" => Some(("__vow_hex_decode", Ty::Ptr)),
        "args" => Some(("__vow_args", Ty::Ptr)),
        "stdin_read" => Some(("__vow_stdin_read", Ty::Ptr)),
        "stdin_read_line" => Some(("__vow_stdin_read_line", Ty::Ptr)),
        "stdin_ready" => Some(("__vow_stdin_ready", Ty::Bool)),
        "process_exit" => Some(("__vow_process_exit", Ty::Unit)),
        "process_run" => Some(("__vow_process_run", Ty::I64)),
        "process_get_stdout" => Some(("__vow_process_get_stdout", Ty::Ptr)),
        "process_get_stderr" => Some(("__vow_process_get_stderr", Ty::Ptr)),
        "process_start" => Some(("__vow_process_start", Ty::I64)),
        "process_wait" => Some(("__vow_process_wait", Ty::I64)),
        "process_wait_timeout" => Some(("__vow_process_wait_timeout", Ty::I64)),
        "process_poll_wait" => Some(("__vow_process_poll_wait", Ty::I64)),
        "process_kill" => Some(("__vow_process_kill", Ty::I64)),
        "process_stdout_for" => Some(("__vow_process_stdout_for", Ty::Ptr)),
        "process_stderr_for" => Some(("__vow_process_stderr_for", Ty::Ptr)),
        "__vow_clif_create" => Some(("__vow_clif_create", Ty::I64)),
        "__vow_clif_add_string" => Some(("__vow_clif_add_string", Ty::Unit)),
        "__vow_clif_declare_extern" => Some(("__vow_clif_declare_extern", Ty::Unit)),
        "__vow_clif_declare_function" => Some(("__vow_clif_declare_function", Ty::Unit)),
        "__vow_clif_fn_begin" => Some(("__vow_clif_fn_begin", Ty::I64)),
        "__vow_clif_fn_block" => Some(("__vow_clif_fn_block", Ty::I64)),
        "__vow_clif_fn_inst" => Some(("__vow_clif_fn_inst", Ty::I64)),
        "__vow_clif_fn_vow" => Some(("__vow_clif_fn_vow", Ty::I64)),
        "__vow_clif_fn_end" => Some(("__vow_clif_fn_end", Ty::I64)),
        "__vow_clif_finish" => Some(("__vow_clif_finish", Ty::I64)),
        "__vow_clif_link" => Some(("__vow_clif_link", Ty::I64)),
        "__vow_clif_destroy" => Some(("__vow_clif_destroy", Ty::Unit)),
        _ => None,
    }
}

fn narrow_intrinsic_target(name: &str) -> Option<Ty> {
    let (source, rest) = name.split_once("_to_")?;
    let (target, mode) = rest.rsplit_once('_')?;
    if !matches!(mode, "try" | "wrap" | "sat") {
        return None;
    }
    let supported = matches!(
        (source, target),
        ("i16" | "u16" | "i32" | "u32" | "i64" | "u64", "i8")
            | ("i32" | "u32" | "i64" | "u64", "i16" | "u16")
            | ("i64" | "u64", "u32")
    );
    if !supported {
        return None;
    }
    match target {
        "i8" => Some(Ty::I8),
        "i16" => Some(Ty::I16),
        "u16" => Some(Ty::U16),
        "u32" => Some(Ty::U32),
        _ => None,
    }
}

fn vow_builtin_to_runtime(name: &str) -> Option<(String, Ty)> {
    if let Some((symbol, ty)) = vow_static_builtin_to_runtime(name) {
        return Some((symbol.to_string(), ty));
    }
    narrow_intrinsic_target(name).map(|target| {
        let return_ty = if name.ends_with("_try") {
            Ty::Ptr
        } else {
            target
        };
        (format!("__vow_{name}"), return_ty)
    })
}

// Keep this list in sync with the builtin result tags in compiler/lower.vow.
// pin_to_root depends on these heap tags for direct builtin call results.
fn tag_builtin_result(ctx: &mut LowerCtx, name: &str, result: InstId) {
    if name.ends_with("_try")
        && let Some(target) = narrow_intrinsic_target(name)
    {
        ctx.inst_struct_type.insert(result, "Option".to_string());
        ctx.inst_option_elem_ty.insert(result, target);
        return;
    }
    match name {
        "fs_read" | "fs_read_line" | "stdin_read" | "stdin_read_line" | "string_substr"
        | "string_trim" | "string_to_upper" | "string_to_lower" | "string_replace"
        | "string_join" | "int_to_string" | "uint_to_string" | "i64_to_string" | "hex_encode"
        | "process_get_stdout" | "process_get_stderr" | "process_stdout_for"
        | "process_stderr_for" => {
            ctx.inst_struct_type.insert(result, "String".to_string());
        }
        "args" | "fs_listdir" | "string_split" | "vec_sort" | "hex_decode" => {
            ctx.inst_struct_type.insert(result, "Vec".to_string());
        }
        "parse_i8" => {
            ctx.inst_struct_type.insert(result, "Option".to_string());
            ctx.inst_option_elem_ty.insert(result, Ty::I8);
        }
        "parse_i16" => {
            ctx.inst_struct_type.insert(result, "Option".to_string());
            ctx.inst_option_elem_ty.insert(result, Ty::I16);
        }
        "parse_u16" => {
            ctx.inst_struct_type.insert(result, "Option".to_string());
            ctx.inst_option_elem_ty.insert(result, Ty::U16);
        }
        "parse_u32" => {
            ctx.inst_struct_type.insert(result, "Option".to_string());
            ctx.inst_option_elem_ty.insert(result, Ty::U32);
        }
        "parse_u8" | "i16_to_u8_try" | "i32_to_u8_try" | "i64_to_u8_try" | "i128_to_u8_try"
        | "u16_to_u8_try" | "u32_to_u8_try" | "u64_to_u8_try" | "u128_to_u8_try" => {
            ctx.inst_struct_type.insert(result, "Option".to_string());
            ctx.inst_option_elem_ty.insert(result, Ty::U8);
        }
        "parse_i32" | "i64_to_i32_try" | "u32_to_i32_try" | "u64_to_i32_try" => {
            ctx.inst_struct_type.insert(result, "Option".to_string());
            ctx.inst_option_elem_ty.insert(result, Ty::I32);
        }
        "parse_i64" => {
            ctx.inst_struct_type.insert(result, "Option".to_string());
            ctx.inst_option_elem_ty.insert(result, Ty::I64);
        }
        _ => {}
    }
}

fn propagate_vec_element_metadata(ctx: &mut LowerCtx, source: InstId, result: InstId) {
    let Some(elem_types) = ctx.inst_vec_elem_types.get(&source).cloned() else {
        return;
    };
    let Some((elem_name, remaining)) = elem_types.split_first() else {
        return;
    };
    ctx.inst_struct_type.insert(result, elem_name.clone());
    let option_types = ctx
        .inst_vec_option_elem_tys
        .get(&source)
        .cloned()
        .unwrap_or_else(|| vec![None; elem_types.len()]);
    if let Some(Some(elem_type)) = option_types.first() {
        ctx.inst_option_elem_ty.insert(result, *elem_type);
    }
    let variant_types = ctx
        .inst_vec_variant_payload_tys
        .get(&source)
        .cloned()
        .unwrap_or_else(|| vec![Vec::new(); elem_types.len()]);
    if let Some(elem_variant_types) = variant_types.first()
        && elem_variant_types.iter().any(Option::is_some)
    {
        ctx.inst_variant_payload_tys
            .insert(result, elem_variant_types.clone());
    }
    if !remaining.is_empty() {
        ctx.inst_vec_elem_types.insert(result, remaining.to_vec());
        let remaining_option_types = option_types.into_iter().skip(1).collect::<Vec<_>>();
        if remaining_option_types.iter().any(Option::is_some) {
            ctx.inst_vec_option_elem_tys
                .insert(result, remaining_option_types);
        }
        let remaining_variant_types = variant_types.into_iter().skip(1).collect::<Vec<_>>();
        if remaining_variant_types
            .iter()
            .flatten()
            .any(Option::is_some)
        {
            ctx.inst_vec_variant_payload_tys
                .insert(result, remaining_variant_types);
        }
    }
}

fn pattern_scalar_ir_type(ty: PatternScalarType) -> Ty {
    match ty {
        PatternScalarType::I8 => Ty::I8,
        PatternScalarType::I16 => Ty::I16,
        PatternScalarType::I32 => Ty::I32,
        PatternScalarType::I64 => Ty::I64,
        PatternScalarType::I128 => Ty::I128,
        PatternScalarType::U8 => Ty::U8,
        PatternScalarType::U16 => Ty::U16,
        PatternScalarType::U32 => Ty::U32,
        PatternScalarType::U64 => Ty::U64,
        PatternScalarType::U128 => Ty::U128,
        PatternScalarType::F32 => Ty::F32,
        PatternScalarType::F64 => Ty::F64,
        PatternScalarType::Bool => Ty::Bool,
    }
}

fn tag_pattern_aggregate_metadata(ctx: &mut LowerCtx, result: InstId, info: PatternAggregateInfo) {
    ctx.inst_struct_type.insert(result, info.type_name);
    if !info.vec_elem_types.is_empty() {
        let option_types = info
            .vec_option_elem_types
            .into_iter()
            .map(|ty| ty.map(pattern_scalar_ir_type))
            .collect::<Vec<_>>();
        ctx.inst_vec_elem_types.insert(result, info.vec_elem_types);
        if option_types.iter().any(Option::is_some) {
            ctx.inst_vec_option_elem_tys.insert(result, option_types);
        }
        let variant_types = info
            .vec_variant_payload_types
            .into_iter()
            .map(|variant| {
                variant
                    .into_iter()
                    .map(|ty| ty.map(pattern_scalar_ir_type))
                    .collect::<Vec<_>>()
            })
            .collect::<Vec<_>>();
        if variant_types.iter().flatten().any(Option::is_some) {
            ctx.inst_vec_variant_payload_tys
                .insert(result, variant_types);
        }
    }
    if let Some(elem_type) = info.option_elem_type {
        ctx.inst_option_elem_ty
            .insert(result, pattern_scalar_ir_type(elem_type));
    }
    let variant_types = info
        .variant_payload_types
        .into_iter()
        .map(|ty| ty.map(pattern_scalar_ir_type))
        .collect::<Vec<_>>();
    if variant_types.iter().any(Option::is_some) {
        ctx.inst_variant_payload_tys.insert(result, variant_types);
    }
}

fn compatible_metadata_value<T: Clone + Eq>(
    sources: &[InstId],
    get: impl Fn(InstId) -> Option<T>,
) -> Result<Option<T>, ()> {
    let mut compatible = None;
    for source in sources {
        let Some(value) = get(*source) else {
            continue;
        };
        if compatible.as_ref().is_some_and(|known| known != &value) {
            return Err(());
        }
        compatible = Some(value);
    }
    Ok(compatible)
}

fn copy_compatible_aggregate_metadata(ctx: &mut LowerCtx, sources: &[InstId], result: InstId) {
    let Some(type_name) = sources
        .first()
        .and_then(|source| ctx.inst_struct_type.get(source))
        .cloned()
    else {
        return;
    };
    if !sources
        .iter()
        .all(|source| ctx.inst_struct_type.get(source) == Some(&type_name))
    {
        return;
    }

    let Ok(vec_elem_types) = compatible_metadata_value(sources, |source| {
        ctx.inst_vec_elem_types.get(&source).cloned()
    }) else {
        return;
    };
    let Ok(vec_option_elem_tys) = compatible_metadata_value(sources, |source| {
        ctx.inst_vec_option_elem_tys.get(&source).cloned()
    }) else {
        return;
    };
    let Ok(vec_variant_payload_tys) = compatible_metadata_value(sources, |source| {
        ctx.inst_vec_variant_payload_tys.get(&source).cloned()
    }) else {
        return;
    };
    let Ok(option_elem_ty) = compatible_metadata_value(sources, |source| {
        ctx.inst_option_elem_ty.get(&source).copied()
    }) else {
        return;
    };
    let Ok(variant_payload_tys) = compatible_metadata_value(sources, |source| {
        ctx.inst_variant_payload_tys.get(&source).cloned()
    }) else {
        return;
    };

    ctx.inst_struct_type.insert(result, type_name);
    if let Some(types) = vec_elem_types {
        ctx.inst_vec_elem_types.insert(result, types);
    }
    if let Some(types) = vec_option_elem_tys {
        ctx.inst_vec_option_elem_tys.insert(result, types);
    }
    if let Some(types) = vec_variant_payload_tys {
        ctx.inst_vec_variant_payload_tys.insert(result, types);
    }
    if let Some(ty) = option_elem_ty {
        ctx.inst_option_elem_ty.insert(result, ty);
    }
    if let Some(types) = variant_payload_tys {
        ctx.inst_variant_payload_tys.insert(result, types);
    }
}

fn ast_type_is_linear_owner(ast_ty: &AstType, linear_owner_names: &HashSet<String>) -> bool {
    match ast_ty {
        AstType::Named { name, .. } => linear_owner_names.contains(name),
        AstType::Generic { name, args, .. } if name == "Option" || name == "Result" => args
            .iter()
            .any(|arg| ast_type_is_linear_owner(arg, linear_owner_names)),
        _ => false,
    }
}

fn resolve_type_alias<'a>(
    ast_ty: &'a AstType,
    type_aliases: &'a HashMap<String, AstType>,
) -> &'a AstType {
    match ast_ty {
        AstType::Named { name, .. } => type_aliases.get(name).unwrap_or(ast_ty),
        _ => ast_ty,
    }
}

fn lower_ty_with_linear(
    ast_ty: &AstType,
    linear_owner_names: &HashSet<String>,
    type_aliases: &HashMap<String, AstType>,
) -> Ty {
    let resolved_ty = resolve_type_alias(ast_ty, type_aliases);
    match resolved_ty {
        AstType::Named { name, .. } => match name.as_str() {
            "i8" => Ty::I8,
            "i16" => Ty::I16,
            "i32" => Ty::I32,
            "i64" => Ty::I64,
            "i128" => Ty::I128,
            "u8" => Ty::U8,
            "u16" => Ty::U16,
            "u32" => Ty::U32,
            "u64" => Ty::U64,
            "u128" => Ty::U128,
            "f32" => Ty::F32,
            "f64" => Ty::F64,
            "bool" => Ty::Bool,
            _ if linear_owner_names.contains(name) => Ty::LinearPtr,
            _ => Ty::Ptr,
        },
        AstType::Unit { .. } => Ty::Unit,
        AstType::Never { .. } => Ty::Unit,
        _ if ast_type_is_linear_owner(resolved_ty, linear_owner_names) => Ty::LinearPtr,
        _ => Ty::Ptr,
    }
}

fn resolve_type_alias_name(
    name: &str,
    direct: &HashMap<String, AstType>,
    resolved: &mut HashMap<String, AstType>,
    visiting: &mut HashSet<String>,
) -> AstType {
    if let Some(ty) = resolved.get(name) {
        return ty.clone();
    }
    let target = direct
        .get(name)
        .expect("type alias name originates from the direct map");
    if !visiting.insert(name.to_string()) {
        return target.clone();
    }
    let final_ty = match target {
        AstType::Named {
            name: target_name, ..
        } if direct.contains_key(target_name) => {
            resolve_type_alias_name(target_name, direct, resolved, visiting)
        }
        _ => target.clone(),
    };
    visiting.remove(name);
    resolved.insert(name.to_string(), final_ty.clone());
    final_ty
}

fn collect_type_aliases(module: &AstModule) -> HashMap<String, AstType> {
    let direct: HashMap<String, AstType> = module
        .items
        .iter()
        .filter_map(|item| match item {
            Item::TypeAlias(alias) => Some((alias.name.clone(), alias.ty.clone())),
            _ => None,
        })
        .collect();
    let mut resolved = HashMap::with_capacity(direct.len());
    for name in direct.keys() {
        resolve_type_alias_name(name, &direct, &mut resolved, &mut HashSet::new());
    }
    resolved
}

fn collect_linear_owner_names(module: &AstModule) -> HashSet<String> {
    let mut names: HashSet<String> = module
        .items
        .iter()
        .filter_map(|item| match item {
            Item::Struct(s) if s.is_linear => Some(s.name.clone()),
            _ => None,
        })
        .collect();

    loop {
        let newly_linear: Vec<String> = module
            .items
            .iter()
            .filter_map(|item| {
                let (name, owns_linear) = match item {
                    Item::Enum(enum_def) => {
                        let owns_linear =
                            enum_def.variants.iter().any(|variant| match &variant.kind {
                                VariantKind::Unit => false,
                                VariantKind::Tuple(types) => {
                                    types.iter().any(|ty| ast_type_is_linear_owner(ty, &names))
                                }
                                VariantKind::Struct(fields) => fields
                                    .iter()
                                    .any(|field| ast_type_is_linear_owner(&field.ty, &names)),
                            });
                        (&enum_def.name, owns_linear)
                    }
                    Item::TypeAlias(alias) => {
                        (&alias.name, ast_type_is_linear_owner(&alias.ty, &names))
                    }
                    _ => return None,
                };
                (!names.contains(name) && owns_linear).then(|| name.clone())
            })
            .collect();
        if newly_linear.is_empty() {
            return names;
        }
        names.extend(newly_linear);
    }
}

fn is_scalar_field_type_name(name: &str) -> bool {
    matches!(
        name,
        "i8" | "i16"
            | "i32"
            | "i64"
            | "i128"
            | "u8"
            | "u16"
            | "u32"
            | "u64"
            | "u128"
            | "f32"
            | "f64"
            | "bool"
    )
}

fn scalar_ty_for_field_type_name(name: &str) -> Ty {
    match name {
        "i8" => Ty::I8,
        "i16" => Ty::I16,
        "i32" => Ty::I32,
        "i64" => Ty::I64,
        "i128" => Ty::I128,
        "u8" => Ty::U8,
        "u16" => Ty::U16,
        "u32" => Ty::U32,
        "u64" => Ty::U64,
        "u128" => Ty::U128,
        "f32" => Ty::F32,
        "f64" => Ty::F64,
        "bool" => Ty::Bool,
        _ => Ty::I64,
    }
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct FuncSigInfo {
    id: FuncId,
    ret_ty: Ty,
    ret_tag: Option<String>,
    ret_vec_elem: Option<String>,
    ret_option_elem: Option<Ty>,
    param_tys: Vec<Ty>,
    param_ast_tys: Vec<AstType>,
}

fn non_scalar_type_tag(
    ast_ty: &AstType,
    type_aliases: &HashMap<String, AstType>,
) -> Option<String> {
    match resolve_type_alias(ast_ty, type_aliases) {
        AstType::Named { name, .. }
            if matches!(
                name.as_str(),
                "i8" | "i16"
                    | "i32"
                    | "i64"
                    | "i128"
                    | "u8"
                    | "u16"
                    | "u32"
                    | "u64"
                    | "u128"
                    | "f32"
                    | "f64"
                    | "bool"
            ) =>
        {
            None
        }
        AstType::Named { name, .. } if name == "str" => Some("String".to_string()),
        AstType::Named { name, .. } => Some(name.clone()),
        AstType::Generic { name, .. } => Some(name.clone()),
        _ => None,
    }
}

fn option_named_elem_type(ast_ty: &AstType, type_aliases: &HashMap<String, AstType>) -> Option<Ty> {
    match resolve_type_alias(ast_ty, type_aliases) {
        AstType::Generic { name, args, .. } if name == "Option" => args
            .first()
            .map(|ty| lower_ty_with_linear(ty, &HashSet::new(), type_aliases))
            .filter(|ty| !matches!(ty, Ty::Ptr | Ty::LinearPtr | Ty::Unit)),
        _ => None,
    }
}

fn vec_named_elem_type(
    ast_ty: &AstType,
    type_aliases: &HashMap<String, AstType>,
) -> Option<String> {
    match resolve_type_alias(ast_ty, type_aliases) {
        AstType::Generic { name, args, .. } if name == "Vec" => {
            args.first()
                .and_then(|elem_ty| match resolve_type_alias(elem_ty, type_aliases) {
                    AstType::Named { name, .. } if name == "str" => Some("String".to_string()),
                    AstType::Named { name, .. }
                        if !matches!(
                            name.as_str(),
                            "i8" | "i16"
                                | "i32"
                                | "i64"
                                | "i128"
                                | "u8"
                                | "u16"
                                | "u32"
                                | "u64"
                                | "u128"
                                | "f32"
                                | "f64"
                                | "bool"
                        ) =>
                    {
                        Some(name.clone())
                    }
                    AstType::Generic { name, .. } => Some(name.clone()),
                    _ => None,
                })
        }
        _ => None,
    }
}

fn type_tag_name(ast_ty: &AstType, type_aliases: &HashMap<String, AstType>) -> String {
    match resolve_type_alias(ast_ty, type_aliases) {
        AstType::Named { name, .. } if name == "str" => "String".to_string(),
        AstType::Named { name, .. } | AstType::Generic { name, .. } => name.clone(),
        _ => String::new(),
    }
}

const FIELD_IDX_SENTINEL: usize = u32::MAX as usize;

pub(crate) struct LowerCtx {
    pub(super) func: Function,
    pub(super) current_block: BlockId,
    next_inst_id: u32,
    scope: Vec<HashMap<String, InstId>>,
    pub(super) vow_block: Option<VowBlock>,
    pub(super) string_pool: Vec<String>,
    string_pool_index: HashMap<String, u32>,
    func_index: HashMap<String, FuncSigInfo>,
    // struct name → field names in declaration order
    pub(super) struct_field_map: HashMap<String, Vec<String>>,
    // enum name → variant names in declaration order (index = tag)
    pub(super) enum_variant_map: HashMap<String, Vec<String>>,
    // enum name → variant tag → declared payload types
    enum_variant_payload_tys: HashMap<String, Vec<Vec<Ty>>>,
    // enum name → variant tag → complete declared payload types
    enum_variant_payload_ast_types: Rc<HashMap<String, Vec<Vec<AstType>>>>,
    // Expressions inside a wide contextual control-flow expression, keyed by
    // their stable AST address for the duration of function lowering.
    wide_literal_contexts: HashMap<usize, Ty>,
    linear_owner_names: HashSet<String>,
    type_aliases: Rc<HashMap<String, AstType>>,
    // InstId of a struct/enum allocation → type name
    pub(super) inst_struct_type: HashMap<InstId, String>,
    inst_ty_cache: HashMap<InstId, Ty>,
    inst_locations: Vec<(BlockId, usize)>,
    phi_dependents: HashMap<InstId, Vec<InstId>>,
    // Complete declared return type for contextual explicit-return lowering.
    func_return_ast_ty: Option<AstType>,
    // source file path for vow entries
    file: String,
    // struct name → field type names (from AST declarations) for FieldGet auto-tagging
    pub(super) struct_field_type_names: HashMap<String, Vec<String>>,
    // struct name → complete declared field types for contextual aggregate lowering
    struct_field_ast_types: Rc<HashMap<String, Vec<AstType>>>,
    // expr addresses whose resolved type is String (from checker)
    string_exprs: StringExprSet,
    // const name → (compile-time value, declared type)
    const_map: HashMap<String, (i64, Ty)>,
    // loop exit block stack for break
    loop_exit_blocks: Vec<BlockId>,
    // loop header block stack for continue
    loop_header_blocks: Vec<BlockId>,
    // Per-loop Phi IDs for back-edge Upsilons on continue
    loop_continue_phis: Vec<Vec<(String, InstId)>>,
    // For for-each: the index Phi to increment on continue (None for while/loop)
    loop_continue_idx_phi: Vec<Option<InstId>>,
    // Scope depth at loop header (before body scope push) for correct continue resolution.
    // continue must resolve loop-carried vars from this depth, not the current scope, to
    // avoid picking up shadowed bindings in inner blocks.
    loop_continue_scope_depth: Vec<usize>,
    // Per-loop break-value Upsilon collector.  `Some(vec)` for `loop` (collects
    // (source_block, upsilon_id, value_ty)), `None` for `while`.
    loop_break_upsilons: Vec<Option<Vec<(BlockId, InstId, Ty)>>>,
    // Per-loop exit-block Phi IDs for mutation variables.  Break emits Upsilons
    // targeting these so the exit block receives updated values.
    loop_exit_phis: Vec<Vec<(String, InstId)>>,
    // InstId of a Vec allocation → aggregate element type names, outermost first.
    // A Vec<Vec<Box>> carries ["Vec", "Box"], so each index can consume one
    // name and retain the rest for deeper indexing.
    inst_vec_elem_types: HashMap<InstId, Vec<String>>,
    // Parallel Option payload widths for aggregate entries in inst_vec_elem_types.
    inst_vec_option_elem_tys: HashMap<InstId, Vec<Option<Ty>>>,
    // Parallel per-variant scalar payload widths for enum entries in inst_vec_elem_types.
    inst_vec_variant_payload_tys: HashMap<InstId, Vec<Vec<Option<Ty>>>>,
    // InstId of an Option-tagged value → its payload type (for Option::Some(v) match-arm FieldGet)
    inst_option_elem_ty: HashMap<InstId, Ty>,
    // InstId of an enum-tagged value → first scalar payload type per variant tag.
    inst_variant_payload_tys: HashMap<InstId, Vec<Option<Ty>>>,
    // Identifier-pattern address → checker-resolved aggregate metadata.
    pattern_aggregates: Rc<PatternAggregateMap>,
    // struct name → per-field Vec element type name (for FieldGet → Vec propagation)
    struct_field_vec_elems: HashMap<String, Vec<String>>,
    warnings: Vec<vow_diag::Diagnostic>,
}

impl LowerCtx {
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn new(
        name: String,
        params: Vec<Ty>,
        param_names: Vec<String>,
        return_ty: Ty,
        effects: Vec<Effect>,
        file: String,
        func_index: HashMap<String, FuncSigInfo>,
        struct_field_map: HashMap<String, Vec<String>>,
        enum_variant_map: HashMap<String, Vec<String>>,
        linear_owner_names: HashSet<String>,
        type_aliases: Rc<HashMap<String, AstType>>,
        struct_field_type_names: HashMap<String, Vec<String>>,
        struct_field_ast_types: Rc<HashMap<String, Vec<AstType>>>,
        struct_field_vec_elems: HashMap<String, Vec<String>>,
        string_exprs: StringExprSet,
        pattern_aggregates: Rc<PatternAggregateMap>,
    ) -> Self {
        let entry = BasicBlock {
            id: BlockId(0),
            insts: vec![],
        };
        let func = Function {
            id: FuncId(0),
            name,
            params,
            param_names,
            return_ty,
            effects,
            vows: vec![],
            blocks: vec![entry],
            local_names: HashMap::new(),
            summary: RegionSummary::default(),
            source_file: file.clone(),
        };
        let mut enum_variant_map = enum_variant_map;
        enum_variant_map
            .entry("Option".to_string())
            .or_insert_with(|| vec!["None".to_string(), "Some".to_string()]);
        enum_variant_map
            .entry("Result".to_string())
            .or_insert_with(|| vec!["Ok".to_string(), "Err".to_string()]);
        LowerCtx {
            func,
            current_block: BlockId(0),
            next_inst_id: 0,
            scope: vec![HashMap::new()],
            vow_block: None,
            string_pool: Vec::new(),
            string_pool_index: HashMap::new(),
            func_index,
            struct_field_map,
            enum_variant_map,
            enum_variant_payload_tys: HashMap::new(),
            enum_variant_payload_ast_types: Rc::new(HashMap::new()),
            wide_literal_contexts: HashMap::new(),
            linear_owner_names,
            type_aliases,
            inst_struct_type: HashMap::new(),
            inst_ty_cache: HashMap::new(),
            inst_locations: Vec::new(),
            phi_dependents: HashMap::new(),
            func_return_ast_ty: None,
            file,
            struct_field_type_names,
            struct_field_ast_types,
            string_exprs,
            const_map: HashMap::new(),
            loop_exit_blocks: Vec::new(),
            loop_header_blocks: Vec::new(),
            loop_continue_phis: Vec::new(),
            loop_continue_idx_phi: Vec::new(),
            loop_continue_scope_depth: Vec::new(),
            loop_break_upsilons: Vec::new(),
            loop_exit_phis: Vec::new(),
            inst_vec_elem_types: HashMap::new(),
            inst_vec_option_elem_tys: HashMap::new(),
            inst_vec_variant_payload_tys: HashMap::new(),
            inst_option_elem_ty: HashMap::new(),
            inst_variant_payload_tys: HashMap::new(),
            pattern_aggregates,
            struct_field_vec_elems,
            warnings: Vec::new(),
        }
    }

    pub(super) fn intern_str(&mut self, s: &str) -> u32 {
        if let Some(&idx) = self.string_pool_index.get(s) {
            return idx;
        }
        let idx = self.string_pool.len() as u32;
        self.string_pool_index.insert(s.to_string(), idx);
        self.string_pool.push(s.to_string());
        idx
    }

    pub(super) fn push_scope(&mut self) {
        self.scope.push(HashMap::new());
    }

    pub(super) fn pop_scope(&mut self) {
        self.scope.pop();
    }

    pub(super) fn define(&mut self, name: String, id: InstId) {
        if let Some(top) = self.scope.last_mut() {
            top.insert(name, id);
        }
    }

    /// Update an existing binding in the outermost scope frame that contains it.
    /// If not found, creates a new binding in the current frame.
    pub(super) fn assign(&mut self, name: &str, id: InstId) {
        for frame in self.scope.iter_mut().rev() {
            if frame.contains_key(name) {
                frame.insert(name.to_string(), id);
                return;
            }
        }
        self.define(name.to_string(), id);
    }

    pub(super) fn inst_ty(&self, id: InstId) -> Ty {
        self.inst_ty_cache.get(&id).copied().unwrap_or(Ty::Unit)
    }

    fn merge_inst_ty(&mut self, id: InstId, incoming_ty: Ty) {
        let merged_ty = merge_phi_ty(self.inst_ty(id), incoming_ty);
        if merged_ty == self.inst_ty(id) {
            return;
        }
        let (block_id, inst_index) = self.inst_locations[id.0 as usize];
        self.func.blocks[block_id.0 as usize].insts[inst_index].ty = merged_ty;
        self.inst_ty_cache.insert(id, merged_ty);

        let dependents = self.phi_dependents.get(&id).cloned().unwrap_or_default();
        for dependent in dependents {
            self.merge_inst_ty(dependent, merged_ty);
        }
    }

    fn link_phi_input(&mut self, input: InstId, target: InstId) {
        if input != target {
            self.phi_dependents.entry(input).or_default().push(target);
        }
        self.merge_inst_ty(target, self.inst_ty(input));
    }

    pub(super) fn emit_linear_consume_if_needed(&mut self, id: InstId, span: Span) {
        if self.inst_ty(id) == Ty::LinearPtr {
            self.emit(
                Opcode::LinearConsume,
                Ty::Unit,
                vec![id],
                InstData::None,
                span,
            );
        }
    }

    pub(super) fn lookup(&self, name: &str) -> Option<InstId> {
        for frame in self.scope.iter().rev() {
            if let Some(&id) = frame.get(name) {
                return Some(id);
            }
        }
        None
    }

    /// Look up a variable considering only scope frames up to (exclusive) `depth`.
    /// Used by `continue` to resolve loop-carried vars from the loop header scope,
    /// skipping any inner-scope shadows introduced in the loop body.
    pub(super) fn lookup_at_depth(&self, name: &str, depth: usize) -> Option<InstId> {
        for frame in self.scope[..depth].iter().rev() {
            if let Some(&id) = frame.get(name) {
                return Some(id);
            }
        }
        None
    }

    /// Snapshot the current scope (all variable bindings) for save/restore.
    pub(super) fn snapshot_scope(&self) -> Vec<HashMap<String, InstId>> {
        self.scope.clone()
    }

    /// Restore scope to a previously saved snapshot.
    pub(super) fn restore_scope(&mut self, snap: Vec<HashMap<String, InstId>>) {
        self.scope = snap;
    }

    pub(super) fn new_block(&mut self) -> BlockId {
        let id = BlockId(self.func.blocks.len() as u32);
        self.func.blocks.push(BasicBlock { id, insts: vec![] });
        id
    }

    pub(super) fn switch_to_block(&mut self, block: BlockId) {
        self.current_block = block;
    }

    pub(super) fn alloc_vow(
        &mut self,
        description: String,
        blame: Blame,
        bindings: Vec<(String, InstId)>,
        offset: u32,
    ) -> VowId {
        let id = VowId(self.func.vows.len() as u32);
        self.func.vows.push(VowEntry {
            id,
            description,
            blame,
            bindings,
            file: self.file.clone(),
            offset,
        });
        id
    }

    pub(super) fn emit(
        &mut self,
        opcode: Opcode,
        ty: Ty,
        args: Vec<InstId>,
        data: InstData,
        origin: Span,
    ) -> InstId {
        let phi_link = if opcode == Opcode::Upsilon {
            match (&data, args.first()) {
                (InstData::PhiTarget(target), Some(input)) if *target != InstId(u32::MAX) => {
                    Some((*input, *target))
                }
                _ => None,
            }
        } else {
            None
        };
        let id = InstId(self.next_inst_id);
        self.next_inst_id += 1;
        let inst = Inst {
            id,
            opcode,
            ty,
            args,
            data,
            origin,
            region: RegionId::Root,
        };
        self.inst_ty_cache.insert(id, ty);
        let block_idx = self.current_block.0 as usize;
        let inst_idx = self.func.blocks[block_idx].insts.len();
        self.func.blocks[block_idx].insts.push(inst);
        self.inst_locations.push((self.current_block, inst_idx));
        if let Some((input, target)) = phi_link {
            self.link_phi_input(input, target);
        }
        id
    }

    pub(super) fn is_terminated(&self) -> bool {
        let block_idx = self.current_block.0 as usize;
        self.func.blocks[block_idx]
            .insts
            .last()
            .map(|i| {
                matches!(
                    i.opcode,
                    Opcode::Return | Opcode::Jump | Opcode::Branch | Opcode::Unreachable
                )
            })
            .unwrap_or(false)
    }

    fn warn(&mut self, message: String, span: Span) {
        self.warnings.push(vow_diag::Diagnostic {
            severity: vow_diag::Severity::Warning,
            code: vow_diag::ErrorCode::LoweringWarning,
            message,
            primary: vow_diag::SourceLocation {
                file: self.file.clone(),
                byte_offset: span.start,
                byte_len: span.len,
            },
            secondary: vec![],
            blame: vow_diag::Blame::None,
            hints: vec![],
        });
    }

    pub fn finish(self) -> (Function, Vec<String>, Vec<vow_diag::Diagnostic>) {
        (self.func, self.string_pool, self.warnings)
    }
}

/// Collect names of variables assigned anywhere in a block (recursively).
/// Used to identify loop-carried variables that need Phi nodes.
fn collect_assigned_vars(block: &Block) -> Vec<String> {
    let mut seen = HashSet::new();
    let mut result = vec![];
    for stmt in &block.stmts {
        collect_assigned_in_stmt(stmt, &mut seen, &mut result);
    }
    if let Some(e) = &block.trailing_expr {
        collect_assigned_in_expr(e, &mut seen, &mut result);
    }
    result
}

fn collect_assigned_in_stmt(stmt: &Stmt, seen: &mut HashSet<String>, out: &mut Vec<String>) {
    if let Stmt::Expr { expr, .. } = stmt {
        collect_assigned_in_expr(expr, seen, out);
    }
}

fn collect_assigned_in_expr(expr: &Expr, seen: &mut HashSet<String>, out: &mut Vec<String>) {
    match &expr.kind {
        ExprKind::Assign { lhs, rhs } => {
            if let ExprKind::Ident(name) = &lhs.kind
                && seen.insert(name.clone())
            {
                out.push(name.clone());
            }
            collect_assigned_in_expr(rhs, seen, out);
        }
        ExprKind::Block(b) => {
            for s in &b.stmts {
                collect_assigned_in_stmt(s, seen, out);
            }
            if let Some(e) = &b.trailing_expr {
                collect_assigned_in_expr(e, seen, out);
            }
        }
        ExprKind::If {
            condition,
            then_branch,
            else_branch,
        } => {
            collect_assigned_in_expr(condition, seen, out);
            for s in &then_branch.stmts {
                collect_assigned_in_stmt(s, seen, out);
            }
            if let Some(e) = &then_branch.trailing_expr {
                collect_assigned_in_expr(e, seen, out);
            }
            if let Some(e) = else_branch {
                collect_assigned_in_expr(e, seen, out);
            }
        }
        ExprKind::While {
            condition, body, ..
        } => {
            collect_assigned_in_expr(condition, seen, out);
            for s in &body.stmts {
                collect_assigned_in_stmt(s, seen, out);
            }
            if let Some(e) = &body.trailing_expr {
                collect_assigned_in_expr(e, seen, out);
            }
        }
        ExprKind::Loop { body, .. } => {
            for s in &body.stmts {
                collect_assigned_in_stmt(s, seen, out);
            }
            if let Some(e) = &body.trailing_expr {
                collect_assigned_in_expr(e, seen, out);
            }
        }
        ExprKind::ForEach { body, .. } => {
            for s in &body.stmts {
                collect_assigned_in_stmt(s, seen, out);
            }
            if let Some(e) = &body.trailing_expr {
                collect_assigned_in_expr(e, seen, out);
            }
        }
        ExprKind::BinaryOp { lhs, rhs, .. } => {
            collect_assigned_in_expr(lhs, seen, out);
            collect_assigned_in_expr(rhs, seen, out);
        }
        ExprKind::UnaryOp { operand, .. } => collect_assigned_in_expr(operand, seen, out),
        ExprKind::Return { value: Some(v), .. } => {
            collect_assigned_in_expr(v, seen, out);
        }
        ExprKind::Return { value: None, .. } => {}
        ExprKind::Match { arms, .. } => {
            for arm in arms {
                collect_assigned_in_expr(&arm.body, seen, out);
            }
        }
        _ => {}
    }
}

fn ir_ty_is_integer(ty: Ty) -> bool {
    matches!(
        ty,
        Ty::I8
            | Ty::U8
            | Ty::I16
            | Ty::U16
            | Ty::I32
            | Ty::U32
            | Ty::I64
            | Ty::U64
            | Ty::I128
            | Ty::U128
    )
}

fn expr_is_coercible_int_marker(expr: &Expr) -> bool {
    match &expr.kind {
        ExprKind::Lit(Lit::Int(_)) => true,
        ExprKind::UnaryOp {
            op: UnOp::Neg,
            operand,
        } => expr_is_coercible_int_marker(operand),
        ExprKind::BinaryOp {
            op:
                BinOp::Add
                | BinOp::Sub
                | BinOp::Mul
                | BinOp::Div
                | BinOp::Rem
                | BinOp::AddChecked
                | BinOp::SubChecked
                | BinOp::MulChecked
                | BinOp::DivChecked
                | BinOp::RemChecked
                | BinOp::BitAnd
                | BinOp::BitOr
                | BinOp::BitXor
                | BinOp::Shl
                | BinOp::Shr,
            lhs,
            rhs,
        } => expr_is_coercible_int_marker(lhs) && expr_is_coercible_int_marker(rhs),
        ExprKind::Block(block) => block_result_is_coercible_int_marker(block),
        ExprKind::If {
            then_branch,
            else_branch: Some(else_expr),
            ..
        } => {
            block_result_is_coercible_int_marker(then_branch)
                && expr_is_coercible_int_marker(else_expr)
        }
        _ => false,
    }
}

fn block_result_is_coercible_int_marker(block: &Block) -> bool {
    if let Some(expr) = &block.trailing_expr {
        return expr_is_coercible_int_marker(expr);
    }
    if let Some(Stmt::Expr {
        expr,
        has_semicolon: false,
        ..
    }) = block.stmts.last()
    {
        return expr_is_coercible_int_marker(expr);
    }
    false
}

fn choose_match_result_ty(
    arm_results: &[(BlockId, InstId, Ty, Vec<InstId>)],
    arm_result_markers: &[bool],
) -> Ty {
    if arm_results.iter().any(|(_, _, ty, _)| *ty == Ty::LinearPtr) {
        // Empty generic variants have no payload value from which lowering can
        // infer ownership. A linear sibling carries the checker-resolved
        // wrapper type for the merge, so the Phi must retain that obligation.
        return Ty::LinearPtr;
    }
    let Some((_, _, first_ty, _)) = arm_results.first() else {
        return Ty::I64;
    };
    let mut result_ty = *first_ty;
    let mut result_is_marker = arm_result_markers.first().copied().unwrap_or(false);

    for (i, (_, _, arm_ty, _)) in arm_results.iter().enumerate().skip(1) {
        let arm_is_marker = arm_result_markers.get(i).copied().unwrap_or(false);
        if result_is_marker && ir_ty_is_integer(*arm_ty) {
            result_ty = *arm_ty;
            result_is_marker = arm_is_marker && *arm_ty == Ty::I64;
        } else if !(arm_is_marker && ir_ty_is_integer(result_ty)) {
            result_is_marker = false;
        }
    }

    result_ty
}

fn merge_phi_ty(primary: Ty, sibling: Ty) -> Ty {
    if primary == Ty::LinearPtr || sibling == Ty::LinearPtr {
        Ty::LinearPtr
    } else {
        primary
    }
}

/// Return variables that are assigned in `then_branch` or `else_branch` AND
/// currently exist in scope (so they're live across the branch).
fn collect_if_mutations(
    ctx: &LowerCtx,
    then_branch: &Block,
    else_branch: Option<&Expr>,
) -> Vec<(String, InstId)> {
    let mut seen = HashSet::new();
    let mut names = vec![];
    for s in &then_branch.stmts {
        collect_assigned_in_stmt(s, &mut seen, &mut names);
    }
    if let Some(e) = &then_branch.trailing_expr {
        collect_assigned_in_expr(e, &mut seen, &mut names);
    }
    if let Some(e) = else_branch {
        collect_assigned_in_expr(e, &mut seen, &mut names);
    }
    names
        .into_iter()
        .filter_map(|name| ctx.lookup(&name).map(|id| (name, id)))
        .collect()
}

pub(super) fn lower_expr_pub(ctx: &mut LowerCtx, expr: &vow_syntax::ast::Expr) -> InstId {
    lower_expr(ctx, expr)
}

fn lower_expr(ctx: &mut LowerCtx, expr: &vow_syntax::ast::Expr) -> InstId {
    let span = expr.span;
    match &expr.kind {
        ExprKind::Lit(lit) => match lit {
            Lit::Int(v) => match ctx
                .wide_literal_contexts
                .get(&(expr as *const _ as usize))
                .copied()
            {
                Some(ty @ (Ty::I128 | Ty::U128)) => emit_narrow_integer_constant(ctx, *v, ty, span),
                _ => ctx.emit(
                    Opcode::ConstI64,
                    Ty::I64,
                    vec![],
                    InstData::ConstI64(*v as i64),
                    span,
                ),
            },
            Lit::Float(v) => ctx.emit(
                Opcode::ConstF64,
                Ty::F64,
                vec![],
                InstData::ConstF64(*v),
                span,
            ),
            Lit::Bool(v) => ctx.emit(
                Opcode::ConstBool,
                Ty::Bool,
                vec![],
                InstData::ConstBool(*v),
                span,
            ),
            Lit::String(s) => {
                let idx = ctx.intern_str(s);
                let cstr = ctx.emit(
                    Opcode::ConstStr,
                    Ty::Ptr,
                    vec![],
                    InstData::ConstStr(idx),
                    span,
                );
                let vow_str = ctx.emit(
                    Opcode::Call,
                    Ty::Ptr,
                    vec![cstr],
                    InstData::CallExtern("__vow_string_literal".to_string()),
                    span,
                );
                ctx.inst_struct_type.insert(vow_str, "String".to_string());
                vow_str
            }
        },
        ExprKind::Ident(name) => {
            if let Some(&(val, ref ty)) = ctx.const_map.get(name.as_str()) {
                let (opcode, data) = if *ty == Ty::Bool {
                    (Opcode::ConstBool, InstData::ConstBool(val != 0))
                } else {
                    (Opcode::ConstI64, InstData::ConstI64(val))
                };
                return ctx.emit(opcode, *ty, vec![], data, span);
            }
            ctx.lookup(name)
                .unwrap_or_else(|| panic!("undefined variable: {name}"))
        }
        ExprKind::BinaryOp { op, lhs, rhs } => {
            // Short-circuit evaluation for && and ||
            if *op == BinOp::And || *op == BinOp::Or {
                let lhs_id = lower_expr(ctx, lhs);
                let rhs_block = ctx.new_block();
                let short_block = ctx.new_block();
                let merge_block = ctx.new_block();

                // For &&: if LHS false → short-circuit (false); else → evaluate RHS
                // For ||: if LHS true → short-circuit (true); else → evaluate RHS
                let (then_target, else_target) = if *op == BinOp::And {
                    (rhs_block, short_block)
                } else {
                    (short_block, rhs_block)
                };
                ctx.emit(
                    Opcode::Branch,
                    Ty::Unit,
                    vec![lhs_id],
                    InstData::BranchTargets {
                        then_block: then_target,
                        else_block: else_target,
                    },
                    span,
                );

                // RHS block: evaluate RHS and feed it into the merge Phi.
                ctx.switch_to_block(rhs_block);
                let rhs_id = lower_expr(ctx, rhs);
                let rhs_upsilon = ctx.emit(
                    Opcode::Upsilon,
                    Ty::Unit,
                    vec![rhs_id],
                    InstData::PhiTarget(InstId(u32::MAX)),
                    span,
                );
                let rhs_upsilon_block = ctx.current_block;
                ctx.emit(
                    Opcode::Jump,
                    Ty::Unit,
                    vec![],
                    InstData::JumpTarget(merge_block),
                    span,
                );

                // Short-circuit block: produce constant false (&&) or true (||)
                ctx.switch_to_block(short_block);
                let short_val = ctx.emit(
                    Opcode::ConstBool,
                    Ty::Bool,
                    vec![],
                    InstData::ConstBool(*op == BinOp::Or),
                    span,
                );
                let short_upsilon = ctx.emit(
                    Opcode::Upsilon,
                    Ty::Unit,
                    vec![short_val],
                    InstData::PhiTarget(InstId(u32::MAX)),
                    span,
                );
                let short_upsilon_block = ctx.current_block;
                ctx.emit(
                    Opcode::Jump,
                    Ty::Unit,
                    vec![],
                    InstData::JumpTarget(merge_block),
                    span,
                );

                // Merge block: Phi collects the result
                ctx.switch_to_block(merge_block);
                let phi = ctx.emit(Opcode::Phi, Ty::Bool, vec![], InstData::None, span);
                backpatch_upsilon(ctx, rhs_upsilon_block, rhs_upsilon, phi);
                backpatch_upsilon(ctx, short_upsilon_block, short_upsilon, phi);

                return phi;
            }

            if expr_is_integer_literal(lhs)
                && let Some(context_ty) = known_wide_expr_ty(ctx, rhs)
            {
                record_wide_control_flow_context(ctx, lhs, context_ty);
            }
            if expr_is_integer_literal(rhs)
                && let Some(context_ty) = known_wide_expr_ty(ctx, lhs)
            {
                record_wide_control_flow_context(ctx, rhs, context_ty);
            }
            let mut lhs_id = lower_expr(ctx, lhs);
            let mut rhs_id = lower_expr(ctx, rhs);
            let lhs_is_str = ctx
                .string_exprs
                .contains(&(lhs.as_ref() as *const Expr as usize));
            let rhs_is_str = ctx
                .string_exprs
                .contains(&(rhs.as_ref() as *const Expr as usize));
            if (lhs_is_str || rhs_is_str) && (*op == BinOp::Eq || *op == BinOp::Ne) {
                let eq_result = ctx.emit(
                    Opcode::Call,
                    Ty::Bool,
                    vec![lhs_id, rhs_id],
                    InstData::CallExtern("__vow_string_eq".to_string()),
                    span,
                );
                if *op == BinOp::Ne {
                    ctx.emit(Opcode::Not, Ty::Bool, vec![eq_result], InstData::None, span)
                } else {
                    eq_result
                }
            } else {
                let lhs_ty = ctx.inst_ty(lhs_id);
                let rhs_ty = ctx.inst_ty(rhs_id);
                let is_bitwise = matches!(
                    op,
                    BinOp::BitAnd | BinOp::BitOr | BinOp::BitXor | BinOp::Shl | BinOp::Shr
                );
                let contextual_narrow_literal_ty = |narrow_ty: Ty| {
                    (expr_is_integer_literal(lhs) && rhs_ty == narrow_ty)
                        || (expr_is_integer_literal(rhs) && lhs_ty == narrow_ty)
                };
                let operand_ty = if contextual_narrow_literal_ty(Ty::I8) {
                    Ty::I8
                } else if contextual_narrow_literal_ty(Ty::U8) {
                    Ty::U8
                } else if contextual_narrow_literal_ty(Ty::I16) {
                    Ty::I16
                } else if contextual_narrow_literal_ty(Ty::U16) {
                    Ty::U16
                } else if contextual_narrow_literal_ty(Ty::I32) {
                    Ty::I32
                } else if contextual_narrow_literal_ty(Ty::U32) {
                    Ty::U32
                } else if contextual_narrow_literal_ty(Ty::I128) {
                    Ty::I128
                } else if contextual_narrow_literal_ty(Ty::U128) {
                    Ty::U128
                } else if is_bitwise && lhs_ty == Ty::I64 {
                    if rhs_ty != Ty::I64 { rhs_ty } else { lhs_ty }
                } else {
                    lhs_ty
                };
                if matches!(
                    operand_ty,
                    Ty::I8 | Ty::U8 | Ty::I16 | Ty::U16 | Ty::I32 | Ty::U32 | Ty::I128 | Ty::U128
                ) {
                    lhs_id = lower_narrow_literal(ctx, lhs, lhs_id, operand_ty);
                    if !matches!(op, BinOp::Shl | BinOp::Shr) {
                        rhs_id = lower_narrow_literal(ctx, rhs, rhs_id, operand_ty);
                    }
                }
                let (opcode, ty, data) = binop_opcode(*op, &operand_ty);
                ctx.emit(opcode, ty, vec![lhs_id, rhs_id], data, span)
            }
        }
        ExprKind::UnaryOp { op, operand } => {
            let val = lower_expr(ctx, operand);
            match op {
                UnOp::Not => ctx.emit(Opcode::Not, Ty::Bool, vec![val], InstData::None, span),
                UnOp::Neg => {
                    let operand_ty = ctx.inst_ty(val);
                    let result_ty = if matches!(
                        operand_ty,
                        Ty::I8
                            | Ty::U8
                            | Ty::I16
                            | Ty::U16
                            | Ty::I32
                            | Ty::U32
                            | Ty::I64
                            | Ty::U64
                            | Ty::I128
                            | Ty::U128
                    ) {
                        operand_ty
                    } else {
                        Ty::I64
                    };
                    let zero = emit_integer_zero(ctx, result_ty, span);
                    ctx.emit(
                        Opcode::WrappingSub,
                        result_ty,
                        vec![zero, val],
                        InstData::Integer(integer_type_for_ir_ty(result_ty)),
                        span,
                    )
                }
            }
        }
        ExprKind::Call { callee, args } => {
            let callee_name = match &callee.kind {
                ExprKind::Ident(name) => name.clone(),
                _ => todo!("non-ident callee in Call lowering"),
            };
            let call_info = ctx.func_index.get(&callee_name).cloned();
            if callee_name == "string_matches_literal_at" {
                let string_id = args
                    .first()
                    .map(|a| lower_consumed_expr(ctx, a))
                    .unwrap_or_else(|| {
                        ctx.emit(Opcode::ConstUnit, Ty::Unit, vec![], InstData::None, span)
                    });
                let pos_id = args
                    .get(1)
                    .map(|a| lower_consumed_expr(ctx, a))
                    .unwrap_or_else(|| {
                        ctx.emit(
                            Opcode::ConstI64,
                            Ty::I64,
                            vec![],
                            InstData::ConstI64(0),
                            span,
                        )
                    });
                if let Some((literal_ptr, literal_len)) = args
                    .get(2)
                    .and_then(|arg| lower_static_string_literal(ctx, arg))
                {
                    return ctx.emit(
                        Opcode::Call,
                        Ty::I64,
                        vec![string_id, pos_id, literal_ptr, literal_len],
                        InstData::CallExtern("__vow_string_matches_literal_at".to_string()),
                        span,
                    );
                }
                return ctx.emit(Opcode::ConstUnit, Ty::Unit, vec![], InstData::None, span);
            }
            let arg_ids: Vec<InstId> = args
                .iter()
                .enumerate()
                .map(|(i, a)| {
                    if let Some(expected) = call_info
                        .as_ref()
                        .and_then(|info| info.param_ast_tys.get(i))
                    {
                        record_wide_expected_ast_context(ctx, a, expected);
                    }
                    if let Some(info) = &call_info
                        && let Some(&param_ty) = info.param_tys.get(i)
                        && matches!(
                            param_ty,
                            Ty::I8
                                | Ty::U8
                                | Ty::I16
                                | Ty::U16
                                | Ty::I32
                                | Ty::U32
                                | Ty::I128
                                | Ty::U128
                        )
                    {
                        record_wide_control_flow_context(ctx, a, param_ty);
                        let original = lower_consumed_expr(ctx, a);
                        lower_narrow_literal(ctx, a, original, param_ty)
                    } else {
                        lower_consumed_expr(ctx, a)
                    }
                })
                .collect();
            if callee_name == "pin_to_root" {
                let Some(source_id) = arg_ids.first().copied() else {
                    return ctx.emit(Opcode::ConstUnit, Ty::Unit, vec![], InstData::None, span);
                };
                if ctx
                    .inst_struct_type
                    .get(&source_id)
                    .is_some_and(|tag| tag == "String")
                {
                    let result = ctx.emit(
                        Opcode::Call,
                        Ty::Ptr,
                        vec![source_id],
                        InstData::CallExtern("__vow_string_pin_to_root".to_string()),
                        span,
                    );
                    ctx.inst_struct_type.insert(result, "String".to_string());
                    return result;
                }
                if ctx
                    .inst_struct_type
                    .get(&source_id)
                    .is_some_and(|tag| tag == "Vec")
                {
                    let result = ctx.emit(
                        Opcode::Call,
                        Ty::Ptr,
                        vec![source_id],
                        InstData::CallExtern("__vow_vec_pin_to_root_val".to_string()),
                        span,
                    );
                    ctx.inst_struct_type.insert(result, "Vec".to_string());
                    if let Some(elem_types) = ctx.inst_vec_elem_types.get(&source_id).cloned() {
                        ctx.inst_vec_elem_types.insert(result, elem_types);
                    }
                    if let Some(option_types) =
                        ctx.inst_vec_option_elem_tys.get(&source_id).cloned()
                    {
                        ctx.inst_vec_option_elem_tys.insert(result, option_types);
                    }
                    return result;
                }
                // pin_to_root relies on lowering-time String/Vec tags. Keep
                // tag_builtin_result in sync for heap-returning builtins, or a
                // direct pin_to_root(builtin_call()) becomes a no-op here.
                return source_id;
            }
            if let Some(call_info) = call_info {
                let result = ctx.emit(
                    Opcode::Call,
                    call_info.ret_ty,
                    arg_ids,
                    InstData::CallTarget(call_info.id),
                    span,
                );
                if let Some(ret_tag) = call_info.ret_tag {
                    ctx.inst_struct_type.insert(result, ret_tag);
                }
                if let Some(ret_vec_elem) = call_info.ret_vec_elem {
                    ctx.inst_vec_elem_types.insert(result, vec![ret_vec_elem]);
                }
                if let Some(ret_option_elem) = call_info.ret_option_elem {
                    ctx.inst_option_elem_ty.insert(result, ret_option_elem);
                }
                result
            } else if let Some((sym, ret_ty)) = vow_debug_builtin_to_runtime(&callee_name) {
                ctx.emit(
                    Opcode::DebugCall,
                    ret_ty,
                    arg_ids,
                    InstData::CallExtern(sym.to_string()),
                    span,
                )
            } else if let Some((sym, ret_ty)) = vow_builtin_to_runtime(&callee_name) {
                let result = ctx.emit(
                    Opcode::Call,
                    ret_ty,
                    arg_ids,
                    InstData::CallExtern(sym.to_string()),
                    span,
                );
                tag_builtin_result(ctx, &callee_name, result);
                result
            } else {
                ctx.emit(
                    Opcode::Call,
                    Ty::Unit,
                    arg_ids,
                    InstData::CallExtern(callee_name),
                    span,
                )
            }
        }
        ExprKind::If {
            condition,
            then_branch,
            else_branch,
        } => {
            // Collect variables that may be mutated in any branch AND exist in outer scope.
            let mutations: Vec<(String, InstId)> =
                collect_if_mutations(ctx, then_branch, else_branch.as_deref());

            let cond_id = lower_expr(ctx, condition);
            let then_block = ctx.new_block();
            let else_block = ctx.new_block();
            let merge_block = ctx.new_block();

            ctx.emit(
                Opcode::Branch,
                Ty::Unit,
                vec![cond_id],
                InstData::BranchTargets {
                    then_block,
                    else_block,
                },
                span,
            );

            // Snapshot scope so then-branch mutations don't bleed into else-branch.
            let scope_snap = ctx.snapshot_scope();

            // Lower then-branch.
            ctx.switch_to_block(then_block);
            let then_val = lower_block(ctx, then_branch);
            let then_terminated = ctx.is_terminated();
            let then_upsilon_block = ctx.current_block;
            // Capture mutation values from then-branch (or pre-if value if not modified).
            let then_mut_vals: Vec<InstId> = mutations
                .iter()
                .map(|(name, pre_id)| ctx.lookup(name).unwrap_or(*pre_id))
                .collect();
            let then_upsilon_id = if !then_terminated {
                let u = ctx.emit(
                    Opcode::Upsilon,
                    Ty::Unit,
                    vec![then_val],
                    InstData::PhiTarget(InstId(u32::MAX)),
                    span,
                );
                ctx.emit(
                    Opcode::Jump,
                    Ty::Unit,
                    vec![],
                    InstData::JumpTarget(merge_block),
                    span,
                );
                Some(u)
            } else {
                None
            };

            // Restore scope so else-branch starts from the pre-if state.
            ctx.restore_scope(scope_snap.clone());

            // Lower else-branch.
            ctx.switch_to_block(else_block);
            let else_val = if let Some(else_expr) = else_branch {
                lower_expr(ctx, else_expr)
            } else {
                ctx.emit(Opcode::ConstUnit, Ty::Unit, vec![], InstData::None, span)
            };
            let else_terminated = ctx.is_terminated();
            let else_upsilon_block = ctx.current_block;
            let else_mut_vals: Vec<InstId> = mutations
                .iter()
                .map(|(name, pre_id)| ctx.lookup(name).unwrap_or(*pre_id))
                .collect();
            let else_upsilon_id = if !else_terminated {
                let u = ctx.emit(
                    Opcode::Upsilon,
                    Ty::Unit,
                    vec![else_val],
                    InstData::PhiTarget(InstId(u32::MAX)),
                    span,
                );
                ctx.emit(
                    Opcode::Jump,
                    Ty::Unit,
                    vec![],
                    InstData::JumpTarget(merge_block),
                    span,
                );
                Some(u)
            } else {
                None
            };

            // Restore scope before building merge.
            ctx.restore_scope(scope_snap);

            ctx.switch_to_block(merge_block);

            // Create Phis for each mutated variable, wiring Upsilons from both branches.
            // Upsilons are appended even after the Jump (they are no-ops in codegen but
            // are found by collect_target_block_args which scans all instructions).
            for (i, (name, pre_id)) in mutations.iter().enumerate() {
                let t_val = then_mut_vals[i];
                let e_val = else_mut_vals[i];
                if t_val == *pre_id && e_val == *pre_id {
                    // Variable unchanged by both branches — no phi needed.
                    continue;
                }
                let phi_ty = merge_phi_ty(ctx.inst_ty(t_val), ctx.inst_ty(e_val));
                let phi_id = ctx.emit(Opcode::Phi, phi_ty, vec![], InstData::None, span);
                if !then_terminated {
                    ctx.switch_to_block(then_upsilon_block);
                    ctx.emit(
                        Opcode::Upsilon,
                        phi_ty,
                        vec![t_val],
                        InstData::PhiTarget(phi_id),
                        span,
                    );
                    ctx.switch_to_block(merge_block);
                }
                if !else_terminated {
                    ctx.switch_to_block(else_upsilon_block);
                    ctx.emit(
                        Opcode::Upsilon,
                        phi_ty,
                        vec![e_val],
                        InstData::PhiTarget(phi_id),
                        span,
                    );
                    ctx.switch_to_block(merge_block);
                }
                ctx.assign(name, phi_id);
            }

            match (then_upsilon_id, else_upsilon_id) {
                (None, None) => {
                    // Both branches terminate — merge block is unreachable.
                    ctx.emit(Opcode::Unreachable, Ty::Unit, vec![], InstData::None, span)
                }
                (Some(t_up), None) => {
                    let phi_ty = ctx.inst_ty(then_val);
                    let phi_id = ctx.emit(Opcode::Phi, phi_ty, vec![], InstData::None, span);
                    backpatch_upsilon(ctx, then_upsilon_block, t_up, phi_id);
                    phi_id
                }
                (None, Some(e_up)) => {
                    let phi_ty = ctx.inst_ty(else_val);
                    let phi_id = ctx.emit(Opcode::Phi, phi_ty, vec![], InstData::None, span);
                    backpatch_upsilon(ctx, else_upsilon_block, e_up, phi_id);
                    phi_id
                }
                (Some(t_up), Some(e_up)) => {
                    let phi_ty = merge_phi_ty(ctx.inst_ty(then_val), ctx.inst_ty(else_val));
                    let phi_id = ctx.emit(Opcode::Phi, phi_ty, vec![], InstData::None, span);
                    backpatch_upsilon(ctx, then_upsilon_block, t_up, phi_id);
                    backpatch_upsilon(ctx, else_upsilon_block, e_up, phi_id);
                    phi_id
                }
            }
        }
        ExprKind::Block(block) => {
            ctx.push_scope();
            let result = lower_block_inner(ctx, block);
            ctx.pop_scope();
            result
        }
        ExprKind::Return { value } => {
            if let Some(val_expr) = value {
                if let Some(expected) = ctx.func_return_ast_ty.clone() {
                    record_wide_expected_ast_context(ctx, val_expr, &expected);
                }
                record_wide_control_flow_context(ctx, val_expr, ctx.func.return_ty);
                let original = lower_expr(ctx, val_expr);
                let val = lower_narrow_literal(ctx, val_expr, original, ctx.func.return_ty);
                if let Some(vow_block) = ctx.vow_block.clone() {
                    vow::lower_ensures(ctx, &vow_block, val);
                }
                ctx.emit(Opcode::Return, Ty::Unit, vec![val], InstData::None, span)
            } else {
                let unit = ctx.emit(Opcode::ConstUnit, Ty::Unit, vec![], InstData::None, span);
                if let Some(vow_block) = ctx.vow_block.clone() {
                    vow::lower_ensures(ctx, &vow_block, unit);
                }
                ctx.emit(Opcode::Return, Ty::Unit, vec![unit], InstData::None, span)
            }
        }
        ExprKind::Assign { lhs, rhs } => {
            let index_ty = known_index_assignment_ty(ctx, lhs);
            if let Some(field_ty) = known_field_assignment_ty(ctx, lhs) {
                record_wide_control_flow_context(ctx, rhs, field_ty);
            }
            if let Some(index_ty) = index_ty {
                record_wide_control_flow_context(ctx, rhs, index_ty);
            }
            if let ExprKind::Ident(name) = &lhs.kind
                && let Some(current) = ctx.lookup(name)
            {
                record_wide_control_flow_context(ctx, rhs, ctx.inst_ty(current));
            }
            let mut new_val = lower_expr(ctx, rhs);
            match &lhs.kind {
                ExprKind::Ident(name) => {
                    if let Some(current) = ctx.lookup(name) {
                        let current_ty = ctx.inst_ty(current);
                        if matches!(
                            current_ty,
                            Ty::I8
                                | Ty::U8
                                | Ty::I16
                                | Ty::U16
                                | Ty::I32
                                | Ty::U32
                                | Ty::I128
                                | Ty::U128
                        ) {
                            new_val = lower_narrow_literal(ctx, rhs, new_val, current_ty);
                        }
                    }
                    ctx.assign(name, new_val);
                }
                ExprKind::FieldAccess { base, field } => {
                    let ptr_id = lower_expr(ctx, base);
                    let struct_name = ctx
                        .inst_struct_type
                        .get(&ptr_id)
                        .cloned()
                        .unwrap_or_default();
                    if struct_name.is_empty() {
                        ctx.warn(
                            format!("FieldSet on untagged instruction %{}, field '{}' -- ICE: returning sentinel index", ptr_id.0, field),
                            span,
                        );
                    }
                    let field_idx = if let Some(names) = ctx.struct_field_map.get(&struct_name) {
                        match names.iter().position(|n| n == field) {
                            Some(idx) => idx,
                            None => {
                                if !struct_name.is_empty() {
                                    ctx.warn(
                                        format!("field '{}' not found in struct '{}' -- ICE: returning sentinel index", field, struct_name),
                                        span,
                                    );
                                }
                                FIELD_IDX_SENTINEL
                            }
                        }
                    } else {
                        if !struct_name.is_empty() {
                            ctx.warn(
                                format!("struct '{}' not registered -- field lookup ICE: returning sentinel index", struct_name),
                                span,
                            );
                        }
                        FIELD_IDX_SENTINEL
                    } as u32;
                    if field_idx != FIELD_IDX_SENTINEL as u32 {
                        let field_ty = ctx
                            .struct_field_type_names
                            .get(&struct_name)
                            .and_then(|types| types.get(field_idx as usize))
                            .map(|type_name| scalar_ty_for_field_type_name(type_name));
                        if let Some(field_ty @ (Ty::I128 | Ty::U128)) = field_ty {
                            new_val = lower_narrow_literal(ctx, rhs, new_val, field_ty);
                        }
                        // Field store transfers a linear RHS into the heap container.
                        ctx.emit_linear_consume_if_needed(new_val, span);
                        ctx.emit(
                            Opcode::FieldSet,
                            Ty::Unit,
                            vec![ptr_id, new_val],
                            InstData::FieldIndex(field_idx),
                            span,
                        );
                    }
                }
                ExprKind::Index { base, index } => {
                    let vec_ptr = lower_expr(ctx, base);
                    let idx_id = lower_expr(ctx, index);
                    if let Some(index_ty) = index_ty {
                        new_val = lower_narrow_literal(ctx, rhs, new_val, index_ty);
                    }
                    // Index store transfers a linear RHS into the heap container.
                    ctx.emit_linear_consume_if_needed(new_val, span);
                    ctx.emit(
                        Opcode::Call,
                        Ty::Unit,
                        vec![vec_ptr, idx_id, new_val],
                        InstData::CallExtern("__vow_vec_set_val".to_string()),
                        span,
                    );
                }
                _ => {}
            }
            new_val
        }
        ExprKind::While {
            condition,
            body,
            vow: while_vow,
        } => {
            let mutated = collect_assigned_vars(body);

            // Gather pre-loop (name, current_value) for mutated vars that exist in scope.
            let loop_vars: Vec<(String, InstId)> = mutated
                .into_iter()
                .filter_map(|name| ctx.lookup(&name).map(|id| (name, id)))
                .collect();

            let pre_header_block = ctx.current_block;
            let header_block = ctx.new_block();
            let body_block = ctx.new_block();
            let exit_block = ctx.new_block();

            // Emit placeholder Upsilons for each loop var, then jump to header.
            let mut upsilon_ids: Vec<(String, InstId)> = vec![];
            for (name, pre_val) in &loop_vars {
                let ty = ctx.inst_ty(*pre_val);
                let up_id = ctx.emit(
                    Opcode::Upsilon,
                    ty,
                    vec![*pre_val],
                    InstData::PhiTarget(InstId(u32::MAX)),
                    span,
                );
                upsilon_ids.push((name.clone(), up_id));
            }
            ctx.emit(
                Opcode::Jump,
                Ty::Unit,
                vec![],
                InstData::JumpTarget(header_block),
                span,
            );

            // Header: emit Phis, then backpatch the pre-header Upsilons.
            ctx.switch_to_block(header_block);
            let mut phi_ids: Vec<(String, InstId)> = vec![];
            for (name, pre_val) in &loop_vars {
                let ty = ctx.inst_ty(*pre_val);
                let phi_id = ctx.emit(Opcode::Phi, ty, vec![], InstData::None, span);
                phi_ids.push((name.clone(), phi_id));
            }
            for (name, up_id) in &upsilon_ids {
                let phi_id = phi_ids.iter().find(|(n, _)| n == name).unwrap().1;
                backpatch_upsilon(ctx, pre_header_block, *up_id, phi_id);
            }

            // Update scope: rebind each loop var to its Phi.
            for (name, phi_id) in &phi_ids {
                ctx.assign(name, *phi_id);
            }

            // Lower vow invariant at top of header (before condition).
            if let Some(wv) = while_vow {
                vow::lower_invariant(ctx, wv);
            }

            // Lower condition, then branch.
            let cond_id = lower_expr(ctx, condition);
            // Save the block we're in after condition lowering (may differ from
            // header_block if the condition created new blocks, e.g. &&/||).
            let cond_block = ctx.current_block;

            // Pre-emit exit-block Phis for mutation variables so break sites
            // (and the natural condition-false exit) can supply updated values.
            let mut exit_phi_ids: Vec<(String, InstId)> = vec![];
            ctx.switch_to_block(exit_block);
            for (name, pre_val) in &loop_vars {
                let ty = ctx.inst_ty(*pre_val);
                let phi_id = ctx.emit(Opcode::Phi, ty, vec![], InstData::None, span);
                exit_phi_ids.push((name.clone(), phi_id));
            }
            ctx.switch_to_block(cond_block);

            // Upsilons for natural exit (condition false → exit_block):
            // pass header Phi values into exit-block Phis.
            for (name, exit_phi) in &exit_phi_ids {
                let header_phi = phi_ids.iter().find(|(n, _)| n == name).unwrap().1;
                ctx.emit(
                    Opcode::Upsilon,
                    ctx.inst_ty(header_phi),
                    vec![header_phi],
                    InstData::PhiTarget(*exit_phi),
                    span,
                );
            }

            ctx.emit(
                Opcode::Branch,
                Ty::Unit,
                vec![cond_id],
                InstData::BranchTargets {
                    then_block: body_block,
                    else_block: exit_block,
                },
                span,
            );

            // Body: lower body (push/pop scope handles lets inside body).
            ctx.switch_to_block(body_block);
            ctx.loop_exit_blocks.push(exit_block);
            ctx.loop_header_blocks.push(header_block);
            ctx.loop_continue_phis.push(phi_ids.clone());
            ctx.loop_exit_phis.push(exit_phi_ids.clone());
            ctx.loop_continue_idx_phi.push(None);
            ctx.loop_continue_scope_depth.push(ctx.scope.len());
            ctx.loop_break_upsilons.push(None);
            lower_block(ctx, body);
            ctx.loop_break_upsilons.pop();
            ctx.loop_continue_scope_depth.pop();
            ctx.loop_continue_idx_phi.pop();
            ctx.loop_exit_phis.pop();
            ctx.loop_continue_phis.pop();
            ctx.loop_header_blocks.pop();
            ctx.loop_exit_blocks.pop();

            // Emit back-edge Upsilons with the current scope values.
            if !ctx.is_terminated() {
                for (name, phi_id) in &phi_ids {
                    if let Some(cur_val) = ctx.lookup(name) {
                        ctx.emit(
                            Opcode::Upsilon,
                            ctx.inst_ty(cur_val),
                            vec![cur_val],
                            InstData::PhiTarget(*phi_id),
                            span,
                        );
                    }
                }
                ctx.emit(
                    Opcode::Jump,
                    Ty::Unit,
                    vec![],
                    InstData::JumpTarget(header_block),
                    span,
                );
            }

            // Bind names to exit-block Phis so post-loop code reads correct values.
            for (name, exit_phi) in &exit_phi_ids {
                ctx.assign(name, *exit_phi);
            }

            // Exit block (Phis already emitted above).
            ctx.switch_to_block(exit_block);
            ctx.emit(Opcode::ConstUnit, Ty::Unit, vec![], InstData::None, span)
        }
        ExprKind::ForEach {
            binding,
            iterable,
            body,
            vow: for_vow,
        } => {
            // Desugar: for <binding> in <iterable> { <body> }
            // into:    let iter = <iterable>; let len = iter.len(); let idx = 0;
            //          while idx < len { let <binding> = iter[idx]; <body>; idx = idx + 1; }

            let iter_id = lower_expr(ctx, iterable);
            ctx.inst_struct_type.insert(iter_id, "Vec".to_string());

            let len_id = ctx.emit(
                Opcode::Call,
                Ty::I64,
                vec![iter_id],
                InstData::CallExtern("__vow_vec_len".to_string()),
                span,
            );
            let idx_init = ctx.emit(
                Opcode::ConstI64,
                Ty::I64,
                vec![],
                InstData::ConstI64(0),
                span,
            );

            let mutated = collect_assigned_vars(body);
            let loop_vars: Vec<(String, InstId)> = mutated
                .into_iter()
                .filter_map(|name| ctx.lookup(&name).map(|id| (name, id)))
                .collect();

            let pre_header_block = ctx.current_block;
            let header_block = ctx.new_block();
            let body_block = ctx.new_block();
            let exit_block = ctx.new_block();

            // Pre-header: Upsilon for index
            let idx_up = ctx.emit(
                Opcode::Upsilon,
                Ty::I64,
                vec![idx_init],
                InstData::PhiTarget(InstId(u32::MAX)),
                span,
            );

            // Pre-header: Upsilons for user mutated vars
            let mut upsilon_ids: Vec<(String, InstId)> = vec![];
            for (name, pre_val) in &loop_vars {
                let ty = ctx.inst_ty(*pre_val);
                let up_id = ctx.emit(
                    Opcode::Upsilon,
                    ty,
                    vec![*pre_val],
                    InstData::PhiTarget(InstId(u32::MAX)),
                    span,
                );
                upsilon_ids.push((name.clone(), up_id));
            }
            ctx.emit(
                Opcode::Jump,
                Ty::Unit,
                vec![],
                InstData::JumpTarget(header_block),
                span,
            );

            // Header: Phi for index
            ctx.switch_to_block(header_block);
            let idx_phi = ctx.emit(Opcode::Phi, Ty::I64, vec![], InstData::None, span);
            backpatch_upsilon(ctx, pre_header_block, idx_up, idx_phi);

            // Header: Phi for user mutated vars
            let mut phi_ids: Vec<(String, InstId)> = vec![];
            for (name, pre_val) in &loop_vars {
                let ty = ctx.inst_ty(*pre_val);
                let phi_id = ctx.emit(Opcode::Phi, ty, vec![], InstData::None, span);
                phi_ids.push((name.clone(), phi_id));
            }
            for (name, up_id) in &upsilon_ids {
                let phi_id = phi_ids.iter().find(|(n, _)| n == name).unwrap().1;
                backpatch_upsilon(ctx, pre_header_block, *up_id, phi_id);
            }

            // Update scope: rebind mutated vars to their Phis
            for (name, phi_id) in &phi_ids {
                ctx.assign(name, *phi_id);
            }

            // Lower vow invariant at top of header (before condition)
            if let Some(wv) = for_vow {
                vow::lower_invariant(ctx, wv);
            }

            // Condition: idx < len
            let cond_id = ctx.emit(
                Opcode::Lt,
                Ty::Bool,
                vec![idx_phi, len_id],
                InstData::Integer(IntegerType::I64),
                span,
            );

            // Pre-emit exit-block Phis for mutation variables so break sites
            // (and the natural condition-false exit) can supply updated values.
            let mut exit_phi_ids: Vec<(String, InstId)> = vec![];
            ctx.switch_to_block(exit_block);
            for (name, pre_val) in &loop_vars {
                let ty = ctx.inst_ty(*pre_val);
                let phi_id = ctx.emit(Opcode::Phi, ty, vec![], InstData::None, span);
                exit_phi_ids.push((name.clone(), phi_id));
            }
            ctx.switch_to_block(header_block);

            // Upsilons for natural exit (condition false → exit_block):
            // pass header Phi values into exit-block Phis.
            for (name, exit_phi) in &exit_phi_ids {
                let header_phi = phi_ids.iter().find(|(n, _)| n == name).unwrap().1;
                ctx.emit(
                    Opcode::Upsilon,
                    ctx.inst_ty(header_phi),
                    vec![header_phi],
                    InstData::PhiTarget(*exit_phi),
                    span,
                );
            }

            ctx.emit(
                Opcode::Branch,
                Ty::Unit,
                vec![cond_id],
                InstData::BranchTargets {
                    then_block: body_block,
                    else_block: exit_block,
                },
                span,
            );

            // Body: get element and bind to loop variable
            ctx.switch_to_block(body_block);
            let elem_id = ctx.emit(
                Opcode::Call,
                Ty::I64,
                vec![iter_id, idx_phi],
                InstData::CallExtern("__vow_vec_get_val".to_string()),
                span,
            );
            propagate_vec_element_metadata(ctx, iter_id, elem_id);

            // Save scope depth before pushing the for-each binding scope.
            // Loop-carried phis track outer mutation variables whose bindings
            // live at this depth; the for-each binding is a new scope that must
            // be excluded from continue's lookup to avoid resolving to it.
            let for_scope_depth = ctx.scope.len();

            ctx.push_scope();
            ctx.define(binding.clone(), elem_id);

            ctx.loop_exit_blocks.push(exit_block);
            ctx.loop_header_blocks.push(header_block);
            ctx.loop_continue_phis.push(phi_ids.clone());
            ctx.loop_exit_phis.push(exit_phi_ids.clone());
            ctx.loop_continue_idx_phi.push(Some(idx_phi));
            ctx.loop_continue_scope_depth.push(for_scope_depth);
            lower_block(ctx, body);
            ctx.loop_continue_scope_depth.pop();
            ctx.loop_continue_idx_phi.pop();
            ctx.loop_exit_phis.pop();
            ctx.loop_continue_phis.pop();
            ctx.loop_header_blocks.pop();
            ctx.loop_exit_blocks.pop();

            ctx.pop_scope();

            // Increment index and emit back-edge
            if !ctx.is_terminated() {
                let one = ctx.emit(
                    Opcode::ConstI64,
                    Ty::I64,
                    vec![],
                    InstData::ConstI64(1),
                    span,
                );
                let idx_next = ctx.emit(
                    Opcode::WrappingAdd,
                    Ty::I64,
                    vec![idx_phi, one],
                    InstData::Integer(IntegerType::I64),
                    span,
                );
                ctx.emit(
                    Opcode::Upsilon,
                    Ty::I64,
                    vec![idx_next],
                    InstData::PhiTarget(idx_phi),
                    span,
                );
                for (name, phi_id) in &phi_ids {
                    if let Some(cur_val) = ctx.lookup(name) {
                        ctx.emit(
                            Opcode::Upsilon,
                            ctx.inst_ty(cur_val),
                            vec![cur_val],
                            InstData::PhiTarget(*phi_id),
                            span,
                        );
                    }
                }
                ctx.emit(
                    Opcode::Jump,
                    Ty::Unit,
                    vec![],
                    InstData::JumpTarget(header_block),
                    span,
                );
            }

            // Bind names to exit-block Phis so post-loop code reads correct values.
            for (name, exit_phi) in &exit_phi_ids {
                ctx.assign(name, *exit_phi);
            }

            // Exit block (Phis already emitted above).
            ctx.switch_to_block(exit_block);
            ctx.emit(Opcode::ConstUnit, Ty::Unit, vec![], InstData::None, span)
        }
        ExprKind::Loop {
            body,
            vow: loop_vow,
        } => {
            let mutated = collect_assigned_vars(body);
            let loop_vars: Vec<(String, InstId)> = mutated
                .into_iter()
                .filter_map(|name| ctx.lookup(&name).map(|id| (name, id)))
                .collect();

            let pre_header_block = ctx.current_block;
            let header_block = ctx.new_block();
            let exit_block = ctx.new_block();

            let mut upsilon_ids: Vec<(String, InstId)> = vec![];
            for (name, pre_val) in &loop_vars {
                let ty = ctx.inst_ty(*pre_val);
                let up_id = ctx.emit(
                    Opcode::Upsilon,
                    ty,
                    vec![*pre_val],
                    InstData::PhiTarget(InstId(u32::MAX)),
                    span,
                );
                upsilon_ids.push((name.clone(), up_id));
            }
            ctx.emit(
                Opcode::Jump,
                Ty::Unit,
                vec![],
                InstData::JumpTarget(header_block),
                span,
            );

            ctx.switch_to_block(header_block);
            let mut phi_ids: Vec<(String, InstId)> = vec![];
            for (name, pre_val) in &loop_vars {
                let ty = ctx.inst_ty(*pre_val);
                let phi_id = ctx.emit(Opcode::Phi, ty, vec![], InstData::None, span);
                phi_ids.push((name.clone(), phi_id));
            }
            for (name, up_id) in &upsilon_ids {
                let phi_id = phi_ids.iter().find(|(n, _)| n == name).unwrap().1;
                backpatch_upsilon(ctx, pre_header_block, *up_id, phi_id);
            }
            for (name, phi_id) in &phi_ids {
                ctx.assign(name, *phi_id);
            }

            if let Some(lv) = loop_vow {
                vow::lower_invariant(ctx, lv);
            }

            // Pre-emit exit-block Phis for mutation variables so break sites
            // can supply updated values via Upsilons.
            let mut exit_phi_ids: Vec<(String, InstId)> = vec![];
            ctx.switch_to_block(exit_block);
            for (name, pre_val) in &loop_vars {
                let ty = ctx.inst_ty(*pre_val);
                let phi_id = ctx.emit(Opcode::Phi, ty, vec![], InstData::None, span);
                exit_phi_ids.push((name.clone(), phi_id));
            }
            ctx.switch_to_block(header_block);

            ctx.loop_exit_blocks.push(exit_block);
            ctx.loop_header_blocks.push(header_block);
            ctx.loop_continue_phis.push(phi_ids.clone());
            ctx.loop_exit_phis.push(exit_phi_ids.clone());
            ctx.loop_continue_idx_phi.push(None);
            ctx.loop_continue_scope_depth.push(ctx.scope.len());
            ctx.loop_break_upsilons.push(Some(Vec::new()));
            lower_block(ctx, body);
            let break_ups = ctx.loop_break_upsilons.pop().unwrap();
            ctx.loop_continue_scope_depth.pop();
            ctx.loop_continue_idx_phi.pop();
            ctx.loop_exit_phis.pop();
            ctx.loop_continue_phis.pop();
            ctx.loop_header_blocks.pop();
            ctx.loop_exit_blocks.pop();

            // Back-edge Upsilons
            if !ctx.is_terminated() {
                for (name, phi_id) in &phi_ids {
                    if let Some(cur_val) = ctx.lookup(name) {
                        ctx.emit(
                            Opcode::Upsilon,
                            ctx.inst_ty(cur_val),
                            vec![cur_val],
                            InstData::PhiTarget(*phi_id),
                            span,
                        );
                    }
                }
                ctx.emit(
                    Opcode::Jump,
                    Ty::Unit,
                    vec![],
                    InstData::JumpTarget(header_block),
                    span,
                );
            }

            // Bind names to exit-block Phis so post-loop code reads correct values.
            for (name, exit_phi) in &exit_phi_ids {
                ctx.assign(name, *exit_phi);
            }

            ctx.switch_to_block(exit_block);

            // If any break carried a value, emit a Phi to merge them.
            if let Some(ups) = break_ups {
                if ups.is_empty() {
                    ctx.emit(Opcode::ConstUnit, Ty::Unit, vec![], InstData::None, span)
                } else {
                    let ty = ups[0].2;
                    let phi_id = ctx.emit(Opcode::Phi, ty, vec![], InstData::None, span);
                    for (block, up_id, _) in &ups {
                        backpatch_upsilon(ctx, *block, *up_id, phi_id);
                    }
                    phi_id
                }
            } else {
                ctx.emit(Opcode::ConstUnit, Ty::Unit, vec![], InstData::None, span)
            }
        }
        ExprKind::Break { value } => {
            let exit_block = ctx
                .loop_exit_blocks
                .last()
                .copied()
                .expect("break outside of loop");

            if let Some(val_expr) = value {
                let val_id = lower_expr(ctx, val_expr);
                // If inside a `loop` (Some), emit Upsilon for the break-value Phi.
                let is_loop = matches!(ctx.loop_break_upsilons.last(), Some(Some(_)));
                if is_loop {
                    let val_ty = ctx.inst_ty(val_id);
                    let up_id = ctx.emit(
                        Opcode::Upsilon,
                        val_ty,
                        vec![val_id],
                        InstData::PhiTarget(InstId(u32::MAX)),
                        span,
                    );
                    let block = ctx.current_block;
                    if let Some(Some(ups)) = ctx.loop_break_upsilons.last_mut() {
                        ups.push((block, up_id, val_ty));
                    }
                }
            }

            // Emit Upsilons for loop mutation variables targeting the
            // exit-block Phis so the exit block receives updated values.
            // Use lookup_at_depth to resolve from the loop header scope,
            // not the current scope, to avoid picking up shadowed bindings.
            let exit_phis = ctx.loop_exit_phis.last().cloned().unwrap_or_default();
            let scope_depth = ctx.loop_continue_scope_depth.last().copied().unwrap_or(0);
            for (name, exit_phi) in &exit_phis {
                if let Some(cur_val) = ctx.lookup_at_depth(name, scope_depth) {
                    ctx.emit(
                        Opcode::Upsilon,
                        ctx.inst_ty(cur_val),
                        vec![cur_val],
                        InstData::PhiTarget(*exit_phi),
                        span,
                    );
                }
            }

            ctx.emit(
                Opcode::Jump,
                Ty::Unit,
                vec![],
                InstData::JumpTarget(exit_block),
                span,
            )
        }
        ExprKind::Continue => {
            let header_block = ctx
                .loop_header_blocks
                .last()
                .copied()
                .expect("continue outside of loop");
            let phis = ctx.loop_continue_phis.last().cloned().unwrap_or_default();
            let idx_phi = ctx.loop_continue_idx_phi.last().copied().flatten();
            let scope_depth = ctx
                .loop_continue_scope_depth
                .last()
                .copied()
                .expect("continue outside of loop");

            // Emit back-edge Upsilons for mutation variables.
            // Use lookup_at_depth to resolve from the loop header scope, not the
            // current scope, so that shadowed bindings in inner blocks are skipped.
            for (name, phi_id) in &phis {
                if let Some(cur_val) = ctx.lookup_at_depth(name, scope_depth) {
                    ctx.emit(
                        Opcode::Upsilon,
                        ctx.inst_ty(cur_val),
                        vec![cur_val],
                        InstData::PhiTarget(*phi_id),
                        span,
                    );
                }
            }

            // For for-each: increment index and emit Upsilon for index Phi.
            if let Some(ip) = idx_phi {
                let one = ctx.emit(
                    Opcode::ConstI64,
                    Ty::I64,
                    vec![],
                    InstData::ConstI64(1),
                    span,
                );
                let idx_next = ctx.emit(
                    Opcode::WrappingAdd,
                    Ty::I64,
                    vec![ip, one],
                    InstData::Integer(IntegerType::I64),
                    span,
                );
                ctx.emit(
                    Opcode::Upsilon,
                    Ty::I64,
                    vec![idx_next],
                    InstData::PhiTarget(ip),
                    span,
                );
            }

            ctx.emit(
                Opcode::Jump,
                Ty::Unit,
                vec![],
                InstData::JumpTarget(header_block),
                span,
            )
        }
        ExprKind::FieldAccess { base, field } => {
            let ptr_id = lower_expr(ctx, base);
            let struct_name = ctx
                .inst_struct_type
                .get(&ptr_id)
                .cloned()
                .unwrap_or_default();
            if struct_name.is_empty() {
                ctx.warn(
                    format!(
                        "FieldGet on untagged instruction %{}, field '{}' -- ICE: returning sentinel index",
                        ptr_id.0, field
                    ),
                    span,
                );
            }
            let field_idx = if let Some(names) = ctx.struct_field_map.get(&struct_name) {
                match names.iter().position(|n| n == field) {
                    Some(idx) => idx,
                    None => {
                        if !struct_name.is_empty() {
                            ctx.warn(
                                format!(
                                    "field '{}' not found in struct '{}' -- ICE: returning sentinel index",
                                    field, struct_name
                                ),
                                span,
                            );
                        }
                        FIELD_IDX_SENTINEL
                    }
                }
            } else {
                if !struct_name.is_empty() {
                    ctx.warn(
                        format!(
                            "struct '{}' not registered -- field lookup ICE: returning sentinel index",
                            struct_name
                        ),
                        span,
                    );
                }
                FIELD_IDX_SENTINEL
            } as u32;
            if field_idx == FIELD_IDX_SENTINEL as u32 {
                ctx.emit(
                    Opcode::ConstI64,
                    Ty::I64,
                    vec![],
                    InstData::ConstI64(0),
                    span,
                )
            } else {
                let field_type_name = ctx
                    .struct_field_type_names
                    .get(&struct_name)
                    .and_then(|names| names.get(field_idx as usize))
                    .cloned()
                    .unwrap_or_default();
                let field_ty = scalar_ty_for_field_type_name(&field_type_name);
                let result_id = ctx.emit(
                    Opcode::FieldGet,
                    field_ty,
                    vec![ptr_id],
                    InstData::FieldIndex(field_idx),
                    span,
                );
                if !field_type_name.is_empty() && !is_scalar_field_type_name(&field_type_name) {
                    ctx.inst_struct_type.insert(result_id, field_type_name);
                }
                if let Some(vec_elems) = ctx.struct_field_vec_elems.get(&struct_name)
                    && let Some(elem_name) = vec_elems.get(field_idx as usize)
                    && !elem_name.is_empty()
                {
                    ctx.inst_vec_elem_types
                        .insert(result_id, vec![elem_name.clone()]);
                }
                result_id
            }
        }
        ExprKind::StructLiteral { name, fields } => {
            let field_names = if let Some(names) = ctx.struct_field_map.get(name) {
                names.clone()
            } else {
                ctx.warn(
                    format!(
                        "struct '{}' not registered -- field lookup ICE: returning sentinel index",
                        name
                    ),
                    span,
                );
                vec![]
            };
            let n_fields = field_names.len().max(fields.len());
            let result_ty = if ctx.linear_owner_names.contains(name) {
                Ty::LinearPtr
            } else {
                Ty::Ptr
            };
            let ptr_id = ctx.emit(
                Opcode::RegionAlloc,
                result_ty,
                vec![],
                InstData::AllocSize {
                    size: (n_fields as u32 + 1) * 8,
                    align: 8,
                },
                span,
            );
            ctx.inst_struct_type.insert(ptr_id, name.clone());
            for (field_name, field_expr) in fields {
                let idx = match field_names.iter().position(|n| n == field_name) {
                    Some(i) => i,
                    None => {
                        if !field_names.is_empty() {
                            ctx.warn(
                                format!("StructLiteral field '{}' not found in struct '{}' -- ICE: returning sentinel index", field_name, name),
                                span,
                            );
                        }
                        FIELD_IDX_SENTINEL
                    }
                } as u32;
                let field_ty = if idx != FIELD_IDX_SENTINEL as u32 {
                    ctx.struct_field_type_names
                        .get(name)
                        .and_then(|types| types.get(idx as usize))
                        .map(|type_name| scalar_ty_for_field_type_name(type_name))
                } else {
                    None
                };
                if let Some(expected) = ctx
                    .struct_field_ast_types
                    .get(name)
                    .and_then(|types| types.get(idx as usize))
                    .cloned()
                {
                    record_wide_expected_ast_context(ctx, field_expr, &expected);
                }
                if let Some(field_ty @ (Ty::I128 | Ty::U128)) = field_ty {
                    record_wide_control_flow_context(ctx, field_expr, field_ty);
                }
                let mut val_id = lower_consumed_expr(ctx, field_expr);
                if idx != FIELD_IDX_SENTINEL as u32 {
                    if let Some(field_ty @ (Ty::I128 | Ty::U128)) = field_ty {
                        val_id = lower_narrow_literal(ctx, field_expr, val_id, field_ty);
                    }
                    ctx.emit(
                        Opcode::FieldSet,
                        Ty::Unit,
                        vec![ptr_id, val_id],
                        InstData::FieldIndex(idx),
                        span,
                    );
                }
            }
            ptr_id
        }
        ExprKind::EnumConstruct { path, fields } => {
            let enum_name = path.first().map(|s| s.as_str()).unwrap_or("");
            let variant_name = path.get(1).map(|s| s.as_str()).unwrap_or("");
            if enum_name == "String" && variant_name == "from" {
                let source_expr = fields.first().expect("String::from requires an argument");
                let source = lower_expr(ctx, source_expr);
                let cloned = ctx.emit(
                    Opcode::Call,
                    Ty::Ptr,
                    vec![source],
                    InstData::CallExtern("__vow_string_clone".to_string()),
                    span,
                );
                ctx.inst_struct_type.insert(cloned, "String".to_string());
                return cloned;
            }
            if enum_name == "String" && variant_name == "from_raw_parts_copy" {
                let ptr_id = fields
                    .first()
                    .map(|e| lower_expr(ctx, e))
                    .unwrap_or_else(|| {
                        ctx.emit(
                            Opcode::ConstI64,
                            Ty::I64,
                            vec![],
                            InstData::ConstI64(0),
                            span,
                        )
                    });
                let len_id = fields
                    .get(1)
                    .map(|e| lower_expr(ctx, e))
                    .unwrap_or_else(|| {
                        ctx.emit(
                            Opcode::ConstI64,
                            Ty::I64,
                            vec![],
                            InstData::ConstI64(0),
                            span,
                        )
                    });
                let result = ctx.emit(
                    Opcode::Call,
                    Ty::Ptr,
                    vec![ptr_id, len_id],
                    InstData::CallExtern("__vow_string_from_raw_parts_copy".to_string()),
                    span,
                );
                ctx.inst_struct_type.insert(result, "String".to_string());
                return result;
            }
            // String::new() builtin — empty string via the String arena router.
            if enum_name == "String" && variant_name == "new" {
                let null_ptr = ctx.emit(
                    Opcode::ConstI64,
                    Ty::I64,
                    vec![],
                    InstData::ConstI64(0),
                    span,
                );
                let len_val = ctx.emit(
                    Opcode::ConstI64,
                    Ty::I64,
                    vec![],
                    InstData::ConstI64(0),
                    span,
                );
                let result = ctx.emit(
                    Opcode::Call,
                    Ty::Ptr,
                    vec![null_ptr, len_val],
                    InstData::CallExtern("__vow_string_new".to_string()),
                    span,
                );
                ctx.inst_struct_type.insert(result, "String".to_string());
                return result;
            }
            // HashMap::new() builtin
            if enum_name == "HashMap" && variant_name == "new" {
                let result = ctx.emit(
                    Opcode::Call,
                    Ty::Ptr,
                    vec![],
                    InstData::CallExtern("__vow_map_new".to_string()),
                    span,
                );
                ctx.inst_struct_type.insert(result, "HashMap".to_string());
                return result;
            }
            // BTreeMap::new() builtin
            if enum_name == "BTreeMap" && variant_name == "new" {
                let result = ctx.emit(
                    Opcode::Call,
                    Ty::Ptr,
                    vec![],
                    InstData::CallExtern("__vow_btreemap_new".to_string()),
                    span,
                );
                ctx.inst_struct_type.insert(result, "BTreeMap".to_string());
                return result;
            }
            // Vec::new() builtin
            if enum_name == "Vec" && variant_name == "new" {
                let size_val = ctx.emit(
                    Opcode::ConstI64,
                    Ty::I64,
                    vec![],
                    InstData::ConstI64(8),
                    span,
                );
                let align_val = ctx.emit(
                    Opcode::ConstI64,
                    Ty::I64,
                    vec![],
                    InstData::ConstI64(8),
                    span,
                );
                let result = ctx.emit(
                    Opcode::Call,
                    Ty::Ptr,
                    vec![size_val, align_val],
                    InstData::CallExtern("__vow_vec_new".to_string()),
                    span,
                );
                ctx.inst_struct_type.insert(result, "Vec".to_string());
                return result;
            }
            if enum_name == "Vec" && variant_name == "from_raw_parts_copy" {
                let ptr_id = fields
                    .first()
                    .map(|e| lower_expr(ctx, e))
                    .unwrap_or_else(|| {
                        ctx.emit(
                            Opcode::ConstI64,
                            Ty::I64,
                            vec![],
                            InstData::ConstI64(0),
                            span,
                        )
                    });
                let len_id = fields
                    .get(1)
                    .map(|e| lower_expr(ctx, e))
                    .unwrap_or_else(|| {
                        ctx.emit(
                            Opcode::ConstI64,
                            Ty::I64,
                            vec![],
                            InstData::ConstI64(0),
                            span,
                        )
                    });
                let result = ctx.emit(
                    Opcode::Call,
                    Ty::Ptr,
                    vec![ptr_id, len_id],
                    InstData::CallExtern("__vow_vec_from_raw_parts_copy_val".to_string()),
                    span,
                );
                ctx.inst_struct_type.insert(result, "Vec".to_string());
                return result;
            }
            let tag = ctx
                .enum_variant_map
                .get(enum_name)
                .and_then(|vs| vs.iter().position(|v| v == variant_name))
                .unwrap_or(0) as i64;
            let payload_tys = ctx
                .enum_variant_payload_tys
                .get(enum_name)
                .and_then(|variants| variants.get(tag as usize))
                .cloned()
                .unwrap_or_default();
            let payload_ast_types = ctx
                .enum_variant_payload_ast_types
                .get(enum_name)
                .and_then(|variants| variants.get(tag as usize))
                .cloned()
                .unwrap_or_default();
            // Evaluate and transfer payloads before allocating the wrapper so
            // built-in Option/Result constructors can inherit linear ownership
            // from their type-erased payload expression.
            let payload_values: Vec<InstId> = fields
                .iter()
                .enumerate()
                .map(|(index, field)| {
                    if let Some(expected) = payload_ast_types.get(index) {
                        record_wide_expected_ast_context(ctx, field, expected);
                    }
                    if let Some(ty @ (Ty::I128 | Ty::U128)) = payload_tys.get(index).copied() {
                        record_wide_control_flow_context(ctx, field, ty);
                    }
                    let original = lower_consumed_expr(ctx, field);
                    match payload_tys.get(index).copied() {
                        Some(ty @ (Ty::I128 | Ty::U128)) => {
                            lower_narrow_literal(ctx, field, original, ty)
                        }
                        _ => original,
                    }
                })
                .collect();
            let contextual_wide_payload_ty = payload_values
                .first()
                .map(|value| ctx.inst_ty(*value))
                .filter(|ty| matches!(ty, Ty::I128 | Ty::U128));
            let owns_linear = ctx.linear_owner_names.contains(enum_name)
                || payload_values
                    .iter()
                    .any(|value| ctx.inst_ty(*value) == Ty::LinearPtr);
            let n_payload = payload_values.len();
            let size = (2 + n_payload) as u32 * 8;
            let ptr_id = ctx.emit(
                Opcode::RegionAlloc,
                if owns_linear { Ty::LinearPtr } else { Ty::Ptr },
                vec![],
                InstData::AllocSize { size, align: 8 },
                span,
            );
            ctx.inst_struct_type.insert(ptr_id, enum_name.to_string());
            if enum_name == "Option" && variant_name == "Some" {
                if let Some(payload_ty) = contextual_wide_payload_ty {
                    ctx.inst_option_elem_ty.insert(ptr_id, payload_ty);
                }
            } else if enum_name == "Result"
                && matches!(variant_name, "Ok" | "Err")
                && let Some(payload_ty) = contextual_wide_payload_ty
            {
                let variant_count = ctx.enum_variant_map.get(enum_name).map_or(2, Vec::len);
                let mut variant_tys = vec![None; variant_count];
                if let Some(slot) = variant_tys.get_mut(tag as usize) {
                    *slot = Some(payload_ty);
                }
                ctx.inst_variant_payload_tys.insert(ptr_id, variant_tys);
            }
            let tag_val = ctx.emit(
                Opcode::ConstI64,
                Ty::I64,
                vec![],
                InstData::ConstI64(tag),
                span,
            );
            ctx.emit(
                Opcode::FieldSet,
                Ty::Unit,
                vec![ptr_id, tag_val],
                InstData::FieldIndex(0),
                span,
            );
            for (i, val_id) in payload_values.into_iter().enumerate() {
                ctx.emit(
                    Opcode::FieldSet,
                    Ty::Unit,
                    vec![ptr_id, val_id],
                    InstData::FieldIndex(1 + i as u32),
                    span,
                );
            }
            ptr_id
        }
        ExprKind::Match { scrutinee, arms } => {
            // The selected arm consumes an owned wrapper. Delaying the transfer
            // until dispatch lets an identifier catchall bind the original
            // obligation instead of rebinding an already-consumed value.
            let ptr_id = lower_expr(ctx, scrutinee);
            let tag_id = ctx.emit(
                Opcode::FieldGet,
                Ty::I64,
                vec![ptr_id],
                InstData::FieldIndex(0),
                span,
            );

            let merge_block = ctx.new_block();

            // Collect mutations across all arm bodies.
            let mutations: Vec<(String, InstId)> = {
                let mut seen = HashSet::new();
                let mut names = vec![];
                for arm in arms {
                    collect_assigned_in_expr(&arm.body, &mut seen, &mut names);
                }
                names
                    .into_iter()
                    .filter_map(|name| ctx.lookup(&name).map(|id| (name, id)))
                    .collect()
            };

            let scope_snap = ctx.snapshot_scope();

            // Merge-reaching arm tracking: (exit_block, result_upsilon, result_ty, mut_vals)
            let mut arm_results: Vec<(BlockId, InstId, Ty, Vec<InstId>)> = Vec::new();
            let mut arm_result_markers: Vec<bool> = Vec::new();
            let mut arm_result_values: Vec<InstId> = Vec::new();
            // Parallel to arm_results/arm_result_markers: the arm body expr, so a marker
            // arm's Upsilon value can be re-narrowed to phi_ty once it's known (see below).
            let mut arm_bodies: Vec<&Expr> = Vec::new();

            let mut arm_iter = arms.iter().peekable();
            while let Some(arm) = arm_iter.next() {
                let is_last = arm_iter.peek().is_none();
                match &arm.pattern.kind {
                    PatKind::EnumVariant { path, inner } => {
                        let enum_name = path.first().map(|s| s.as_str()).unwrap_or("");
                        let variant_name = path.get(1).map(|s| s.as_str()).unwrap_or("");
                        let expected_tag = ctx
                            .enum_variant_map
                            .get(enum_name)
                            .and_then(|vs| vs.iter().position(|v| v == variant_name))
                            .unwrap_or(0) as i64;

                        let arm_block = ctx.new_block();
                        let next_check_block = if is_last { arm_block } else { ctx.new_block() };

                        let expected_id = ctx.emit(
                            Opcode::ConstI64,
                            Ty::I64,
                            vec![],
                            InstData::ConstI64(expected_tag),
                            span,
                        );
                        let cmp_id = ctx.emit(
                            Opcode::Eq,
                            Ty::Bool,
                            vec![tag_id, expected_id],
                            InstData::Integer(IntegerType::I64),
                            span,
                        );
                        ctx.emit(
                            Opcode::Branch,
                            Ty::Unit,
                            vec![cmp_id],
                            InstData::BranchTargets {
                                then_block: arm_block,
                                else_block: next_check_block,
                            },
                            span,
                        );

                        ctx.switch_to_block(arm_block);
                        ctx.emit_linear_consume_if_needed(ptr_id, span);
                        ctx.push_scope();
                        // Narrow scalar Option<T> payloads carry their real IR type.
                        // Aggregate payload metadata is tracked separately so the binding
                        // retains the struct/Vec tag needed by field and index lowering.
                        let payload_ty = ctx
                            .inst_variant_payload_tys
                            .get(&ptr_id)
                            .and_then(|types| types.get(expected_tag as usize))
                            .copied()
                            .flatten()
                            .or_else(|| ctx.inst_option_elem_ty.get(&ptr_id).copied())
                            .unwrap_or(Ty::I64);
                        for (i, inner_pat) in inner.iter().enumerate() {
                            if let PatKind::Ident { name, .. } = &inner_pat.kind {
                                let aggregate = ctx
                                    .pattern_aggregates
                                    .get(&(inner_pat as *const _ as usize))
                                    .cloned();
                                let field_ty =
                                    if aggregate.as_ref().is_some_and(|info| info.is_linear) {
                                        Ty::LinearPtr
                                    } else if aggregate.is_some() {
                                        Ty::Ptr
                                    } else if i == 0 {
                                        payload_ty
                                    } else {
                                        Ty::I64
                                    };
                                let field_val = ctx.emit(
                                    Opcode::FieldGet,
                                    field_ty,
                                    vec![ptr_id],
                                    InstData::FieldIndex(1 + i as u32),
                                    span,
                                );
                                if let Some(info) = aggregate {
                                    tag_pattern_aggregate_metadata(ctx, field_val, info);
                                }
                                ctx.define(name.clone(), field_val);
                            }
                        }
                        let arm_result = lower_expr(ctx, &arm.body);
                        let arm_reaches_merge = !ctx.is_terminated();
                        ctx.pop_scope();

                        if arm_reaches_merge {
                            let arm_ty = ctx.inst_ty(arm_result);
                            let arm_mut_vals: Vec<InstId> = mutations
                                .iter()
                                .map(|(name, pre_id)| ctx.lookup(name).unwrap_or(*pre_id))
                                .collect();

                            let up_id = ctx.emit(
                                Opcode::Upsilon,
                                Ty::Unit,
                                vec![arm_result],
                                InstData::PhiTarget(InstId(u32::MAX)),
                                span,
                            );
                            ctx.emit(
                                Opcode::Jump,
                                Ty::Unit,
                                vec![],
                                InstData::JumpTarget(merge_block),
                                span,
                            );
                            let exit_block = ctx.current_block;
                            arm_results.push((exit_block, up_id, arm_ty, arm_mut_vals));
                            arm_result_markers.push(expr_is_coercible_int_marker(&arm.body));
                            arm_result_values.push(arm_result);
                            arm_bodies.push(&arm.body);
                        }

                        ctx.restore_scope(scope_snap.clone());

                        if !is_last {
                            ctx.switch_to_block(next_check_block);
                        }
                    }
                    PatKind::Wildcard | PatKind::Ident { .. } => {
                        if let PatKind::Ident { name, .. } = &arm.pattern.kind {
                            ctx.push_scope();
                            ctx.define(name.clone(), ptr_id);
                        } else {
                            ctx.emit_linear_consume_if_needed(ptr_id, span);
                            ctx.push_scope();
                        }
                        let arm_result = lower_expr(ctx, &arm.body);
                        let arm_reaches_merge = !ctx.is_terminated();
                        ctx.pop_scope();

                        if arm_reaches_merge {
                            let arm_ty = ctx.inst_ty(arm_result);
                            let arm_mut_vals: Vec<InstId> = mutations
                                .iter()
                                .map(|(name, pre_id)| ctx.lookup(name).unwrap_or(*pre_id))
                                .collect();

                            let up_id = ctx.emit(
                                Opcode::Upsilon,
                                Ty::Unit,
                                vec![arm_result],
                                InstData::PhiTarget(InstId(u32::MAX)),
                                span,
                            );
                            ctx.emit(
                                Opcode::Jump,
                                Ty::Unit,
                                vec![],
                                InstData::JumpTarget(merge_block),
                                span,
                            );
                            let exit_block = ctx.current_block;
                            arm_results.push((exit_block, up_id, arm_ty, arm_mut_vals));
                            arm_result_markers.push(expr_is_coercible_int_marker(&arm.body));
                            arm_result_values.push(arm_result);
                            arm_bodies.push(&arm.body);
                        }

                        ctx.restore_scope(scope_snap.clone());
                    }
                    _ => {
                        ctx.emit_linear_consume_if_needed(ptr_id, span);
                        let arm_block = ctx.current_block;
                        let unit =
                            ctx.emit(Opcode::ConstUnit, Ty::Unit, vec![], InstData::None, span);

                        let arm_mut_vals: Vec<InstId> =
                            mutations.iter().map(|(_, pre_id)| *pre_id).collect();

                        let up_id = ctx.emit(
                            Opcode::Upsilon,
                            Ty::Unit,
                            vec![unit],
                            InstData::PhiTarget(InstId(u32::MAX)),
                            span,
                        );
                        ctx.emit(
                            Opcode::Jump,
                            Ty::Unit,
                            vec![],
                            InstData::JumpTarget(merge_block),
                            span,
                        );
                        arm_results.push((arm_block, up_id, Ty::Unit, arm_mut_vals));
                        arm_result_markers.push(false);
                        arm_result_values.push(unit);
                        arm_bodies.push(&arm.body);
                    }
                }
            }

            ctx.restore_scope(scope_snap);
            ctx.switch_to_block(merge_block);

            if arm_results.is_empty() {
                return ctx.emit(Opcode::Unreachable, Ty::Unit, vec![], InstData::None, span);
            }

            // Create Phis for mutated variables.
            for (i, (name, pre_id)) in mutations.iter().enumerate() {
                let changed = arm_results.iter().any(|(_, _, _, mvs)| mvs[i] != *pre_id);
                if !changed {
                    continue;
                }
                let phi_ty = arm_results
                    .iter()
                    .skip(1)
                    .fold(ctx.inst_ty(arm_results[0].3[i]), |ty, (_, _, _, values)| {
                        merge_phi_ty(ty, ctx.inst_ty(values[i]))
                    });
                let phi_id = ctx.emit(Opcode::Phi, phi_ty, vec![], InstData::None, span);
                for (exit_block, _, _, arm_mut_vals) in &arm_results {
                    ctx.switch_to_block(*exit_block);
                    ctx.emit(
                        Opcode::Upsilon,
                        phi_ty,
                        vec![arm_mut_vals[i]],
                        InstData::PhiTarget(phi_id),
                        span,
                    );
                }
                ctx.switch_to_block(merge_block);
                ctx.assign(name, phi_id);
            }

            let phi_ty = choose_match_result_ty(&arm_results, &arm_result_markers);

            // A marker arm's Upsilon still carries its default `i64`-width value
            // (see expr_is_coercible_int_marker). Now that the merge's real result
            // type is known, re-narrow those arms so every Upsilon feeding the Phi
            // shares its Cranelift register width -- otherwise a plain literal arm
            // (e.g. `None => -1`) merging with a genuinely narrow-typed arm (e.g.
            // `Some(v) => v`) produces a width-mismatched Cranelift Phi.
            if matches!(
                phi_ty,
                Ty::I8 | Ty::U8 | Ty::I16 | Ty::U16 | Ty::I32 | Ty::U32 | Ty::I128 | Ty::U128
            ) {
                for i in 0..arm_results.len() {
                    let arm_block = arm_results[i].0;
                    let up_id = arm_results[i].1;
                    let arm_ty = arm_results[i].2;
                    if !arm_result_markers[i] || arm_ty == phi_ty {
                        continue;
                    }
                    let block_idx = arm_block.0 as usize;
                    // Locate the arm's own result Upsilon by id rather than assuming
                    // block position: the "Phis for mutated variables" pass above may
                    // have already appended extra Upsilons after this arm's own
                    // [Upsilon, Jump] pair, so it's not reliably the block's tail.
                    let up_pos = ctx.func.blocks[block_idx]
                        .insts
                        .iter()
                        .position(|inst| inst.id == up_id);
                    let Some(up_pos) = up_pos else { continue };
                    let old_arg = ctx.func.blocks[block_idx].insts[up_pos].args[0];
                    ctx.switch_to_block(arm_block);
                    let tail = ctx.func.blocks[block_idx].insts.split_off(up_pos + 1);
                    ctx.func.blocks[block_idx].insts.truncate(up_pos);
                    let narrowed = lower_narrow_literal(ctx, arm_bodies[i], old_arg, phi_ty);
                    let new_up_id = ctx.emit(
                        Opcode::Upsilon,
                        Ty::Unit,
                        vec![narrowed],
                        InstData::PhiTarget(InstId(u32::MAX)),
                        span,
                    );
                    // Restore the terminator and any mutation Upsilons after the
                    // replacement marker expression and result Upsilon.
                    ctx.func.blocks[block_idx].insts.extend(tail);
                    arm_results[i].1 = new_up_id;
                }
                ctx.switch_to_block(merge_block);
            }

            let phi_id = ctx.emit(Opcode::Phi, phi_ty, vec![], InstData::None, span);
            copy_compatible_aggregate_metadata(ctx, &arm_result_values, phi_id);

            for (arm_block, up_id, _, _) in &arm_results {
                backpatch_upsilon(ctx, *arm_block, *up_id, phi_id);
            }

            phi_id
        }
        ExprKind::MethodCall {
            receiver,
            method,
            args,
        } => {
            let recv_id = lower_expr(ctx, receiver);
            let recv_struct = ctx.inst_struct_type.get(&recv_id).cloned().or_else(|| {
                if ctx
                    .string_exprs
                    .contains(&(receiver.as_ref() as *const Expr as usize))
                {
                    Some("String".to_string())
                } else {
                    None
                }
            });
            match (recv_struct.as_deref(), method.as_str()) {
                (Some("String"), "len") => ctx.emit(
                    Opcode::Call,
                    Ty::I64,
                    vec![recv_id],
                    InstData::CallExtern("__vow_string_len".to_string()),
                    span,
                ),
                (Some("String"), "push_str") => {
                    let arg_id = args
                        .first()
                        .map(|e| lower_consumed_expr(ctx, e))
                        .unwrap_or_else(|| {
                            ctx.emit(Opcode::ConstUnit, Ty::Unit, vec![], InstData::None, span)
                        });
                    ctx.emit(
                        Opcode::Call,
                        Ty::Unit,
                        vec![recv_id, arg_id],
                        InstData::CallExtern("__vow_string_push_str".to_string()),
                        span,
                    )
                }
                (Some("String"), "eq") => {
                    let arg_id = args
                        .first()
                        .map(|e| lower_consumed_expr(ctx, e))
                        .unwrap_or_else(|| {
                            ctx.emit(Opcode::ConstUnit, Ty::Unit, vec![], InstData::None, span)
                        });
                    ctx.emit(
                        Opcode::Call,
                        Ty::Bool,
                        vec![recv_id, arg_id],
                        InstData::CallExtern("__vow_string_eq".to_string()),
                        span,
                    )
                }
                (Some("String"), "contains") => {
                    let arg_expr = args.first();
                    let arg_id = arg_expr.map(|e| lower_expr(ctx, e)).unwrap_or_else(|| {
                        ctx.emit(Opcode::ConstUnit, Ty::Unit, vec![], InstData::None, span)
                    });
                    ctx.emit(
                        Opcode::Call,
                        Ty::Bool,
                        vec![recv_id, arg_id],
                        InstData::CallExtern("__vow_string_contains".to_string()),
                        span,
                    )
                }
                (Some("String"), "byte_at") => {
                    let idx_id = args
                        .first()
                        .map(|e| lower_consumed_expr(ctx, e))
                        .unwrap_or_else(|| {
                            ctx.emit(Opcode::ConstUnit, Ty::Unit, vec![], InstData::None, span)
                        });
                    ctx.emit(
                        Opcode::Call,
                        Ty::I64,
                        vec![recv_id, idx_id],
                        InstData::CallExtern("__vow_string_byte_at".to_string()),
                        span,
                    )
                }
                (Some("String"), "push_byte") => {
                    let byte_id = args
                        .first()
                        .map(|e| lower_consumed_expr(ctx, e))
                        .unwrap_or_else(|| {
                            ctx.emit(Opcode::ConstUnit, Ty::Unit, vec![], InstData::None, span)
                        });
                    ctx.emit(
                        Opcode::Call,
                        Ty::Unit,
                        vec![recv_id, byte_id],
                        InstData::CallExtern("__vow_string_push_byte".to_string()),
                        span,
                    )
                }
                (Some("String"), "clear") => ctx.emit(
                    Opcode::Call,
                    Ty::Unit,
                    vec![recv_id],
                    InstData::CallExtern("__vow_string_clear".to_string()),
                    span,
                ),
                (Some("String"), "substring") => {
                    let start_id = args
                        .first()
                        .map(|e| lower_consumed_expr(ctx, e))
                        .unwrap_or_else(|| {
                            ctx.emit(
                                Opcode::ConstI64,
                                Ty::I64,
                                vec![],
                                InstData::ConstI64(0),
                                span,
                            )
                        });
                    let end_id = args
                        .get(1)
                        .map(|e| lower_consumed_expr(ctx, e))
                        .unwrap_or_else(|| {
                            ctx.emit(
                                Opcode::ConstI64,
                                Ty::I64,
                                vec![],
                                InstData::ConstI64(0),
                                span,
                            )
                        });
                    let result = ctx.emit(
                        Opcode::Call,
                        Ty::Ptr,
                        vec![recv_id, start_id, end_id],
                        InstData::CallExtern("__vow_string_substring".to_string()),
                        span,
                    );
                    ctx.inst_struct_type.insert(result, "String".to_string());
                    result
                }
                (Some("String"), "parse_i64") => {
                    let result = ctx.emit(
                        Opcode::Call,
                        Ty::Ptr,
                        vec![recv_id],
                        InstData::CallExtern("__vow_string_parse_i64_opt".to_string()),
                        span,
                    );
                    ctx.inst_struct_type.insert(result, "Option".to_string());
                    result
                }
                (Some("String"), "parse_u64") => {
                    let result = ctx.emit(
                        Opcode::Call,
                        Ty::Ptr,
                        vec![recv_id],
                        InstData::CallExtern("__vow_string_parse_u64_opt".to_string()),
                        span,
                    );
                    ctx.inst_struct_type.insert(result, "Option".to_string());
                    result
                }
                (Some("HashMap"), "len") => ctx.emit(
                    Opcode::Call,
                    Ty::I64,
                    vec![recv_id],
                    InstData::CallExtern("__vow_map_len".to_string()),
                    span,
                ),
                (Some("BTreeMap"), "len") => ctx.emit(
                    Opcode::Call,
                    Ty::I64,
                    vec![recv_id],
                    InstData::CallExtern("__vow_btreemap_len".to_string()),
                    span,
                ),
                (Some("BTreeMap"), "insert") => {
                    let k_id = args
                        .first()
                        .map(|e| lower_consumed_expr(ctx, e))
                        .unwrap_or_else(|| {
                            ctx.emit(Opcode::ConstUnit, Ty::Unit, vec![], InstData::None, span)
                        });
                    let v_id = args
                        .get(1)
                        .map(|e| lower_consumed_expr(ctx, e))
                        .unwrap_or_else(|| {
                            ctx.emit(Opcode::ConstUnit, Ty::Unit, vec![], InstData::None, span)
                        });
                    let result = ctx.emit(
                        Opcode::Call,
                        Ty::Ptr,
                        vec![recv_id, k_id, v_id],
                        InstData::CallExtern("__vow_btreemap_insert".to_string()),
                        span,
                    );
                    ctx.inst_struct_type.insert(result, "Option".to_string());
                    result
                }
                (Some("BTreeMap"), "get") => {
                    let k_id = args
                        .first()
                        .map(|e| lower_consumed_expr(ctx, e))
                        .unwrap_or_else(|| {
                            ctx.emit(Opcode::ConstUnit, Ty::Unit, vec![], InstData::None, span)
                        });
                    let result = ctx.emit(
                        Opcode::Call,
                        Ty::Ptr,
                        vec![recv_id, k_id],
                        InstData::CallExtern("__vow_btreemap_get".to_string()),
                        span,
                    );
                    ctx.inst_struct_type.insert(result, "Option".to_string());
                    result
                }
                (Some("BTreeMap"), "contains") => {
                    let k_id = args
                        .first()
                        .map(|e| lower_consumed_expr(ctx, e))
                        .unwrap_or_else(|| {
                            ctx.emit(Opcode::ConstUnit, Ty::Unit, vec![], InstData::None, span)
                        });
                    ctx.emit(
                        Opcode::Call,
                        Ty::Bool,
                        vec![recv_id, k_id],
                        InstData::CallExtern("__vow_btreemap_contains".to_string()),
                        span,
                    )
                }
                (Some("HashMap"), "insert") => {
                    let k_id = args
                        .first()
                        .map(|e| lower_consumed_expr(ctx, e))
                        .unwrap_or_else(|| {
                            ctx.emit(Opcode::ConstUnit, Ty::Unit, vec![], InstData::None, span)
                        });
                    let v_id = args
                        .get(1)
                        .map(|e| lower_consumed_expr(ctx, e))
                        .unwrap_or_else(|| {
                            ctx.emit(Opcode::ConstUnit, Ty::Unit, vec![], InstData::None, span)
                        });
                    ctx.emit(
                        Opcode::Call,
                        Ty::Unit,
                        vec![recv_id, k_id, v_id],
                        InstData::CallExtern("__vow_map_insert".to_string()),
                        span,
                    )
                }
                (Some("HashMap"), "get") => {
                    let k_id = args
                        .first()
                        .map(|e| lower_consumed_expr(ctx, e))
                        .unwrap_or_else(|| {
                            ctx.emit(Opcode::ConstUnit, Ty::Unit, vec![], InstData::None, span)
                        });
                    ctx.emit(
                        Opcode::Call,
                        Ty::I64,
                        vec![recv_id, k_id],
                        InstData::CallExtern("__vow_map_get".to_string()),
                        span,
                    )
                }
                (Some("HashMap"), "contains_key") => {
                    let k_id = args
                        .first()
                        .map(|e| lower_consumed_expr(ctx, e))
                        .unwrap_or_else(|| {
                            ctx.emit(Opcode::ConstUnit, Ty::Unit, vec![], InstData::None, span)
                        });
                    ctx.emit(
                        Opcode::Call,
                        Ty::Bool,
                        vec![recv_id, k_id],
                        InstData::CallExtern("__vow_map_contains".to_string()),
                        span,
                    )
                }
                (Some("HashMap"), "remove") => {
                    let k_id = args
                        .first()
                        .map(|e| lower_consumed_expr(ctx, e))
                        .unwrap_or_else(|| {
                            ctx.emit(Opcode::ConstUnit, Ty::Unit, vec![], InstData::None, span)
                        });
                    ctx.emit(
                        Opcode::Call,
                        Ty::Unit,
                        vec![recv_id, k_id],
                        InstData::CallExtern("__vow_map_remove".to_string()),
                        span,
                    )
                }
                (_, "len") => ctx.emit(
                    Opcode::Call,
                    Ty::I64,
                    vec![recv_id],
                    InstData::CallExtern("__vow_vec_len".to_string()),
                    span,
                ),
                (_, "push") => {
                    let elem_ty = ctx
                        .inst_vec_elem_types
                        .get(&recv_id)
                        .and_then(|path| path.first())
                        .filter(|name| is_scalar_field_type_name(name))
                        .map(|name| scalar_ty_for_field_type_name(name))
                        .filter(|ty| matches!(ty, Ty::I128 | Ty::U128));
                    let elem_id = args
                        .first()
                        .map(|e| {
                            if let Some(ty) = elem_ty {
                                record_wide_control_flow_context(ctx, e, ty);
                            }
                            let original = lower_consumed_expr(ctx, e);
                            elem_ty
                                .map(|ty| lower_narrow_literal(ctx, e, original, ty))
                                .unwrap_or(original)
                        })
                        .unwrap_or_else(|| {
                            ctx.emit(Opcode::ConstUnit, Ty::Unit, vec![], InstData::None, span)
                        });
                    ctx.emit(
                        Opcode::Call,
                        Ty::Unit,
                        vec![recv_id, elem_id],
                        InstData::CallExtern("__vow_vec_push_val".to_string()),
                        span,
                    )
                }
                (_, "pop") => ctx.emit(
                    Opcode::Call,
                    Ty::Unit,
                    vec![recv_id],
                    InstData::CallExtern("__vow_vec_pop".to_string()),
                    span,
                ),
                (_, "clear") => ctx.emit(
                    Opcode::Call,
                    Ty::Unit,
                    vec![recv_id],
                    InstData::CallExtern("__vow_vec_clear".to_string()),
                    span,
                ),
                (_, "truncate") => {
                    let len_id = args
                        .first()
                        .map(|e| lower_consumed_expr(ctx, e))
                        .unwrap_or_else(|| {
                            ctx.emit(
                                Opcode::ConstI64,
                                Ty::I64,
                                vec![],
                                InstData::ConstI64(0),
                                span,
                            )
                        });
                    ctx.emit(
                        Opcode::Call,
                        Ty::Unit,
                        vec![recv_id, len_id],
                        InstData::CallExtern("__vow_vec_truncate".to_string()),
                        span,
                    )
                }
                _ => {
                    for a in args {
                        lower_consumed_expr(ctx, a);
                    }
                    ctx.emit(Opcode::ConstUnit, Ty::Unit, vec![], InstData::None, span)
                }
            }
        }
        ExprKind::Index { base, index } => {
            let vec_ptr = lower_expr(ctx, base);
            let idx_id = lower_expr(ctx, index);
            let result = ctx.emit(
                Opcode::Call,
                Ty::I64,
                vec![vec_ptr, idx_id],
                InstData::CallExtern("__vow_vec_get_val".to_string()),
                span,
            );
            propagate_vec_element_metadata(ctx, vec_ptr, result);
            result
        }
        // ? operator: unwrap Option::Some or short-circuit with None
        ExprKind::Question { expr: inner } => {
            let ptr_id = lower_consumed_expr(ctx, inner);
            // Load discriminant from field 0
            let tag_id = ctx.emit(
                Opcode::FieldGet,
                Ty::I64,
                vec![ptr_id],
                InstData::FieldIndex(0),
                span,
            );
            let zero_id = ctx.emit(
                Opcode::ConstI64,
                Ty::I64,
                vec![],
                InstData::ConstI64(0),
                span,
            );
            // tag == 0 means None (short-circuit) for Option; Ok (continue) for Result
            let is_none = ctx.emit(
                Opcode::Eq,
                Ty::Bool,
                vec![tag_id, zero_id],
                InstData::Integer(IntegerType::I64),
                span,
            );
            let early_return_block = ctx.new_block();
            let continue_block = ctx.new_block();
            ctx.emit(
                Opcode::Branch,
                Ty::Unit,
                vec![is_none],
                InstData::BranchTargets {
                    then_block: early_return_block,
                    else_block: continue_block,
                },
                span,
            );

            // Early return: wrap as None and return
            ctx.switch_to_block(early_return_block);
            let none_size: u32 = 16; // discriminant + guard slot
            let none_ptr = ctx.emit(
                Opcode::RegionAlloc,
                if ctx.func.return_ty == Ty::LinearPtr {
                    Ty::LinearPtr
                } else {
                    Ty::Ptr
                },
                vec![],
                InstData::AllocSize {
                    size: none_size,
                    align: 8,
                },
                span,
            );
            let none_tag = ctx.emit(
                Opcode::ConstI64,
                Ty::I64,
                vec![],
                InstData::ConstI64(0),
                span,
            );
            ctx.emit(
                Opcode::FieldSet,
                Ty::Unit,
                vec![none_ptr, none_tag],
                InstData::FieldIndex(0),
                span,
            );
            if let Some(vow_block) = ctx.vow_block.clone() {
                vow::lower_ensures(ctx, &vow_block, none_ptr);
            }
            ctx.emit(
                Opcode::Return,
                Ty::Unit,
                vec![none_ptr],
                InstData::None,
                span,
            );

            // Continue: extract payload from field 1
            ctx.switch_to_block(continue_block);
            let aggregate = ctx
                .pattern_aggregates
                .get(&(expr as *const _ as usize))
                .cloned();
            let payload_ty = if aggregate.as_ref().is_some_and(|info| info.is_linear) {
                Ty::LinearPtr
            } else if aggregate.is_some() {
                Ty::Ptr
            } else {
                ctx.inst_option_elem_ty
                    .get(&ptr_id)
                    .copied()
                    .unwrap_or(Ty::I64)
            };
            let payload = ctx.emit(
                Opcode::FieldGet,
                payload_ty,
                vec![ptr_id],
                InstData::FieldIndex(1),
                span,
            );
            if let Some(info) = aggregate {
                tag_pattern_aggregate_metadata(ctx, payload, info);
            }
            payload
        }
        ExprKind::Cast { expr, target_ty } => {
            let tgt = lower_ty_with_linear(target_ty, &ctx.linear_owner_names, &ctx.type_aliases);
            if matches!(tgt, Ty::I128 | Ty::U128) {
                record_wide_marker_context(ctx, expr, tgt);
            }
            let val = lower_expr(ctx, expr);
            let src_ty = ctx.inst_ty(val);
            if let ExprKind::Lit(Lit::Int(v)) = &expr.kind {
                match tgt {
                    Ty::U8 => ctx.emit(
                        Opcode::ConstU8,
                        Ty::U8,
                        vec![],
                        InstData::ConstU8(*v as u8),
                        span,
                    ),
                    Ty::U64 => ctx.emit(
                        Opcode::ConstU64,
                        Ty::U64,
                        vec![],
                        InstData::ConstU64(*v as u64),
                        span,
                    ),
                    Ty::I128 => ctx.emit(
                        Opcode::ConstI128,
                        Ty::I128,
                        vec![],
                        InstData::ConstI128(*v as i128),
                        span,
                    ),
                    Ty::U128 => ctx.emit(
                        Opcode::ConstU128,
                        Ty::U128,
                        vec![],
                        InstData::ConstU128(*v),
                        span,
                    ),
                    _ if ir_ty_is_integer(tgt) => ctx.emit(
                        Opcode::IntCast,
                        tgt,
                        vec![val],
                        InstData::IntegerCast {
                            from: integer_type_for_ir_ty(src_ty),
                            to: integer_type_for_ir_ty(tgt),
                        },
                        span,
                    ),
                    _ => val,
                }
            } else if ir_ty_is_integer(src_ty) && ir_ty_is_integer(tgt) {
                ctx.emit(
                    Opcode::IntCast,
                    tgt,
                    vec![val],
                    InstData::IntegerCast {
                        from: integer_type_for_ir_ty(src_ty),
                        to: integer_type_for_ir_ty(tgt),
                    },
                    span,
                )
            } else {
                val
            }
        }
        _ => todo!("IR lowering not implemented for {:?}", expr.kind),
    }
}

fn lower_static_string_literal(
    ctx: &mut LowerCtx,
    expr: &vow_syntax::ast::Expr,
) -> Option<(InstId, InstId)> {
    let Lit::String(s) = (match &expr.kind {
        ExprKind::Lit(lit) => lit,
        _ => return None,
    }) else {
        return None;
    };
    let idx = ctx.intern_str(s);
    let ptr = ctx.emit(
        Opcode::ConstStr,
        Ty::Ptr,
        vec![],
        InstData::ConstStr(idx),
        expr.span,
    );
    let len = ctx.emit(
        Opcode::ConstI64,
        Ty::I64,
        vec![],
        InstData::ConstI64(s.len() as i64),
        expr.span,
    );
    Some((ptr, len))
}

fn lower_consumed_expr(ctx: &mut LowerCtx, expr: &vow_syntax::ast::Expr) -> InstId {
    let id = lower_expr(ctx, expr);
    ctx.emit_linear_consume_if_needed(id, expr.span);
    id
}

fn integer_type_for_ir_ty(ty: Ty) -> IntegerType {
    match ty {
        Ty::I8 => IntegerType::I8,
        Ty::U8 => IntegerType::U8,
        Ty::I16 => IntegerType::I16,
        Ty::U16 => IntegerType::U16,
        Ty::I32 => IntegerType::I32,
        Ty::U32 => IntegerType::U32,
        Ty::I64 => IntegerType::I64,
        Ty::U64 => IntegerType::U64,
        Ty::I128 => IntegerType::I128,
        Ty::U128 => IntegerType::U128,
        _ => IntegerType::I64,
    }
}

fn expr_is_integer_literal(expr: &Expr) -> bool {
    expr_is_coercible_int_marker(expr)
}

fn integer_marker_from_block(block: &Block) -> Option<&Expr> {
    if let Some(expr) = &block.trailing_expr {
        return Some(expr);
    }
    if let Some(Stmt::Expr {
        expr,
        has_semicolon: false,
        ..
    }) = block.stmts.last()
    {
        return Some(expr);
    }
    None
}

fn known_wide_expr_ty(ctx: &LowerCtx, expr: &Expr) -> Option<Ty> {
    let ty = match &expr.kind {
        ExprKind::Ident(name) => ctx.lookup(name).map(|id| ctx.inst_ty(id)),
        ExprKind::Cast { target_ty, .. } => Some(lower_ty_with_linear(
            target_ty,
            &ctx.linear_owner_names,
            &ctx.type_aliases,
        )),
        ExprKind::Block(block) => {
            integer_marker_from_block(block).and_then(|result| known_wide_expr_ty(ctx, result))
        }
        _ => None,
    };
    ty.filter(|ty| matches!(ty, Ty::I128 | Ty::U128))
}

fn known_struct_expr_name(ctx: &LowerCtx, expr: &Expr) -> Option<String> {
    match &expr.kind {
        ExprKind::Ident(name) => ctx
            .lookup(name)
            .and_then(|id| ctx.inst_struct_type.get(&id).cloned()),
        ExprKind::FieldAccess { base, field } => {
            let base_name = known_struct_expr_name(ctx, base)?;
            let field_idx = ctx
                .struct_field_map
                .get(&base_name)?
                .iter()
                .position(|name| name == field)?;
            ctx.struct_field_type_names
                .get(&base_name)
                .and_then(|types| types.get(field_idx))
                .cloned()
        }
        _ => None,
    }
}

fn known_field_assignment_ty(ctx: &LowerCtx, lhs: &Expr) -> Option<Ty> {
    let ExprKind::FieldAccess { base, field } = &lhs.kind else {
        return None;
    };
    let struct_name = known_struct_expr_name(ctx, base)?;
    let field_idx = ctx
        .struct_field_map
        .get(&struct_name)?
        .iter()
        .position(|name| name == field)?;
    ctx.struct_field_type_names
        .get(&struct_name)
        .and_then(|types| types.get(field_idx))
        .map(|name| scalar_ty_for_field_type_name(name))
        .filter(|ty| matches!(ty, Ty::I128 | Ty::U128))
}

fn known_vec_element_path(ctx: &LowerCtx, expr: &Expr) -> Option<Vec<String>> {
    match &expr.kind {
        ExprKind::Ident(name) => ctx
            .lookup(name)
            .and_then(|id| ctx.inst_vec_elem_types.get(&id))
            .cloned(),
        ExprKind::FieldAccess { base, field } => {
            let struct_name = known_struct_expr_name(ctx, base)?;
            let field_idx = ctx
                .struct_field_map
                .get(&struct_name)?
                .iter()
                .position(|name| name == field)?;
            ctx.struct_field_vec_elems
                .get(&struct_name)
                .and_then(|types| types.get(field_idx))
                .filter(|name| !name.is_empty())
                .map(|name| vec![name.clone()])
        }
        ExprKind::Index { base, .. } => {
            let mut path = known_vec_element_path(ctx, base)?;
            if path.is_empty() {
                return None;
            }
            path.remove(0);
            Some(path)
        }
        _ => None,
    }
}

fn known_index_assignment_ty(ctx: &LowerCtx, lhs: &Expr) -> Option<Ty> {
    let ExprKind::Index { base, .. } = &lhs.kind else {
        return None;
    };
    known_vec_element_path(ctx, base)
        .and_then(|path| path.first().cloned())
        .filter(|name| is_scalar_field_type_name(name))
        .map(|name| scalar_ty_for_field_type_name(&name))
        .filter(|ty| matches!(ty, Ty::I128 | Ty::U128))
}

fn wide_context_contains_control_flow(expr: &Expr) -> bool {
    match &expr.kind {
        ExprKind::If { .. } | ExprKind::Match { .. } | ExprKind::Loop { .. } => true,
        ExprKind::UnaryOp {
            op: UnOp::Neg,
            operand,
        } => wide_context_contains_control_flow(operand),
        ExprKind::BinaryOp { lhs, rhs, .. } => {
            wide_context_contains_control_flow(lhs) || wide_context_contains_control_flow(rhs)
        }
        ExprKind::Block(block) => {
            integer_marker_from_block(block).is_some_and(wide_context_contains_control_flow)
        }
        _ => false,
    }
}

fn record_wide_marker_context(ctx: &mut LowerCtx, expr: &Expr, ty: Ty) {
    if !matches!(ty, Ty::I128 | Ty::U128) {
        return;
    }
    ctx.wide_literal_contexts
        .insert(expr as *const _ as usize, ty);
    match &expr.kind {
        ExprKind::UnaryOp {
            op: UnOp::Neg,
            operand,
        } => record_wide_marker_context(ctx, operand, ty),
        ExprKind::BinaryOp { op, lhs, rhs } => {
            record_wide_marker_context(ctx, lhs, ty);
            if !matches!(op, BinOp::Shl | BinOp::Shr) {
                record_wide_marker_context(ctx, rhs, ty);
            }
        }
        ExprKind::Block(block) => {
            if let Some(result) = integer_marker_from_block(block) {
                record_wide_marker_context(ctx, result, ty);
            }
        }
        ExprKind::If {
            then_branch,
            else_branch: Some(else_expr),
            ..
        } => {
            if let Some(result) = integer_marker_from_block(then_branch) {
                record_wide_marker_context(ctx, result, ty);
            }
            record_wide_marker_context(ctx, else_expr, ty);
        }
        ExprKind::Match { arms, .. } => {
            for arm in arms {
                record_wide_marker_context(ctx, &arm.body, ty);
            }
        }
        ExprKind::Loop { body, .. } => {
            for value in loop_break_values(body) {
                record_wide_marker_context(ctx, value, ty);
            }
        }
        _ => {}
    }
}

fn record_wide_control_flow_context(ctx: &mut LowerCtx, expr: &Expr, ty: Ty) {
    if matches!(ty, Ty::I128 | Ty::U128) && wide_context_contains_control_flow(expr) {
        record_wide_marker_context(ctx, expr, ty);
    }
}

fn record_wide_expected_ast_context(ctx: &mut LowerCtx, expr: &Expr, expected: &AstType) {
    let expected = resolve_type_alias(expected, &ctx.type_aliases).clone();
    if matches!(expected, AstType::Generic { .. }) {
        match &expr.kind {
            ExprKind::Block(block) => {
                if let Some(result) = block.trailing_expr.as_deref() {
                    record_wide_expected_ast_context(ctx, result, &expected);
                }
                return;
            }
            ExprKind::If {
                then_branch,
                else_branch,
                ..
            } => {
                if let Some(result) = then_branch.trailing_expr.as_deref() {
                    record_wide_expected_ast_context(ctx, result, &expected);
                }
                if let Some(else_expr) = else_branch.as_deref() {
                    record_wide_expected_ast_context(ctx, else_expr, &expected);
                }
                return;
            }
            ExprKind::Match { arms, .. } => {
                for arm in arms {
                    record_wide_expected_ast_context(ctx, &arm.body, &expected);
                }
                return;
            }
            ExprKind::Loop { body, .. } => {
                for value in loop_break_values(body) {
                    record_wide_expected_ast_context(ctx, value, &expected);
                }
                return;
            }
            _ => {}
        }
    }
    match expected {
        AstType::Named { name, .. } => {
            let ty = match name.as_str() {
                "i128" => Some(Ty::I128),
                "u128" => Some(Ty::U128),
                _ => None,
            };
            if let Some(ty) = ty {
                record_wide_marker_context(ctx, expr, ty);
            }
        }
        AstType::Generic { name, args, .. } => {
            let ExprKind::EnumConstruct { path, fields } = &expr.kind else {
                return;
            };
            if path.first() != Some(&name) {
                return;
            }
            let payload_index = match (name.as_str(), path.get(1).map(String::as_str)) {
                ("Option", Some("Some")) | ("Result", Some("Ok")) => Some(0),
                ("Result", Some("Err")) => Some(1),
                _ => None,
            };
            if let Some(payload_ty) = payload_index.and_then(|index| args.get(index))
                && let Some(field) = fields.first()
            {
                record_wide_expected_ast_context(ctx, field, payload_ty);
            }
        }
        _ => {}
    }
}

fn emit_narrow_integer_constant(ctx: &mut LowerCtx, value: u128, ty: Ty, span: Span) -> InstId {
    match ty {
        Ty::I8 => ctx.emit(
            Opcode::ConstU8,
            Ty::I8,
            vec![],
            InstData::ConstU8(value as u8),
            span,
        ),
        Ty::U8 => ctx.emit(
            Opcode::ConstU8,
            Ty::U8,
            vec![],
            InstData::ConstU8(value as u8),
            span,
        ),
        Ty::I16 | Ty::U16 | Ty::I32 => ctx.emit(
            Opcode::ConstI32,
            ty,
            vec![],
            InstData::ConstI32(value as i32),
            span,
        ),
        Ty::U32 => ctx.emit(
            Opcode::ConstI32,
            Ty::U32,
            vec![],
            InstData::ConstI32(value as u32 as i32),
            span,
        ),
        Ty::I128 => ctx.emit(
            Opcode::ConstI128,
            Ty::I128,
            vec![],
            InstData::ConstI128(value as i128),
            span,
        ),
        Ty::U128 => ctx.emit(
            Opcode::ConstU128,
            Ty::U128,
            vec![],
            InstData::ConstU128(value),
            span,
        ),
        _ => unreachable!("non-narrow integer context: {ty:?}"),
    }
}

fn emit_integer_zero(ctx: &mut LowerCtx, ty: Ty, span: Span) -> InstId {
    match ty {
        Ty::I8 | Ty::U8 | Ty::I16 | Ty::U16 | Ty::I32 | Ty::U32 | Ty::I128 | Ty::U128 => {
            emit_narrow_integer_constant(ctx, 0, ty, span)
        }
        Ty::U64 => ctx.emit(
            Opcode::ConstU64,
            Ty::U64,
            vec![],
            InstData::ConstU64(0),
            span,
        ),
        _ => ctx.emit(
            Opcode::ConstI64,
            Ty::I64,
            vec![],
            InstData::ConstI64(0),
            span,
        ),
    }
}

/// Re-lower a coercible integer marker in its contextual narrow type. This
/// preserves the original operators -- especially checked arithmetic --
/// instead of folding the whole expression into a wrapping constant.
fn lower_integer_marker_as(ctx: &mut LowerCtx, expr: &Expr, ty: Ty) -> Option<InstId> {
    match &expr.kind {
        ExprKind::Lit(Lit::Int(value)) => {
            Some(emit_narrow_integer_constant(ctx, *value, ty, expr.span))
        }
        ExprKind::UnaryOp {
            op: UnOp::Neg,
            operand,
        } => {
            let zero = emit_narrow_integer_constant(ctx, 0, ty, expr.span);
            let value = lower_integer_marker_as(ctx, operand, ty)?;
            Some(ctx.emit(
                Opcode::WrappingSub,
                ty,
                vec![zero, value],
                InstData::Integer(integer_type_for_ir_ty(ty)),
                expr.span,
            ))
        }
        ExprKind::BinaryOp { op, lhs, rhs } if expr_is_coercible_int_marker(expr) => {
            let lhs = lower_integer_marker_as(ctx, lhs, ty)?;
            let rhs_ty = if matches!(op, BinOp::Shl | BinOp::Shr) {
                Ty::U32
            } else {
                ty
            };
            let rhs = lower_integer_marker_as(ctx, rhs, rhs_ty)?;
            let (opcode, result_ty, data) = binop_opcode(*op, &ty);
            Some(ctx.emit(opcode, result_ty, vec![lhs, rhs], data, expr.span))
        }
        ExprKind::Block(block) => {
            lower_integer_marker_as(ctx, integer_marker_from_block(block)?, ty)
        }
        _ => None,
    }
}

/// Coerce an expression into an admitted contextual narrow integer type.
/// Marker expressions are re-lowered at that width so checked operations keep
/// their overflow behavior; control-flow results are explicitly reduced after
/// their Phi so later users observe the annotated type.
fn lower_narrow_literal(ctx: &mut LowerCtx, expr: &Expr, original: InstId, ty: Ty) -> InstId {
    if !matches!(
        ty,
        Ty::I8 | Ty::U8 | Ty::I16 | Ty::U16 | Ty::I32 | Ty::U32 | Ty::I128 | Ty::U128
    ) {
        return original;
    }
    if matches!(ty, Ty::I128 | Ty::U128)
        && ctx
            .wide_literal_contexts
            .get(&(expr as *const _ as usize))
            .is_some_and(|context_ty| *context_ty == ty)
    {
        return original;
    }
    if let Some(narrowed) = lower_integer_marker_as(ctx, expr, ty) {
        return narrowed;
    }
    let source_ty = ctx.inst_ty(original);
    if ir_ty_is_integer(source_ty) && source_ty != ty {
        return ctx.emit(
            Opcode::IntCast,
            ty,
            vec![original],
            InstData::IntegerCast {
                from: integer_type_for_ir_ty(source_ty),
                to: integer_type_for_ir_ty(ty),
            },
            expr.span,
        );
    }
    original
}

fn binop_opcode(op: BinOp, operand_ty: &Ty) -> (Opcode, Ty, InstData) {
    let result_ty = *operand_ty;
    let integer_data = InstData::Integer(integer_type_for_ir_ty(result_ty));
    match op {
        BinOp::Add => (Opcode::WrappingAdd, result_ty, integer_data),
        BinOp::Sub => (Opcode::WrappingSub, result_ty, integer_data),
        BinOp::Mul => (Opcode::WrappingMul, result_ty, integer_data),
        BinOp::Div => (Opcode::WrappingDiv, result_ty, integer_data),
        BinOp::Rem => (Opcode::WrappingRem, result_ty, integer_data),
        BinOp::AddChecked => (Opcode::CheckedAdd, result_ty, integer_data),
        BinOp::SubChecked => (Opcode::CheckedSub, result_ty, integer_data),
        BinOp::MulChecked => (Opcode::CheckedMul, result_ty, integer_data),
        BinOp::DivChecked => (Opcode::CheckedDiv, result_ty, integer_data),
        BinOp::RemChecked => (Opcode::CheckedRem, result_ty, integer_data),
        BinOp::Eq => (Opcode::Eq, Ty::Bool, integer_data),
        BinOp::Ne => (Opcode::Ne, Ty::Bool, integer_data),
        BinOp::Lt => (Opcode::Lt, Ty::Bool, integer_data),
        BinOp::Le => (Opcode::Le, Ty::Bool, integer_data),
        BinOp::Gt => (Opcode::Gt, Ty::Bool, integer_data),
        BinOp::Ge => (Opcode::Ge, Ty::Bool, integer_data),
        BinOp::And => (Opcode::And, Ty::Bool, InstData::None),
        BinOp::Or => (Opcode::Or, Ty::Bool, InstData::None),
        BinOp::BitAnd => (Opcode::BitAnd, result_ty, integer_data),
        BinOp::BitOr => (Opcode::BitOr, result_ty, integer_data),
        BinOp::BitXor => (Opcode::BitXor, result_ty, integer_data),
        BinOp::Shl => (Opcode::Shl, result_ty, integer_data),
        BinOp::Shr => (Opcode::Shr, result_ty, integer_data),
    }
}

fn backpatch_upsilon(ctx: &mut LowerCtx, block_id: BlockId, upsilon_id: InstId, phi_id: InstId) {
    let block_idx = block_id.0 as usize;
    let mut input = None;
    for inst in ctx.func.blocks[block_idx].insts.iter_mut() {
        if inst.id == upsilon_id {
            inst.data = InstData::PhiTarget(phi_id);
            input = inst.args.first().copied();
            break;
        }
    }
    if let Some(input) = input {
        ctx.link_phi_input(input, phi_id);
    }
}

fn lower_stmt(ctx: &mut LowerCtx, stmt: &Stmt) {
    match stmt {
        Stmt::Let {
            pattern, init, ty, ..
        } => {
            if let Some(expected) = ty {
                record_wide_expected_ast_context(ctx, init, expected);
            }
            if let Some(AstType::Named {
                name: type_name, ..
            }) = ty
                .as_ref()
                .map(|ann| resolve_type_alias(ann, &ctx.type_aliases))
            {
                let context_ty = match type_name.as_str() {
                    "i128" => Some(Ty::I128),
                    "u128" => Some(Ty::U128),
                    _ => None,
                };
                if let Some(context_ty) = context_ty {
                    record_wide_control_flow_context(ctx, init, context_ty);
                }
            }
            let mut val = lower_expr(ctx, init);
            let span = init.span;
            if let Some(AstType::Named {
                name: type_name, ..
            }) = ty
                .as_ref()
                .map(|ann| resolve_type_alias(ann, &ctx.type_aliases))
            {
                if type_name == "i8" {
                    val = lower_narrow_literal(ctx, init, val, Ty::I8);
                } else if type_name == "u8" {
                    val = lower_narrow_literal(ctx, init, val, Ty::U8);
                } else if type_name == "i16" {
                    val = lower_narrow_literal(ctx, init, val, Ty::I16);
                } else if type_name == "u16" {
                    val = lower_narrow_literal(ctx, init, val, Ty::U16);
                } else if type_name == "i32" {
                    val = lower_narrow_literal(ctx, init, val, Ty::I32);
                } else if type_name == "u32" {
                    val = lower_narrow_literal(ctx, init, val, Ty::U32);
                } else if type_name == "i128" {
                    val = lower_narrow_literal(ctx, init, val, Ty::I128);
                } else if type_name == "u128" {
                    val = lower_narrow_literal(ctx, init, val, Ty::U128);
                }
            }
            if let Some(AstType::Named {
                name: type_name, ..
            }) = ty
                .as_ref()
                .map(|ann| resolve_type_alias(ann, &ctx.type_aliases))
                && type_name == "u64"
                && ctx.inst_ty(val) != Ty::U64
            {
                let source_ty = ctx.inst_ty(val);
                val = ctx.emit(
                    Opcode::IntCast,
                    Ty::U64,
                    vec![val],
                    InstData::IntegerCast {
                        from: integer_type_for_ir_ty(source_ty),
                        to: IntegerType::U64,
                    },
                    span,
                );
            }
            if let PatKind::Ident { name, .. } = &pattern.kind {
                if let Some(ann) = ty {
                    let ann = resolve_type_alias(ann, &ctx.type_aliases);
                    match ann {
                        AstType::Named {
                            name: type_name, ..
                        } => match type_name.as_str() {
                            "i32" | "i64" | "u8" | "u64" | "f32" | "f64" | "bool" => {}
                            _ => {
                                ctx.inst_struct_type.insert(val, type_name.clone());
                            }
                        },
                        AstType::Generic {
                            name: type_name,
                            args,
                            ..
                        } => {
                            ctx.inst_struct_type.insert(val, type_name.clone());
                            if type_name == "Vec"
                                && let Some(elem_ty) = args.first()
                                && let AstType::Named {
                                    name: elem_name, ..
                                } = resolve_type_alias(elem_ty, &ctx.type_aliases)
                                && !matches!(
                                    elem_name.as_str(),
                                    "i32" | "i64" | "u8" | "u64" | "f32" | "f64" | "bool"
                                )
                            {
                                ctx.inst_vec_elem_types.insert(val, vec![elem_name.clone()]);
                            }
                        }
                        _ => {}
                    }
                }
                ctx.func.local_names.insert(val.0, name.clone());
                ctx.define(name.clone(), val);
            }
        }
        Stmt::Expr { expr, .. } => {
            lower_expr(ctx, expr);
        }
    }
}

fn lower_block(ctx: &mut LowerCtx, block: &Block) -> InstId {
    ctx.push_scope();
    let result = lower_block_inner(ctx, block);
    ctx.pop_scope();
    result
}

fn lower_block_inner(ctx: &mut LowerCtx, block: &Block) -> InstId {
    for stmt in &block.stmts {
        if ctx.is_terminated() {
            break;
        }
        lower_stmt(ctx, stmt);
    }
    if ctx.is_terminated() {
        // Block already terminated (e.g. by a return statement); no trailing expr.
        // Return a sentinel — callers that care will check is_terminated().
        InstId(u32::MAX)
    } else if let Some(expr) = &block.trailing_expr {
        lower_expr(ctx, expr)
    } else {
        ctx.emit(
            Opcode::ConstUnit,
            Ty::Unit,
            vec![],
            InstData::None,
            block.span,
        )
    }
}

#[allow(clippy::too_many_arguments)]
fn lower_function_with_pattern_aggregates(
    fn_def: &FnDef,
    file: &str,
    func_index: &HashMap<String, FuncSigInfo>,
    struct_field_map: HashMap<String, Vec<String>>,
    enum_variant_map: HashMap<String, Vec<String>>,
    enum_variant_payload_tys: HashMap<String, Vec<Vec<Ty>>>,
    enum_variant_payload_ast_types: Rc<HashMap<String, Vec<Vec<AstType>>>>,
    linear_owner_names: &HashSet<String>,
    type_aliases: Rc<HashMap<String, AstType>>,
    struct_field_type_names: HashMap<String, Vec<String>>,
    struct_field_ast_types: Rc<HashMap<String, Vec<AstType>>>,
    struct_field_vec_elems: HashMap<String, Vec<String>>,
    string_exprs: &StringExprSet,
    pattern_aggregates: &Rc<PatternAggregateMap>,
    const_map: &HashMap<String, (i64, Ty)>,
) -> (Function, Vec<String>, Vec<vow_diag::Diagnostic>) {
    let params: Vec<Ty> = fn_def
        .params
        .iter()
        .map(|p| lower_ty_with_linear(&p.ty, linear_owner_names, &type_aliases))
        .collect();
    let param_names: Vec<String> = fn_def.params.iter().map(|p| p.name.clone()).collect();
    let return_ty = lower_ty_with_linear(&fn_def.return_ty, linear_owner_names, &type_aliases);
    let effects = fn_def.effects.clone();

    let mut ctx = LowerCtx::new(
        fn_def.name.clone(),
        params.clone(),
        param_names,
        return_ty,
        effects,
        file.to_string(),
        func_index.clone(),
        struct_field_map,
        enum_variant_map,
        linear_owner_names.clone(),
        Rc::clone(&type_aliases),
        struct_field_type_names,
        struct_field_ast_types,
        struct_field_vec_elems,
        string_exprs.clone(),
        Rc::clone(pattern_aggregates),
    );

    ctx.enum_variant_payload_tys = enum_variant_payload_tys;
    ctx.enum_variant_payload_ast_types = enum_variant_payload_ast_types;
    ctx.func_return_ast_ty = Some(fn_def.return_ty.clone());
    ctx.const_map = const_map.clone();

    if let Some(vow) = &fn_def.vow {
        ctx.vow_block = Some(vow.clone());
    }

    for (idx, param) in fn_def.params.iter().enumerate() {
        let ty = params[idx];
        let arg_id = ctx.emit(
            Opcode::GetArg,
            ty,
            vec![],
            InstData::ArgIndex(idx as u32),
            fn_def.span,
        );
        match resolve_type_alias(&param.ty, &type_aliases) {
            AstType::Named { name, .. } if name == "str" || name == "String" => {
                ctx.inst_struct_type.insert(arg_id, "String".to_string());
            }
            AstType::Generic { name, .. } if name == "HashMap" => {
                ctx.inst_struct_type.insert(arg_id, "HashMap".to_string());
            }
            AstType::Generic { name, .. } if name == "BTreeMap" => {
                ctx.inst_struct_type.insert(arg_id, "BTreeMap".to_string());
            }
            AstType::Generic { name, args, .. } if name == "Vec" => {
                ctx.inst_struct_type.insert(arg_id, "Vec".to_string());
                if let Some(elem_ty) = args.first()
                    && let AstType::Named {
                        name: elem_name, ..
                    } = resolve_type_alias(elem_ty, &type_aliases)
                    && !matches!(
                        elem_name.as_str(),
                        "i32" | "i64" | "u64" | "f32" | "f64" | "bool"
                    )
                {
                    ctx.inst_vec_elem_types
                        .insert(arg_id, vec![elem_name.clone()]);
                }
            }
            AstType::Generic { name, .. } if name == "Option" => {
                ctx.inst_struct_type.insert(arg_id, "Option".to_string());
                if let Some(elem_ty) = option_named_elem_type(&param.ty, &type_aliases) {
                    ctx.inst_option_elem_ty.insert(arg_id, elem_ty);
                }
            }
            AstType::Named { name, .. } if ctx.struct_field_map.contains_key(name.as_str()) => {
                ctx.inst_struct_type.insert(arg_id, name.clone());
            }
            _ => {}
        }
        ctx.func.local_names.insert(arg_id.0, param.name.clone());
        ctx.define(param.name.clone(), arg_id);
    }

    vow::lower_param_refinements(&mut ctx, &fn_def.params);

    if let Some(vow_block) = &fn_def.vow {
        vow::lower_requires(&mut ctx, vow_block);
    }

    if let Some(expr) = integer_marker_from_block(&fn_def.body) {
        record_wide_expected_ast_context(&mut ctx, expr, &fn_def.return_ty);
        record_wide_control_flow_context(&mut ctx, expr, return_ty);
    }
    ctx.push_scope();
    let mut trailing = lower_block_inner(&mut ctx, &fn_def.body);
    ctx.pop_scope();

    if matches!(
        return_ty,
        Ty::I8 | Ty::U8 | Ty::I16 | Ty::U16 | Ty::I32 | Ty::U32 | Ty::I128 | Ty::U128
    ) && let Some(expr) = &fn_def.body.trailing_expr
    {
        trailing = lower_narrow_literal(&mut ctx, expr, trailing, return_ty);
    }

    let has_return = {
        let block_idx = ctx.current_block.0 as usize;
        ctx.func.blocks[block_idx]
            .insts
            .last()
            .is_some_and(|i| i.opcode.is_terminal())
    };

    if !has_return {
        let span = fn_def.body.span;
        if let Some(vow_block) = &fn_def.vow {
            vow::lower_ensures(&mut ctx, vow_block, trailing);
        }
        ctx.emit(
            Opcode::Return,
            Ty::Unit,
            vec![trailing],
            InstData::None,
            span,
        );
    }

    ctx.finish()
}

#[cfg(test)]
#[allow(clippy::too_many_arguments)]
fn lower_function(
    fn_def: &FnDef,
    file: &str,
    func_index: &HashMap<String, FuncSigInfo>,
    struct_field_map: HashMap<String, Vec<String>>,
    enum_variant_map: HashMap<String, Vec<String>>,
    linear_owner_names: &HashSet<String>,
    struct_field_type_names: HashMap<String, Vec<String>>,
    struct_field_vec_elems: HashMap<String, Vec<String>>,
    string_exprs: &StringExprSet,
    const_map: &HashMap<String, (i64, Ty)>,
) -> (Function, Vec<String>, Vec<vow_diag::Diagnostic>) {
    lower_function_with_pattern_aggregates(
        fn_def,
        file,
        func_index,
        struct_field_map,
        enum_variant_map,
        HashMap::new(),
        Rc::new(HashMap::new()),
        linear_owner_names,
        Rc::new(HashMap::new()),
        struct_field_type_names,
        Rc::new(HashMap::new()),
        struct_field_vec_elems,
        string_exprs,
        &Rc::new(PatternAggregateMap::new()),
        const_map,
    )
}

pub fn lower_module_with_pattern_aggregates(
    module: &AstModule,
    item_files: &[String],
    string_exprs: &StringExprSet,
    pattern_aggregates: PatternAggregateMap,
) -> Module {
    debug_assert_eq!(
        module.items.len(),
        item_files.len(),
        "item_files must be parallel to module.items"
    );
    let pattern_aggregates = Rc::new(pattern_aggregates);
    // Walk module.items keeping the original index so each retained FnDef
    // can be paired with its source-file path from `item_files`.
    let fn_items: Vec<(&FnDef, &str)> = module
        .items
        .iter()
        .enumerate()
        .filter_map(|(idx, item)| {
            if let Item::Fn(fn_def) = item
                && !fn_def.is_declaration
            {
                Some((fn_def, item_files[idx].as_str()))
            } else {
                None
            }
        })
        .collect();

    // A direct linear struct owns its obligation. Enums, Option, Result, and
    // aliases to those owners inherit linearity; references and collection
    // types deliberately do not propagate ownership.
    let linear_owner_names = collect_linear_owner_names(module);
    let type_aliases = Rc::new(collect_type_aliases(module));

    let func_index: HashMap<String, FuncSigInfo> = fn_items
        .iter()
        .enumerate()
        .map(|(idx, (fn_def, _))| {
            (
                fn_def.name.clone(),
                FuncSigInfo {
                    id: FuncId(idx as u32),
                    ret_ty: lower_ty_with_linear(
                        &fn_def.return_ty,
                        &linear_owner_names,
                        &type_aliases,
                    ),
                    ret_tag: non_scalar_type_tag(&fn_def.return_ty, &type_aliases),
                    ret_vec_elem: vec_named_elem_type(&fn_def.return_ty, &type_aliases),
                    ret_option_elem: option_named_elem_type(&fn_def.return_ty, &type_aliases),
                    param_tys: fn_def
                        .params
                        .iter()
                        .map(|p| lower_ty_with_linear(&p.ty, &linear_owner_names, &type_aliases))
                        .collect(),
                    param_ast_tys: fn_def.params.iter().map(|p| p.ty.clone()).collect(),
                },
            )
        })
        .collect();

    // Collect const declarations
    let mut const_map: HashMap<String, (i64, Ty)> = HashMap::new();
    for item in &module.items {
        if let Item::Const(c) = item {
            let val = match &c.value.kind {
                ExprKind::Lit(Lit::Int(v)) => *v as i64,
                ExprKind::Lit(Lit::Bool(b)) => *b as i64,
                ExprKind::UnaryOp {
                    op: UnOp::Neg,
                    operand,
                } => {
                    if let ExprKind::Lit(Lit::Int(v)) = &operand.kind {
                        -(*v as i64)
                    } else {
                        0
                    }
                }
                _ => 0,
            };
            let ty = lower_ty_with_linear(&c.ty, &linear_owner_names, &type_aliases);
            const_map.insert(c.name.clone(), (val, ty));
        }
    }

    // Build struct layout info
    let mut struct_field_map: HashMap<String, Vec<String>> = HashMap::new();
    let mut struct_layouts: Vec<StructLayout> = Vec::new();
    for item in &module.items {
        if let Item::Struct(s) = item {
            let field_names: Vec<String> = s.fields.iter().map(|f| f.name.clone()).collect();
            let field_layouts: Vec<FieldLayout> = s
                .fields
                .iter()
                .map(|f| FieldLayout {
                    name: f.name.clone(),
                    ty: lower_ty_with_linear(&f.ty, &linear_owner_names, &type_aliases),
                })
                .collect();
            struct_field_map.insert(s.name.clone(), field_names);
            struct_layouts.push(StructLayout {
                name: s.name.clone(),
                fields: field_layouts,
                is_linear: s.is_linear,
            });
        }
    }

    // Build struct field type names for FieldGet auto-tagging
    let mut struct_field_type_names: HashMap<String, Vec<String>> = HashMap::new();
    let mut struct_field_ast_types: HashMap<String, Vec<AstType>> = HashMap::new();
    // struct name → per-field Vec element type name (empty if not Vec<Named>)
    let mut struct_field_vec_elems: HashMap<String, Vec<String>> = HashMap::new();
    for item in &module.items {
        if let Item::Struct(s) = item {
            let type_names: Vec<String> = s
                .fields
                .iter()
                .map(|f| type_tag_name(&f.ty, &type_aliases))
                .collect();
            let vec_elems: Vec<String> = s
                .fields
                .iter()
                .map(|f| match resolve_type_alias(&f.ty, &type_aliases) {
                    AstType::Generic { name, args, .. } if name == "Vec" => {
                        if let Some(elem_ty) = args.first()
                            && let AstType::Named {
                                name: elem_name, ..
                            } = resolve_type_alias(elem_ty, &type_aliases)
                            && !matches!(
                                elem_name.as_str(),
                                "i32" | "i64" | "u64" | "f32" | "f64" | "bool"
                            )
                        {
                            return elem_name.clone();
                        }
                        String::new()
                    }
                    _ => String::new(),
                })
                .collect();
            struct_field_type_names.insert(s.name.clone(), type_names);
            struct_field_ast_types.insert(
                s.name.clone(),
                s.fields.iter().map(|field| field.ty.clone()).collect(),
            );
            struct_field_vec_elems.insert(s.name.clone(), vec_elems);
        }
    }
    let struct_field_ast_types = Rc::new(struct_field_ast_types);

    // Build enum layout info
    let mut enum_variant_map: HashMap<String, Vec<String>> = HashMap::new();
    let mut enum_variant_payload_tys: HashMap<String, Vec<Vec<Ty>>> = HashMap::new();
    let mut enum_variant_payload_ast_types: HashMap<String, Vec<Vec<AstType>>> = HashMap::new();
    let mut enum_layouts: Vec<EnumLayout> = Vec::new();
    for item in &module.items {
        if let Item::Enum(e) = item {
            let variant_names: Vec<String> = e.variants.iter().map(|v| v.name.clone()).collect();
            let variant_layouts: Vec<VariantLayout> = e
                .variants
                .iter()
                .enumerate()
                .map(|(tag, v)| {
                    let payload: Vec<FieldLayout> = match &v.kind {
                        VariantKind::Unit => vec![],
                        VariantKind::Tuple(tys) => tys
                            .iter()
                            .enumerate()
                            .map(|(i, ty)| FieldLayout {
                                name: i.to_string(),
                                ty: lower_ty_with_linear(ty, &linear_owner_names, &type_aliases),
                            })
                            .collect(),
                        VariantKind::Struct(fields) => fields
                            .iter()
                            .map(|f| FieldLayout {
                                name: f.name.clone(),
                                ty: lower_ty_with_linear(&f.ty, &linear_owner_names, &type_aliases),
                            })
                            .collect(),
                    };
                    VariantLayout {
                        name: v.name.clone(),
                        tag: tag as u64,
                        payload,
                    }
                })
                .collect();
            let payload_tys = variant_layouts
                .iter()
                .map(|variant| variant.payload.iter().map(|field| field.ty).collect())
                .collect();
            let payload_ast_types = e
                .variants
                .iter()
                .map(|variant| match &variant.kind {
                    VariantKind::Unit => vec![],
                    VariantKind::Tuple(types) => types.clone(),
                    VariantKind::Struct(fields) => {
                        fields.iter().map(|field| field.ty.clone()).collect()
                    }
                })
                .collect();
            enum_variant_map.insert(e.name.clone(), variant_names);
            enum_variant_payload_tys.insert(e.name.clone(), payload_tys);
            enum_variant_payload_ast_types.insert(e.name.clone(), payload_ast_types);
            enum_layouts.push(EnumLayout {
                name: e.name.clone(),
                variants: variant_layouts,
            });
        }
    }
    let enum_variant_payload_ast_types = Rc::new(enum_variant_payload_ast_types);

    let mut all_strings: Vec<String> = Vec::new();
    let mut all_warnings: Vec<vow_diag::Diagnostic> = Vec::new();
    let functions: Vec<Function> = fn_items
        .iter()
        .enumerate()
        .map(|(idx, (fn_def, src_file))| {
            let (mut func, pool, func_warnings) = lower_function_with_pattern_aggregates(
                fn_def,
                src_file,
                &func_index,
                struct_field_map.clone(),
                enum_variant_map.clone(),
                enum_variant_payload_tys.clone(),
                Rc::clone(&enum_variant_payload_ast_types),
                &linear_owner_names,
                Rc::clone(&type_aliases),
                struct_field_type_names.clone(),
                Rc::clone(&struct_field_ast_types),
                struct_field_vec_elems.clone(),
                string_exprs,
                &pattern_aggregates,
                &const_map,
            );
            func.id = FuncId(idx as u32);
            let base = all_strings.len() as u32;
            if base > 0 || !pool.is_empty() {
                for block in &mut func.blocks {
                    for inst in &mut block.insts {
                        if let InstData::ConstStr(ref mut i) = inst.data {
                            *i += base;
                        }
                    }
                }
            }
            all_strings.extend(pool);
            all_warnings.extend(func_warnings);
            func
        })
        .collect();

    Module {
        name: module.name.clone(),
        strings: all_strings,
        struct_layouts,
        enum_layouts,
        functions,
        warnings: all_warnings,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use vow_syntax::ast::{
        Block, Effect, Expr, ExprKind, FnDef, Lit, MatchArm, Pat, PatKind, Stmt, Type, Visibility,
        VowBlock, VowClause,
    };
    use vow_syntax::span::Span;

    fn sp() -> Span {
        Span::new(0, 1)
    }

    fn unit_ty() -> Type {
        Type::Unit { span: sp() }
    }

    fn i64_ty() -> Type {
        Type::Named {
            name: "i64".to_string(),
            span: sp(),
        }
    }

    fn u64_ty() -> Type {
        Type::Named {
            name: "u64".to_string(),
            span: sp(),
        }
    }

    fn u8_ty() -> Type {
        Type::Named {
            name: "u8".to_string(),
            span: sp(),
        }
    }

    fn named_ty(name: &str) -> Type {
        Type::Named {
            name: name.to_string(),
            span: sp(),
        }
    }

    fn string_ty() -> Type {
        Type::Named {
            name: "String".to_string(),
            span: sp(),
        }
    }

    fn option_ty(elem: Type) -> Type {
        Type::Generic {
            name: "Option".to_string(),
            args: vec![elem],
            span: sp(),
        }
    }

    fn int_expr(v: u128) -> Expr {
        Expr {
            kind: ExprKind::Lit(Lit::Int(v)),
            span: sp(),
        }
    }

    fn string_expr(v: &str) -> Expr {
        Expr {
            kind: ExprKind::Lit(Lit::String(v.to_string())),
            span: sp(),
        }
    }

    fn bool_expr(v: bool) -> Expr {
        Expr {
            kind: ExprKind::Lit(Lit::Bool(v)),
            span: sp(),
        }
    }

    fn string_from_expr(arg: Expr) -> Expr {
        Expr {
            kind: ExprKind::EnumConstruct {
                path: vec!["String".to_string(), "from".to_string()],
                fields: vec![arg],
            },
            span: sp(),
        }
    }

    fn ident_expr(name: &str) -> Expr {
        Expr {
            kind: ExprKind::Ident(name.to_string()),
            span: sp(),
        }
    }

    fn call_expr(callee: &str, args: Vec<Expr>) -> Expr {
        Expr {
            kind: ExprKind::Call {
                callee: Box::new(ident_expr(callee)),
                args,
            },
            span: sp(),
        }
    }

    fn empty_block() -> Block {
        Block {
            stmts: vec![],
            trailing_expr: None,
            span: sp(),
        }
    }

    fn make_fn(
        name: &str,
        params: Vec<vow_syntax::ast::Param>,
        return_ty: Type,
        body: Block,
        effects: Vec<Effect>,
    ) -> FnDef {
        FnDef {
            vis: Visibility::Public,
            name: name.to_string(),
            params,
            return_ty,
            effects,
            vow: None,
            body,
            span: sp(),
            is_declaration: false,
        }
    }

    fn make_param(name: &str, ty: Type) -> vow_syntax::ast::Param {
        vow_syntax::ast::Param {
            name: name.to_string(),
            ty,
            refinement: None,
            span: sp(),
        }
    }

    #[test]
    fn debug_builtins_lower_to_runtime_symbols() {
        let cases = [
            ("debug_str", "__vow_debug_str", Ty::Unit),
            ("debug_i64", "__vow_debug_i64", Ty::Unit),
            ("debug_u64", "__vow_debug_u64", Ty::Unit),
        ];
        for (name, symbol, ty) in cases {
            assert_eq!(vow_debug_builtin_to_runtime(name), Some((symbol, ty)));
        }
        assert_eq!(vow_debug_builtin_to_runtime("debug_missing"), None);
    }

    #[test]
    fn type_alias_chain_to_linear_option_is_a_linear_owner() {
        let source = r#"
module LinearAlias

linear struct Token {
    id: i64,
}

type MaybeToken = Option<Token>;
type ForwardedToken = MaybeToken;
"#;
        let (ast, diagnostics) = vow_syntax::parser::parse_module(source, "linear_alias.vow");
        assert!(diagnostics.is_empty(), "{diagnostics:?}");

        let owners = collect_linear_owner_names(&ast);
        assert!(owners.contains("Token"));
        assert!(owners.contains("MaybeToken"));
        assert!(owners.contains("ForwardedToken"));
    }

    #[test]
    fn type_aliases_are_resolved_transitively_before_lowering() {
        let source = r#"
module NonLinearAlias

type Small = u8;
type Tiny = Small;

struct Pair {
    left: u8,
    right: u8,
}

type PairAlias = Pair;
type PairView = PairAlias;
"#;
        let (ast, diagnostics) = vow_syntax::parser::parse_module(source, "non_linear_alias.vow");
        assert!(diagnostics.is_empty(), "{diagnostics:?}");

        let aliases = collect_type_aliases(&ast);
        let tiny = Type::Named {
            name: "Tiny".to_string(),
            span: sp(),
        };
        let pair_view = Type::Named {
            name: "PairView".to_string(),
            span: sp(),
        };

        assert_eq!(
            lower_ty_with_linear(&tiny, &HashSet::new(), &aliases),
            Ty::U8
        );
        assert_eq!(
            non_scalar_type_tag(&pair_view, &aliases),
            Some("Pair".to_string())
        );
    }

    #[test]
    fn builtins_lower_to_runtime_symbols_and_return_types() {
        // Keep this table in lockstep with every arm of vow_builtin_to_runtime.
        let cases = [
            ("print_str", "__vow_string_print", Ty::Unit),
            ("print_i64", "__vow_print_i64", Ty::Unit),
            ("print_u64", "__vow_print_u64", Ty::Unit),
            ("eprintln_str", "__vow_eprintln_str", Ty::Unit),
            ("fs_read", "__vow_fs_read", Ty::Ptr),
            ("fs_open", "__vow_fs_open", Ty::I64),
            ("fs_read_line", "__vow_fs_read_line", Ty::Ptr),
            ("fs_status", "__vow_fs_status", Ty::I64),
            ("fs_close", "__vow_fs_close", Ty::I64),
            ("fs_write", "__vow_fs_write", Ty::I64),
            ("fs_exists", "__vow_fs_exists", Ty::I64),
            ("fs_mkdir", "__vow_fs_mkdir", Ty::I64),
            ("fs_listdir", "__vow_fs_listdir", Ty::Ptr),
            ("fs_remove", "__vow_fs_remove", Ty::I64),
            ("fs_remove_dir", "__vow_fs_remove_dir", Ty::I64),
            ("fs_is_dir", "__vow_fs_is_dir", Ty::I64),
            ("fs_is_symlink", "__vow_fs_is_symlink", Ty::I64),
            ("fs_rename", "__vow_fs_rename", Ty::I64),
            ("string_substr", "__vow_string_substr", Ty::Ptr),
            ("string_split", "__vow_string_split", Ty::Ptr),
            ("string_starts_with", "__vow_string_starts_with", Ty::I64),
            ("string_ends_with", "__vow_string_ends_with", Ty::I64),
            ("string_trim", "__vow_string_trim", Ty::Ptr),
            ("string_to_upper", "__vow_string_to_upper", Ty::Ptr),
            ("string_to_lower", "__vow_string_to_lower", Ty::Ptr),
            ("string_replace", "__vow_string_replace", Ty::Ptr),
            ("string_join", "__vow_string_join", Ty::Ptr),
            ("parse_i64", "__vow_string_parse_i64_opt", Ty::Ptr),
            ("int_to_string", "__vow_string_from_i64", Ty::Ptr),
            ("uint_to_string", "__vow_string_from_u64", Ty::Ptr),
            ("i64_to_string", "__vow_string_from_i64", Ty::Ptr),
            ("vec_sort", "__vow_vec_sort", Ty::Ptr),
            ("time_unix", "__vow_time_unix", Ty::I64),
            ("time_unix_ms", "__vow_time_unix_ms", Ty::I64),
            ("num_cpus", "__vow_num_cpus", Ty::I64),
            (
                "memory_root_arena_bytes",
                "__vow_memory_root_arena_bytes",
                Ty::U64,
            ),
            ("memory_peak_bytes", "__vow_memory_peak_bytes", Ty::U64),
            (
                "memory_alloc_count_since_start",
                "__vow_memory_alloc_count_since_start",
                Ty::U64,
            ),
            ("time_micros", "__vow_time_micros", Ty::I64),
            ("proc_sample", "__vow_proc_sample", Ty::Ptr),
            ("gzip_write_file", "__vow_gzip_write_file", Ty::I64),
            ("hex_encode", "__vow_hex_encode", Ty::Ptr),
            ("hex_decode", "__vow_hex_decode", Ty::Ptr),
            ("args", "__vow_args", Ty::Ptr),
            ("stdin_read", "__vow_stdin_read", Ty::Ptr),
            ("stdin_read_line", "__vow_stdin_read_line", Ty::Ptr),
            ("stdin_ready", "__vow_stdin_ready", Ty::Bool),
            ("process_exit", "__vow_process_exit", Ty::Unit),
            ("process_run", "__vow_process_run", Ty::I64),
            ("process_get_stdout", "__vow_process_get_stdout", Ty::Ptr),
            ("process_get_stderr", "__vow_process_get_stderr", Ty::Ptr),
            ("process_start", "__vow_process_start", Ty::I64),
            ("process_wait", "__vow_process_wait", Ty::I64),
            (
                "process_wait_timeout",
                "__vow_process_wait_timeout",
                Ty::I64,
            ),
            ("process_poll_wait", "__vow_process_poll_wait", Ty::I64),
            ("process_kill", "__vow_process_kill", Ty::I64),
            ("process_stdout_for", "__vow_process_stdout_for", Ty::Ptr),
            ("process_stderr_for", "__vow_process_stderr_for", Ty::Ptr),
            ("__vow_clif_create", "__vow_clif_create", Ty::I64),
            ("__vow_clif_add_string", "__vow_clif_add_string", Ty::Unit),
            (
                "__vow_clif_declare_extern",
                "__vow_clif_declare_extern",
                Ty::Unit,
            ),
            (
                "__vow_clif_declare_function",
                "__vow_clif_declare_function",
                Ty::Unit,
            ),
            ("__vow_clif_fn_begin", "__vow_clif_fn_begin", Ty::I64),
            ("__vow_clif_fn_block", "__vow_clif_fn_block", Ty::I64),
            ("__vow_clif_fn_inst", "__vow_clif_fn_inst", Ty::I64),
            ("__vow_clif_fn_vow", "__vow_clif_fn_vow", Ty::I64),
            ("__vow_clif_fn_end", "__vow_clif_fn_end", Ty::I64),
            ("__vow_clif_finish", "__vow_clif_finish", Ty::I64),
            ("__vow_clif_link", "__vow_clif_link", Ty::I64),
            ("__vow_clif_destroy", "__vow_clif_destroy", Ty::Unit),
        ];
        for (name, symbol, ty) in cases {
            assert_eq!(vow_builtin_to_runtime(name), Some((symbol.to_string(), ty)));
        }
        assert_eq!(vow_builtin_to_runtime("missing_builtin"), None);
    }

    #[test]
    fn phase3_narrowing_builtins_lower_only_supported_pairs() {
        let cases = [
            ("i16_to_i8_try", "__vow_i16_to_i8_try", Ty::Ptr),
            ("u64_to_i8_wrap", "__vow_u64_to_i8_wrap", Ty::I8),
            ("i32_to_i16_sat", "__vow_i32_to_i16_sat", Ty::I16),
            ("u64_to_u16_try", "__vow_u64_to_u16_try", Ty::Ptr),
            ("i64_to_u32_wrap", "__vow_i64_to_u32_wrap", Ty::U32),
        ];
        for (name, symbol, ty) in cases {
            assert_eq!(vow_builtin_to_runtime(name), Some((symbol.to_string(), ty)));
        }

        for name in [
            "i8_to_i8_try",
            "i16_to_u32_wrap",
            "i64_to_i8_checked",
            "not_a_conversion",
        ] {
            assert_eq!(narrow_intrinsic_target(name), None, "{name}");
        }
    }

    #[test]
    fn phase3_parser_calls_preserve_runtime_symbols() {
        let mut stmts: Vec<Stmt> = ["parse_i8", "parse_i16", "parse_u16", "parse_u32"]
            .into_iter()
            .map(|name| Stmt::Expr {
                expr: call_expr(name, vec![string_expr("0")]),
                has_semicolon: true,
                span: sp(),
            })
            .collect();
        stmts.push(Stmt::Expr {
            expr: call_expr("i16_to_i8_try", vec![int_expr(0)]),
            has_semicolon: true,
            span: sp(),
        });
        let fn_def = make_fn(
            "parse_all_narrow",
            vec![],
            unit_ty(),
            Block {
                stmts,
                trailing_expr: None,
                span: sp(),
            },
            vec![],
        );
        let (func, _, warnings) = lower_function(
            &fn_def,
            "test.vow",
            &HashMap::new(),
            HashMap::new(),
            HashMap::new(),
            &HashSet::new(),
            HashMap::new(),
            HashMap::new(),
            &HashSet::new(),
            &HashMap::new(),
        );

        assert!(warnings.is_empty(), "{warnings:?}");
        let symbols: Vec<&str> = func
            .blocks
            .iter()
            .flat_map(|block| &block.insts)
            .filter_map(|inst| match &inst.data {
                InstData::CallExtern(symbol) => Some(symbol.as_str()),
                _ => None,
            })
            .collect();
        for symbol in [
            "__vow_string_parse_i8_opt",
            "__vow_string_parse_i16_opt",
            "__vow_string_parse_u16_opt",
            "__vow_string_parse_u32_opt",
            "__vow_i16_to_i8_try",
        ] {
            assert!(symbols.contains(&symbol), "missing {symbol}: {symbols:?}");
        }
    }

    #[test]
    fn integer_literals_lower_at_their_native_ir_width() {
        let cases = [
            ("i8", 7, Ty::I8, Opcode::ConstU8, InstData::ConstU8(7)),
            ("i16", 7, Ty::I16, Opcode::ConstI32, InstData::ConstI32(7)),
            ("u16", 7, Ty::U16, Opcode::ConstI32, InstData::ConstI32(7)),
            ("u32", 7, Ty::U32, Opcode::ConstI32, InstData::ConstI32(7)),
            (
                "i128",
                i128::MAX as u128,
                Ty::I128,
                Opcode::ConstI128,
                InstData::ConstI128(i128::MAX),
            ),
            (
                "u128",
                u128::MAX,
                Ty::U128,
                Opcode::ConstU128,
                InstData::ConstU128(u128::MAX),
            ),
        ];

        for (name, value, expected_ty, expected_op, expected_data) in cases {
            let fn_def = make_fn(
                &format!("return_{name}"),
                vec![],
                named_ty(name),
                Block {
                    stmts: vec![Stmt::Let {
                        pattern: Pat {
                            kind: PatKind::Ident {
                                name: "local".to_string(),
                                is_mut: false,
                            },
                            span: sp(),
                        },
                        ty: Some(named_ty(name)),
                        init: Box::new(int_expr(value)),
                        span: sp(),
                    }],
                    trailing_expr: Some(Box::new(int_expr(value))),
                    span: sp(),
                },
                vec![],
            );
            let (func, _, warnings) = lower_function(
                &fn_def,
                "test.vow",
                &HashMap::new(),
                HashMap::new(),
                HashMap::new(),
                &HashSet::new(),
                HashMap::new(),
                HashMap::new(),
                &HashSet::new(),
                &HashMap::new(),
            );

            assert!(warnings.is_empty(), "{name}: {warnings:?}");
            assert_eq!(func.return_ty, expected_ty, "{name}");
            assert!(
                func.blocks
                    .iter()
                    .flat_map(|block| &block.insts)
                    .any(|inst| {
                        inst.opcode == expected_op
                            && inst.ty == expected_ty
                            && inst.data == expected_data
                    }),
                "missing native {name} constant in {func:#?}"
            );
        }
    }

    #[test]
    fn explicit_wide_suffixes_lower_through_the_parser_to_native_constants() {
        let source = r#"
module WideSuffixLowering

fn signed_max() -> i128 {
    170141183460469231731687303715884105727i128
}

fn signed_min() -> i128 {
    -170141183460469231731687303715884105728i128
}

fn unsigned_max() -> u128 {
    340282366920938463463374607431768211455u128
}
"#;
        let (ast, diagnostics) = vow_syntax::parser::parse_module(source, "wide_suffixes.vow");
        assert!(diagnostics.is_empty(), "{diagnostics:?}");

        let item_files = vec!["wide_suffixes.vow".to_string(); ast.items.len()];
        let module = lower_module_with_pattern_aggregates(
            &ast,
            &item_files,
            &StringExprSet::new(),
            PatternAggregateMap::new(),
        );

        let signed_max = &module.functions[0];
        assert!(
            signed_max
                .blocks
                .iter()
                .flat_map(|block| &block.insts)
                .any(|inst| inst.opcode == Opcode::ConstI128
                    && inst.ty == Ty::I128
                    && inst.data == InstData::ConstI128(i128::MAX))
        );

        let signed_min = &module.functions[1];
        let signed_min_insts: Vec<_> = signed_min
            .blocks
            .iter()
            .flat_map(|block| &block.insts)
            .collect();
        assert!(signed_min_insts.iter().any(|inst| {
            inst.opcode == Opcode::ConstI128
                && inst.ty == Ty::I128
                && inst.data == InstData::ConstI128(i128::MIN)
        }));
        assert!(signed_min_insts.iter().any(|inst| {
            inst.opcode == Opcode::WrappingSub
                && inst.ty == Ty::I128
                && inst.data == InstData::Integer(IntegerType::I128)
        }));

        let unsigned_max = &module.functions[2];
        assert!(
            unsigned_max
                .blocks
                .iter()
                .flat_map(|block| &block.insts)
                .any(|inst| inst.opcode == Opcode::ConstU128
                    && inst.ty == Ty::U128
                    && inst.data == InstData::ConstU128(u128::MAX))
        );
    }

    #[test]
    fn checked_marker_return_keeps_narrow_overflow_operation() {
        let checked_add = Expr {
            kind: ExprKind::BinaryOp {
                op: BinOp::AddChecked,
                lhs: Box::new(int_expr(127)),
                rhs: Box::new(int_expr(1)),
            },
            span: sp(),
        };
        let fn_def = make_fn(
            "checked_i8",
            vec![],
            named_ty("i8"),
            Block {
                stmts: vec![],
                trailing_expr: Some(Box::new(checked_add)),
                span: sp(),
            },
            vec![],
        );
        let (func, _, warnings) = lower_function(
            &fn_def,
            "test.vow",
            &HashMap::new(),
            HashMap::new(),
            HashMap::new(),
            &HashSet::new(),
            HashMap::new(),
            HashMap::new(),
            &HashSet::new(),
            &HashMap::new(),
        );

        assert!(warnings.is_empty(), "{warnings:?}");
        let all_insts: Vec<_> = func.blocks.iter().flat_map(|b| b.insts.iter()).collect();
        let checked = all_insts
            .iter()
            .find(|inst| inst.opcode == Opcode::CheckedAdd && inst.ty == Ty::I8)
            .expect("contextual i8 checked add");
        assert_eq!(checked.data, InstData::Integer(IntegerType::I8));
        let ret = all_insts
            .iter()
            .find(|inst| inst.opcode == Opcode::Return)
            .expect("return");
        assert_eq!(ret.args, vec![checked.id]);
    }

    #[test]
    fn annotated_narrow_local_reduces_control_flow_result() {
        let init = Expr {
            kind: ExprKind::If {
                condition: Box::new(bool_expr(true)),
                then_branch: Box::new(Block {
                    stmts: vec![],
                    trailing_expr: Some(Box::new(int_expr(127))),
                    span: sp(),
                }),
                else_branch: Some(Box::new(int_expr(0))),
            },
            span: sp(),
        };
        let fn_def = make_fn(
            "local_i8",
            vec![],
            named_ty("i8"),
            Block {
                stmts: vec![Stmt::Let {
                    pattern: Pat {
                        kind: PatKind::Ident {
                            name: "x".to_string(),
                            is_mut: false,
                        },
                        span: sp(),
                    },
                    ty: Some(named_ty("i8")),
                    init: Box::new(init),
                    span: sp(),
                }],
                trailing_expr: Some(Box::new(ident_expr("x"))),
                span: sp(),
            },
            vec![],
        );
        let (func, _, warnings) = lower_function(
            &fn_def,
            "test.vow",
            &HashMap::new(),
            HashMap::new(),
            HashMap::new(),
            &HashSet::new(),
            HashMap::new(),
            HashMap::new(),
            &HashSet::new(),
            &HashMap::new(),
        );

        assert!(warnings.is_empty(), "{warnings:?}");
        let all_insts: Vec<_> = func.blocks.iter().flat_map(|b| b.insts.iter()).collect();
        let cast = all_insts
            .iter()
            .find(|inst| {
                inst.opcode == Opcode::IntCast
                    && inst.ty == Ty::I8
                    && inst.data
                        == InstData::IntegerCast {
                            from: IntegerType::I64,
                            to: IntegerType::I8,
                        }
            })
            .expect("i64 Phi reduced to annotated i8");
        let ret = all_insts
            .iter()
            .find(|inst| inst.opcode == Opcode::Return)
            .expect("return");
        assert_eq!(ret.args, vec![cast.id]);
    }

    #[test]
    fn integer_binary_literals_follow_the_typed_operand() {
        for (name, expected_ty) in [
            ("i8", Ty::I8),
            ("i16", Ty::I16),
            ("u16", Ty::U16),
            ("u32", Ty::U32),
            ("i128", Ty::I128),
            ("u128", Ty::U128),
        ] {
            let sum = Expr {
                kind: ExprKind::BinaryOp {
                    op: BinOp::Add,
                    lhs: Box::new(ident_expr("value")),
                    rhs: Box::new(int_expr(1)),
                },
                span: sp(),
            };
            let fn_def = make_fn(
                &format!("add_{name}"),
                vec![make_param("value", named_ty(name))],
                named_ty(name),
                Block {
                    stmts: vec![],
                    trailing_expr: Some(Box::new(sum)),
                    span: sp(),
                },
                vec![],
            );
            let (func, _, warnings) = lower_function(
                &fn_def,
                "test.vow",
                &HashMap::new(),
                HashMap::new(),
                HashMap::new(),
                &HashSet::new(),
                HashMap::new(),
                HashMap::new(),
                &HashSet::new(),
                &HashMap::new(),
            );

            assert!(warnings.is_empty(), "{name}: {warnings:?}");
            let add = func
                .blocks
                .iter()
                .flat_map(|block| &block.insts)
                .find(|inst| inst.opcode == Opcode::WrappingAdd)
                .expect("wrapping add");
            assert_eq!(add.ty, expected_ty, "{name}");
            assert_eq!(
                add.data,
                InstData::Integer(integer_type_for_ir_ty(expected_ty)),
                "{name}"
            );
        }
    }

    #[test]
    fn free_parse_i64_lowers_an_option_i64_result() {
        let source = r#"
module ParseI64Lowering

fn parse_or_default(s: String) -> i64 {
    match parse_i64(s) {
        Option::Some(value) => value,
        Option::None => 0,
    }
}
"#;
        let (ast, diagnostics) = vow_syntax::parser::parse_module(source, "parse_i64.vow");
        assert!(diagnostics.is_empty(), "{diagnostics:?}");

        let item_files = vec!["parse_i64.vow".to_string(); ast.items.len()];
        let module = lower_module_with_pattern_aggregates(
            &ast,
            &item_files,
            &StringExprSet::new(),
            PatternAggregateMap::new(),
        );
        let instructions: Vec<&Inst> = module.functions[0]
            .blocks
            .iter()
            .flat_map(|block| &block.insts)
            .collect();
        let parse_call = instructions
            .iter()
            .find(|inst| {
                inst.data == InstData::CallExtern("__vow_string_parse_i64_opt".to_string())
            })
            .expect("free parse_i64 must use the option-returning runtime entry point");

        assert_eq!(parse_call.ty, Ty::Ptr);
        assert!(instructions.iter().any(|inst| {
            inst.opcode == Opcode::FieldGet
                && inst.ty == Ty::I64
                && inst.args == vec![parse_call.id]
                && inst.data == InstData::FieldIndex(1)
        }));
    }

    #[test]
    fn string_matches_literal_at_lowers_literal_without_allocation() {
        let body = Block {
            stmts: vec![],
            trailing_expr: Some(Box::new(call_expr(
                "string_matches_literal_at",
                vec![ident_expr("s"), ident_expr("pos"), string_expr("ab\0cd")],
            ))),
            span: sp(),
        };
        let fn_def = make_fn(
            "matches_literal",
            vec![make_param("s", string_ty()), make_param("pos", i64_ty())],
            i64_ty(),
            body,
            vec![],
        );
        let (func, strings, warnings) = lower_function(
            &fn_def,
            "",
            &HashMap::new(),
            HashMap::new(),
            HashMap::new(),
            &HashSet::new(),
            HashMap::new(),
            HashMap::new(),
            &HashSet::new(),
            &HashMap::new(),
        );

        assert!(warnings.is_empty(), "unexpected warnings: {warnings:?}");
        assert_eq!(strings, vec!["ab\0cd".to_string()]);

        let insts: Vec<&Inst> = func
            .blocks
            .iter()
            .flat_map(|block| block.insts.iter())
            .collect();
        assert!(
            insts.iter().any(|inst| {
                inst.opcode == Opcode::Call
                    && inst.data
                        == InstData::CallExtern("__vow_string_matches_literal_at".to_string())
            }),
            "expected direct runtime helper call in {insts:#?}"
        );
        assert!(
            !insts.iter().any(|inst| {
                inst.opcode == Opcode::Call
                    && inst.data == InstData::CallExtern("__vow_string_from_cstr".to_string())
            }),
            "literal matcher must not allocate a temporary String"
        );
        assert!(
            insts.iter().any(|inst| {
                inst.opcode == Opcode::ConstStr && inst.data == InstData::ConstStr(0)
            }),
            "expected static literal pointer"
        );
        assert!(
            insts.iter().any(|inst| {
                inst.opcode == Opcode::ConstI64 && inst.data == InstData::ConstI64(5)
            }),
            "expected byte length, including embedded NUL"
        );
    }

    #[test]
    fn string_literal_lowers_to_static_descriptor_call() {
        let body = Block {
            stmts: vec![],
            trailing_expr: Some(Box::new(string_expr("hello"))),
            span: sp(),
        };
        let fn_def = make_fn("literal", vec![], string_ty(), body, vec![]);
        let (func, pool, diags) = lower_function(
            &fn_def,
            "test.vow",
            &HashMap::new(),
            HashMap::new(),
            HashMap::new(),
            &HashSet::new(),
            HashMap::new(),
            HashMap::new(),
            &HashSet::new(),
            &HashMap::new(),
        );

        assert!(diags.is_empty(), "{diags:?}");
        assert_eq!(pool, vec!["hello".to_string()]);
        let extern_calls: Vec<&str> = func
            .blocks
            .iter()
            .flat_map(|block| &block.insts)
            .filter_map(|inst| match &inst.data {
                InstData::CallExtern(symbol) => Some(symbol.as_str()),
                _ => None,
            })
            .collect();
        assert!(
            extern_calls.contains(&"__vow_string_literal"),
            "{extern_calls:?}"
        );
        assert!(
            !extern_calls.contains(&"__vow_string_from_cstr"),
            "{extern_calls:?}"
        );
    }

    #[test]
    fn string_from_lowers_to_clone_of_literal() {
        let body = Block {
            stmts: vec![],
            trailing_expr: Some(Box::new(string_from_expr(string_expr("hello")))),
            span: sp(),
        };
        let fn_def = make_fn("owned", vec![], string_ty(), body, vec![]);
        let (func, _, diags) = lower_function(
            &fn_def,
            "test.vow",
            &HashMap::new(),
            HashMap::new(),
            HashMap::new(),
            &HashSet::new(),
            HashMap::new(),
            HashMap::new(),
            &HashSet::new(),
            &HashMap::new(),
        );

        assert!(diags.is_empty(), "{diags:?}");
        let extern_calls: Vec<&str> = func
            .blocks
            .iter()
            .flat_map(|block| &block.insts)
            .filter_map(|inst| match &inst.data {
                InstData::CallExtern(symbol) => Some(symbol.as_str()),
                _ => None,
            })
            .collect();
        assert_eq!(
            extern_calls,
            vec!["__vow_string_literal", "__vow_string_clone"]
        );
    }

    #[test]
    fn lower_const_i64() {
        let body = Block {
            stmts: vec![],
            trailing_expr: Some(Box::new(int_expr(42))),
            span: sp(),
        };
        let fn_def = make_fn("const_fn", vec![], i64_ty(), body, vec![]);
        let (func, _, _) = lower_function(
            &fn_def,
            "",
            &HashMap::new(),
            HashMap::new(),
            HashMap::new(),
            &HashSet::new(),
            HashMap::new(),
            HashMap::new(),
            &HashSet::new(),
            &HashMap::new(),
        );

        assert_eq!(func.name, "const_fn");
        assert_eq!(func.return_ty, Ty::I64);

        let entry = &func.blocks[0];
        let const_inst = entry.insts.iter().find(|i| i.opcode == Opcode::ConstI64);
        assert!(const_inst.is_some());
        assert_eq!(const_inst.unwrap().data, InstData::ConstI64(42));

        let ret = entry.insts.iter().find(|i| i.opcode == Opcode::Return);
        assert!(ret.is_some());
    }

    #[test]
    fn lower_addition() {
        let body = Block {
            stmts: vec![],
            trailing_expr: Some(Box::new(Expr {
                kind: ExprKind::BinaryOp {
                    op: BinOp::Add,
                    lhs: Box::new(ident_expr("a")),
                    rhs: Box::new(ident_expr("b")),
                },
                span: sp(),
            })),
            span: sp(),
        };
        let fn_def = make_fn(
            "add",
            vec![make_param("a", i64_ty()), make_param("b", i64_ty())],
            i64_ty(),
            body,
            vec![],
        );
        let (func, _, _) = lower_function(
            &fn_def,
            "",
            &HashMap::new(),
            HashMap::new(),
            HashMap::new(),
            &HashSet::new(),
            HashMap::new(),
            HashMap::new(),
            &HashSet::new(),
            &HashMap::new(),
        );

        let entry = &func.blocks[0];
        let get_args: Vec<_> = entry
            .insts
            .iter()
            .filter(|i| i.opcode == Opcode::GetArg)
            .collect();
        assert_eq!(get_args.len(), 2);

        let add = entry.insts.iter().find(|i| i.opcode == Opcode::WrappingAdd);
        assert!(add.is_some());
        let add = add.unwrap();
        assert_eq!(add.args.len(), 2);
        assert_eq!(add.data, InstData::Integer(IntegerType::I64));
    }

    #[test]
    fn lower_let_binding() {
        let let_stmt = Stmt::Let {
            pattern: Pat {
                kind: PatKind::Ident {
                    name: "x".to_string(),
                    is_mut: false,
                },
                span: sp(),
            },
            ty: None,
            init: Box::new(int_expr(42)),
            span: sp(),
        };
        let body = Block {
            stmts: vec![let_stmt],
            trailing_expr: Some(Box::new(ident_expr("x"))),
            span: sp(),
        };
        let fn_def = make_fn("let_fn", vec![], i64_ty(), body, vec![]);
        let (func, _, _) = lower_function(
            &fn_def,
            "",
            &HashMap::new(),
            HashMap::new(),
            HashMap::new(),
            &HashSet::new(),
            HashMap::new(),
            HashMap::new(),
            &HashSet::new(),
            &HashMap::new(),
        );

        let entry = &func.blocks[0];
        let const_inst = entry.insts.iter().find(|i| i.opcode == Opcode::ConstI64);
        assert!(const_inst.is_some(), "expected ConstI64 for let binding");
        assert_eq!(const_inst.unwrap().data, InstData::ConstI64(42));

        let ret = entry.insts.iter().find(|i| i.opcode == Opcode::Return);
        assert!(ret.is_some());
        let const_id = const_inst.unwrap().id;
        assert_eq!(ret.unwrap().args, vec![const_id]);
    }

    #[test]
    fn lower_assignment_updates_identifier_binding() {
        let let_stmt = Stmt::Let {
            pattern: Pat {
                kind: PatKind::Ident {
                    name: "x".to_string(),
                    is_mut: true,
                },
                span: sp(),
            },
            ty: None,
            init: Box::new(int_expr(1)),
            span: sp(),
        };
        let assign_stmt = Stmt::Expr {
            expr: Expr {
                kind: ExprKind::Assign {
                    lhs: Box::new(ident_expr("x")),
                    rhs: Box::new(int_expr(2)),
                },
                span: sp(),
            },
            has_semicolon: true,
            span: sp(),
        };
        let body = Block {
            stmts: vec![let_stmt, assign_stmt],
            trailing_expr: Some(Box::new(ident_expr("x"))),
            span: sp(),
        };
        let fn_def = make_fn("assign_fn", vec![], i64_ty(), body, vec![]);
        let (func, _, _) = lower_function(
            &fn_def,
            "",
            &HashMap::new(),
            HashMap::new(),
            HashMap::new(),
            &HashSet::new(),
            HashMap::new(),
            HashMap::new(),
            &HashSet::new(),
            &HashMap::new(),
        );

        let all_insts: Vec<_> = func.blocks.iter().flat_map(|b| b.insts.iter()).collect();
        let assigned_const = all_insts
            .iter()
            .find(|i| i.data == InstData::ConstI64(2))
            .expect("assignment RHS should lower to ConstI64(2)");
        let ret = all_insts
            .iter()
            .find(|i| i.opcode == Opcode::Return)
            .expect("expected Return");
        assert_eq!(ret.args, vec![assigned_const.id]);
    }

    #[test]
    fn lower_if_else() {
        let if_expr = Expr {
            kind: ExprKind::If {
                condition: Box::new(bool_expr(true)),
                then_branch: Box::new(Block {
                    stmts: vec![],
                    trailing_expr: Some(Box::new(int_expr(1))),
                    span: sp(),
                }),
                else_branch: Some(Box::new(int_expr(2))),
            },
            span: sp(),
        };
        let body = Block {
            stmts: vec![],
            trailing_expr: Some(Box::new(if_expr)),
            span: sp(),
        };
        let fn_def = make_fn("if_fn", vec![], i64_ty(), body, vec![]);
        let (func, _, _) = lower_function(
            &fn_def,
            "",
            &HashMap::new(),
            HashMap::new(),
            HashMap::new(),
            &HashSet::new(),
            HashMap::new(),
            HashMap::new(),
            &HashSet::new(),
            &HashMap::new(),
        );

        assert!(
            func.blocks.len() >= 4,
            "expected entry + then + else + merge"
        );

        let all_insts: Vec<_> = func.blocks.iter().flat_map(|b| b.insts.iter()).collect();

        let branch = all_insts.iter().find(|i| i.opcode == Opcode::Branch);
        assert!(branch.is_some(), "expected Branch instruction");

        let phi = all_insts.iter().find(|i| i.opcode == Opcode::Phi);
        assert!(phi.is_some(), "expected Phi instruction");

        let upsilons: Vec<_> = all_insts
            .iter()
            .filter(|i| i.opcode == Opcode::Upsilon)
            .collect();
        assert_eq!(upsilons.len(), 2, "expected 2 Upsilon instructions");

        let phi_id = phi.unwrap().id;
        for up in &upsilons {
            assert_eq!(
                up.data,
                InstData::PhiTarget(phi_id),
                "Upsilon should target the Phi"
            );
        }
    }

    #[test]
    fn match_expression_u64_result_phi_uses_arm_type() {
        let u64_cast = |v| Expr {
            kind: ExprKind::Cast {
                expr: Box::new(int_expr(v)),
                target_ty: Box::new(u64_ty()),
            },
            span: sp(),
        };
        let enum_pat = |variant: &str| Pat {
            kind: PatKind::EnumVariant {
                path: vec!["Pick".to_string(), variant.to_string()],
                inner: vec![],
            },
            span: sp(),
        };
        let match_expr = Expr {
            kind: ExprKind::Match {
                scrutinee: Box::new(ident_expr("p")),
                arms: vec![
                    MatchArm {
                        pattern: enum_pat("Big"),
                        body: u64_cast(9223372036854775808),
                        span: sp(),
                    },
                    MatchArm {
                        pattern: enum_pat("Zero"),
                        body: u64_cast(0),
                        span: sp(),
                    },
                ],
            },
            span: sp(),
        };
        let body = Block {
            stmts: vec![],
            trailing_expr: Some(Box::new(match_expr)),
            span: sp(),
        };
        let fn_def = make_fn(
            "pick",
            vec![make_param(
                "p",
                Type::Named {
                    name: "Pick".to_string(),
                    span: sp(),
                },
            )],
            u64_ty(),
            body,
            vec![],
        );
        let enum_variant_map = HashMap::from([(
            "Pick".to_string(),
            vec!["Big".to_string(), "Zero".to_string()],
        )]);
        let (func, _, _) = lower_function(
            &fn_def,
            "",
            &HashMap::new(),
            HashMap::new(),
            enum_variant_map,
            &HashSet::new(),
            HashMap::new(),
            HashMap::new(),
            &HashSet::new(),
            &HashMap::new(),
        );

        let phis: Vec<_> = func
            .blocks
            .iter()
            .flat_map(|block| block.insts.iter())
            .filter(|inst| inst.opcode == Opcode::Phi)
            .collect();
        assert_eq!(phis.len(), 1, "expected only the match result Phi");
        assert_eq!(phis[0].ty, Ty::U64);
    }

    #[test]
    fn match_expression_u64_result_phi_skips_exiting_first_arm() {
        let u64_cast = |v| Expr {
            kind: ExprKind::Cast {
                expr: Box::new(int_expr(v)),
                target_ty: Box::new(u64_ty()),
            },
            span: sp(),
        };
        let enum_pat = |variant: &str| Pat {
            kind: PatKind::EnumVariant {
                path: vec!["Pick".to_string(), variant.to_string()],
                inner: vec![],
            },
            span: sp(),
        };
        let return_zero = Expr {
            kind: ExprKind::Return {
                value: Some(Box::new(u64_cast(0))),
            },
            span: sp(),
        };
        let exiting_body = Expr {
            kind: ExprKind::Block(Box::new(Block {
                stmts: vec![],
                trailing_expr: Some(Box::new(return_zero)),
                span: sp(),
            })),
            span: sp(),
        };
        let match_expr = Expr {
            kind: ExprKind::Match {
                scrutinee: Box::new(ident_expr("p")),
                arms: vec![
                    MatchArm {
                        pattern: enum_pat("Big"),
                        body: exiting_body,
                        span: sp(),
                    },
                    MatchArm {
                        pattern: enum_pat("Zero"),
                        body: u64_cast(9223372036854775808),
                        span: sp(),
                    },
                ],
            },
            span: sp(),
        };
        let body = Block {
            stmts: vec![],
            trailing_expr: Some(Box::new(match_expr)),
            span: sp(),
        };
        let fn_def = make_fn(
            "pick",
            vec![make_param(
                "p",
                Type::Named {
                    name: "Pick".to_string(),
                    span: sp(),
                },
            )],
            u64_ty(),
            body,
            vec![],
        );
        let enum_variant_map = HashMap::from([(
            "Pick".to_string(),
            vec!["Big".to_string(), "Zero".to_string()],
        )]);
        let (func, _, _) = lower_function(
            &fn_def,
            "",
            &HashMap::new(),
            HashMap::new(),
            enum_variant_map,
            &HashSet::new(),
            HashMap::new(),
            HashMap::new(),
            &HashSet::new(),
            &HashMap::new(),
        );

        let phis: Vec<_> = func
            .blocks
            .iter()
            .flat_map(|block| block.insts.iter())
            .filter(|inst| inst.opcode == Opcode::Phi)
            .collect();
        assert_eq!(phis.len(), 1, "expected only the match result Phi");
        let phi_id = phis[0].id;
        assert_eq!(phis[0].ty, Ty::U64);

        let result_upsilons: Vec<_> = func
            .blocks
            .iter()
            .flat_map(|block| block.insts.iter())
            .filter(|inst| inst.data == InstData::PhiTarget(phi_id))
            .collect();
        assert_eq!(
            result_upsilons.len(),
            1,
            "only the arm that reaches the match merge should feed the result Phi"
        );
    }

    #[test]
    fn match_expression_u64_result_phi_uses_later_u64_for_literal_first_arm() {
        let u64_cast = |v| Expr {
            kind: ExprKind::Cast {
                expr: Box::new(int_expr(v)),
                target_ty: Box::new(u64_ty()),
            },
            span: sp(),
        };
        let enum_pat = |variant: &str| Pat {
            kind: PatKind::EnumVariant {
                path: vec!["Pick".to_string(), variant.to_string()],
                inner: vec![],
            },
            span: sp(),
        };
        let match_expr = Expr {
            kind: ExprKind::Match {
                scrutinee: Box::new(ident_expr("p")),
                arms: vec![
                    MatchArm {
                        pattern: enum_pat("Big"),
                        body: int_expr(0),
                        span: sp(),
                    },
                    MatchArm {
                        pattern: enum_pat("Zero"),
                        body: u64_cast(9223372036854775808),
                        span: sp(),
                    },
                ],
            },
            span: sp(),
        };
        let body = Block {
            stmts: vec![],
            trailing_expr: Some(Box::new(match_expr)),
            span: sp(),
        };
        let fn_def = make_fn(
            "pick",
            vec![make_param(
                "p",
                Type::Named {
                    name: "Pick".to_string(),
                    span: sp(),
                },
            )],
            u64_ty(),
            body,
            vec![],
        );
        let enum_variant_map = HashMap::from([(
            "Pick".to_string(),
            vec!["Big".to_string(), "Zero".to_string()],
        )]);
        let (func, _, _) = lower_function(
            &fn_def,
            "",
            &HashMap::new(),
            HashMap::new(),
            enum_variant_map,
            &HashSet::new(),
            HashMap::new(),
            HashMap::new(),
            &HashSet::new(),
            &HashMap::new(),
        );

        let phis: Vec<_> = func
            .blocks
            .iter()
            .flat_map(|block| block.insts.iter())
            .filter(|inst| inst.opcode == Opcode::Phi)
            .collect();
        assert_eq!(phis.len(), 1, "expected only the match result Phi");
        assert_eq!(phis[0].ty, Ty::U64);
    }

    #[test]
    fn aggregate_match_payload_preserves_linear_pointer_type() {
        let match_expr = Expr {
            kind: ExprKind::Match {
                scrutinee: Box::new(ident_expr("payload")),
                arms: vec![MatchArm {
                    pattern: Pat {
                        kind: PatKind::EnumVariant {
                            path: vec!["Payload".to_string(), "Token".to_string()],
                            inner: vec![Pat {
                                kind: PatKind::Ident {
                                    name: "token".to_string(),
                                    is_mut: false,
                                },
                                span: sp(),
                            }],
                        },
                        span: sp(),
                    },
                    body: ident_expr("token"),
                    span: sp(),
                }],
            },
            span: sp(),
        };
        let fn_def = make_fn(
            "unwrap_token",
            vec![make_param(
                "payload",
                Type::Named {
                    name: "Payload".to_string(),
                    span: sp(),
                },
            )],
            Type::Named {
                name: "Token".to_string(),
                span: sp(),
            },
            Block {
                stmts: vec![],
                trailing_expr: Some(Box::new(match_expr)),
                span: sp(),
            },
            vec![],
        );
        let pattern_key = match &fn_def.body.trailing_expr.as_ref().unwrap().kind {
            ExprKind::Match { arms, .. } => match &arms[0].pattern.kind {
                PatKind::EnumVariant { inner, .. } => &inner[0] as *const Pat as usize,
                _ => unreachable!(),
            },
            _ => unreachable!(),
        };
        let patterns = Rc::new(HashMap::from([(
            pattern_key,
            vow_types::check::PatternAggregateInfo {
                type_name: "Token".to_string(),
                vec_elem_types: vec![],
                vec_option_elem_types: vec![],
                vec_variant_payload_types: vec![],
                option_elem_type: None,
                variant_payload_types: vec![],
                is_linear: true,
            },
        )]));
        let enum_variant_map = HashMap::from([("Payload".to_string(), vec!["Token".to_string()])]);
        let linear_structs = HashSet::from(["Token".to_string(), "Payload".to_string()]);

        let (func, _, warnings) = lower_function_with_pattern_aggregates(
            &fn_def,
            "test.vow",
            &HashMap::new(),
            HashMap::new(),
            enum_variant_map,
            HashMap::new(),
            Rc::new(HashMap::new()),
            &linear_structs,
            Rc::new(HashMap::new()),
            HashMap::new(),
            Rc::new(HashMap::new()),
            HashMap::new(),
            &HashSet::new(),
            &patterns,
            &HashMap::new(),
        );

        assert!(warnings.is_empty(), "unexpected warnings: {warnings:?}");
        let wrapper = func.blocks[0]
            .insts
            .iter()
            .find(|inst| inst.opcode == Opcode::GetArg)
            .expect("linear enum argument");
        assert_eq!(wrapper.ty, Ty::LinearPtr);
        assert!(
            func.blocks
                .iter()
                .flat_map(|block| &block.insts)
                .any(|inst| {
                    inst.opcode == Opcode::LinearConsume && inst.args == vec![wrapper.id]
                })
        );
        let payload_get = func
            .blocks
            .iter()
            .flat_map(|block| &block.insts)
            .find(|inst| inst.opcode == Opcode::FieldGet && inst.data == InstData::FieldIndex(1))
            .expect("enum payload FieldGet");
        assert_eq!(payload_get.ty, Ty::LinearPtr);
    }

    #[test]
    fn aggregate_match_payload_preserves_nested_vec_path() {
        let row_at_zero = Expr {
            kind: ExprKind::Index {
                base: Box::new(ident_expr("rows")),
                index: Box::new(int_expr(0)),
            },
            span: sp(),
        };
        let box_at_zero = Expr {
            kind: ExprKind::Index {
                base: Box::new(row_at_zero),
                index: Box::new(int_expr(0)),
            },
            span: sp(),
        };
        let match_expr = Expr {
            kind: ExprKind::Match {
                scrutinee: Box::new(ident_expr("payload")),
                arms: vec![MatchArm {
                    pattern: Pat {
                        kind: PatKind::EnumVariant {
                            path: vec!["Payload".to_string(), "Rows".to_string()],
                            inner: vec![Pat {
                                kind: PatKind::Ident {
                                    name: "rows".to_string(),
                                    is_mut: false,
                                },
                                span: sp(),
                            }],
                        },
                        span: sp(),
                    },
                    body: Expr {
                        kind: ExprKind::FieldAccess {
                            base: Box::new(box_at_zero),
                            field: "v".to_string(),
                        },
                        span: sp(),
                    },
                    span: sp(),
                }],
            },
            span: sp(),
        };
        let fn_def = make_fn(
            "get_nested",
            vec![make_param(
                "payload",
                Type::Named {
                    name: "Payload".to_string(),
                    span: sp(),
                },
            )],
            i64_ty(),
            Block {
                stmts: vec![],
                trailing_expr: Some(Box::new(match_expr)),
                span: sp(),
            },
            vec![],
        );
        let pattern_key = match &fn_def.body.trailing_expr.as_ref().unwrap().kind {
            ExprKind::Match { arms, .. } => match &arms[0].pattern.kind {
                PatKind::EnumVariant { inner, .. } => &inner[0] as *const Pat as usize,
                _ => unreachable!(),
            },
            _ => unreachable!(),
        };
        let patterns = Rc::new(HashMap::from([(
            pattern_key,
            vow_types::check::PatternAggregateInfo {
                type_name: "Vec".to_string(),
                vec_elem_types: vec!["Vec".to_string(), "Box".to_string()],
                vec_option_elem_types: vec![None, None],
                vec_variant_payload_types: vec![vec![], vec![]],
                option_elem_type: None,
                variant_payload_types: vec![],
                is_linear: false,
            },
        )]));
        let enum_variant_map = HashMap::from([("Payload".to_string(), vec!["Rows".to_string()])]);
        let struct_field_map = HashMap::from([(
            "Box".to_string(),
            vec!["marker".to_string(), "v".to_string()],
        )]);
        let struct_field_types = HashMap::from([(
            "Box".to_string(),
            vec!["i64".to_string(), "i64".to_string()],
        )]);

        let (func, _, warnings) = lower_function_with_pattern_aggregates(
            &fn_def,
            "test.vow",
            &HashMap::new(),
            struct_field_map,
            enum_variant_map,
            HashMap::new(),
            Rc::new(HashMap::new()),
            &HashSet::new(),
            Rc::new(HashMap::new()),
            struct_field_types,
            Rc::new(HashMap::new()),
            HashMap::new(),
            &HashSet::new(),
            &patterns,
            &HashMap::new(),
        );

        assert!(warnings.is_empty(), "unexpected warnings: {warnings:?}");
        let insts: Vec<_> = func.blocks.iter().flat_map(|block| &block.insts).collect();
        let field_get = insts
            .iter()
            .find(|inst| {
                if inst.opcode != Opcode::FieldGet || inst.data != InstData::FieldIndex(1) {
                    return false;
                }
                let Some(source) = inst.args.first() else {
                    return false;
                };
                insts.iter().any(|candidate| {
                    candidate.id == *source
                        && candidate.data == InstData::CallExtern("__vow_vec_get_val".to_string())
                })
            })
            .expect("field access after nested Vec indexes");
        assert_eq!(field_get.ty, Ty::I64);
    }

    #[test]
    fn pattern_scalar_types_map_to_their_exact_ir_types() {
        let cases = [
            (PatternScalarType::I8, Ty::I8),
            (PatternScalarType::I16, Ty::I16),
            (PatternScalarType::I32, Ty::I32),
            (PatternScalarType::I64, Ty::I64),
            (PatternScalarType::I128, Ty::I128),
            (PatternScalarType::U8, Ty::U8),
            (PatternScalarType::U16, Ty::U16),
            (PatternScalarType::U32, Ty::U32),
            (PatternScalarType::U64, Ty::U64),
            (PatternScalarType::U128, Ty::U128),
            (PatternScalarType::F32, Ty::F32),
            (PatternScalarType::F64, Ty::F64),
            (PatternScalarType::Bool, Ty::Bool),
        ];

        for (pattern_type, ir_type) in cases {
            assert_eq!(pattern_scalar_ir_type(pattern_type), ir_type);
        }
    }

    #[test]
    fn vec_element_metadata_preserves_nested_option_payload_width() {
        let mut ctx = LowerCtx::new(
            "metadata".to_string(),
            vec![],
            vec![],
            Ty::Unit,
            vec![],
            "test.vow".to_string(),
            HashMap::new(),
            HashMap::new(),
            HashMap::new(),
            HashSet::new(),
            Rc::new(HashMap::new()),
            HashMap::new(),
            Rc::new(HashMap::new()),
            HashMap::new(),
            HashSet::new(),
            Rc::new(HashMap::new()),
        );
        let source = InstId(1);
        let nested_vec = InstId(2);
        let nested_option = InstId(3);
        ctx.inst_vec_elem_types
            .insert(source, vec!["Vec".to_string(), "Option".to_string()]);
        ctx.inst_vec_option_elem_tys
            .insert(source, vec![None, Some(Ty::U8)]);

        propagate_vec_element_metadata(&mut ctx, source, nested_vec);
        assert_eq!(ctx.inst_struct_type.get(&nested_vec).unwrap(), "Vec");
        assert_eq!(
            ctx.inst_vec_elem_types.get(&nested_vec).unwrap(),
            &["Option".to_string()]
        );
        assert_eq!(
            ctx.inst_vec_option_elem_tys.get(&nested_vec).unwrap(),
            &[Some(Ty::U8)]
        );

        propagate_vec_element_metadata(&mut ctx, nested_vec, nested_option);
        assert_eq!(ctx.inst_struct_type.get(&nested_option).unwrap(), "Option");
        assert_eq!(ctx.inst_option_elem_ty.get(&nested_option), Some(&Ty::U8));
    }

    #[test]
    fn phi_ownership_widening_propagates_to_dependent_phis() {
        let mut ctx = LowerCtx::new(
            "phi_ownership".to_string(),
            vec![],
            vec![],
            Ty::Unit,
            vec![],
            "test.vow".to_string(),
            HashMap::new(),
            HashMap::new(),
            HashMap::new(),
            HashSet::new(),
            Rc::new(HashMap::new()),
            HashMap::new(),
            Rc::new(HashMap::new()),
            HashMap::new(),
            HashSet::new(),
            Rc::new(HashMap::new()),
        );

        let loop_phi = ctx.emit(Opcode::Phi, Ty::Ptr, vec![], InstData::None, sp());
        let exit_phi = ctx.emit(Opcode::Phi, Ty::Ptr, vec![], InstData::None, sp());
        ctx.emit(
            Opcode::Upsilon,
            Ty::Ptr,
            vec![loop_phi],
            InstData::PhiTarget(exit_phi),
            sp(),
        );
        let owned = ctx.emit(Opcode::Phi, Ty::LinearPtr, vec![], InstData::None, sp());
        ctx.emit(
            Opcode::Upsilon,
            Ty::LinearPtr,
            vec![owned],
            InstData::PhiTarget(loop_phi),
            sp(),
        );

        assert_eq!(ctx.inst_ty(loop_phi), Ty::LinearPtr);
        assert_eq!(ctx.inst_ty(exit_phi), Ty::LinearPtr);
        let insts = &ctx.func.blocks[0].insts;
        assert_eq!(insts[loop_phi.0 as usize].ty, Ty::LinearPtr);
        assert_eq!(insts[exit_phi.0 as usize].ty, Ty::LinearPtr);
    }

    #[test]
    fn nested_option_pattern_metadata_preserves_payload_width() {
        let byte_pattern = Pat {
            kind: PatKind::Ident {
                name: "byte".to_string(),
                is_mut: false,
            },
            span: sp(),
        };
        let inner_match = Expr {
            kind: ExprKind::Match {
                scrutinee: Box::new(ident_expr("inner")),
                arms: vec![MatchArm {
                    pattern: Pat {
                        kind: PatKind::EnumVariant {
                            path: vec!["Option".to_string(), "Some".to_string()],
                            inner: vec![byte_pattern],
                        },
                        span: sp(),
                    },
                    body: ident_expr("byte"),
                    span: sp(),
                }],
            },
            span: sp(),
        };
        let outer_match = Expr {
            kind: ExprKind::Match {
                scrutinee: Box::new(ident_expr("value")),
                arms: vec![MatchArm {
                    pattern: Pat {
                        kind: PatKind::EnumVariant {
                            path: vec!["Option".to_string(), "Some".to_string()],
                            inner: vec![Pat {
                                kind: PatKind::Ident {
                                    name: "inner".to_string(),
                                    is_mut: false,
                                },
                                span: sp(),
                            }],
                        },
                        span: sp(),
                    },
                    body: inner_match,
                    span: sp(),
                }],
            },
            span: sp(),
        };
        let fn_def = make_fn(
            "unwrap_nested",
            vec![make_param("value", option_ty(option_ty(u8_ty())))],
            u8_ty(),
            Block {
                stmts: vec![],
                trailing_expr: Some(Box::new(outer_match)),
                span: sp(),
            },
            vec![],
        );
        let pattern_key = match &fn_def.body.trailing_expr.as_ref().unwrap().kind {
            ExprKind::Match { arms, .. } => match &arms[0].pattern.kind {
                PatKind::EnumVariant { inner, .. } => &inner[0] as *const Pat as usize,
                _ => unreachable!(),
            },
            _ => unreachable!(),
        };
        let patterns = Rc::new(HashMap::from([(
            pattern_key,
            vow_types::check::PatternAggregateInfo {
                type_name: "Option".to_string(),
                vec_elem_types: vec![],
                vec_option_elem_types: vec![],
                vec_variant_payload_types: vec![],
                option_elem_type: Some(PatternScalarType::U8),
                variant_payload_types: vec![None, Some(PatternScalarType::U8)],
                is_linear: false,
            },
        )]));
        let enum_variant_map = HashMap::from([(
            "Option".to_string(),
            vec!["None".to_string(), "Some".to_string()],
        )]);

        let (func, _, warnings) = lower_function_with_pattern_aggregates(
            &fn_def,
            "test.vow",
            &HashMap::new(),
            HashMap::new(),
            enum_variant_map,
            HashMap::new(),
            Rc::new(HashMap::new()),
            &HashSet::new(),
            Rc::new(HashMap::new()),
            HashMap::new(),
            Rc::new(HashMap::new()),
            HashMap::new(),
            &HashSet::new(),
            &patterns,
            &HashMap::new(),
        );

        assert!(warnings.is_empty(), "unexpected warnings: {warnings:?}");
        assert!(
            func.blocks
                .iter()
                .flat_map(|block| &block.insts)
                .any(|inst| {
                    inst.opcode == Opcode::FieldGet
                        && inst.data == InstData::FieldIndex(1)
                        && inst.ty == Ty::U8
                }),
            "the nested Some payload must retain its u8 width"
        );
    }

    #[test]
    fn lower_empty_function() {
        let fn_def = make_fn("empty_fn", vec![], unit_ty(), empty_block(), vec![]);
        let (func, _, _) = lower_function(
            &fn_def,
            "",
            &HashMap::new(),
            HashMap::new(),
            HashMap::new(),
            &HashSet::new(),
            HashMap::new(),
            HashMap::new(),
            &HashSet::new(),
            &HashMap::new(),
        );

        let all_insts: Vec<_> = func.blocks.iter().flat_map(|b| b.insts.iter()).collect();
        let ret = all_insts.iter().find(|i| i.opcode == Opcode::Return);
        assert!(ret.is_some(), "expected Return instruction");
        assert_eq!(func.return_ty, Ty::Unit);
    }

    #[test]
    fn pin_to_root_process_stdout_lowers_to_string_pin() {
        let body = Block {
            stmts: vec![],
            trailing_expr: Some(Box::new(call_expr(
                "pin_to_root",
                vec![call_expr("process_get_stdout", vec![])],
            ))),
            span: sp(),
        };
        let fn_def = make_fn(
            "pin_process_stdout",
            vec![],
            string_ty(),
            body,
            vec![Effect::IO],
        );
        let (func, _, _) = lower_function(
            &fn_def,
            "",
            &HashMap::new(),
            HashMap::new(),
            HashMap::new(),
            &HashSet::new(),
            HashMap::new(),
            HashMap::new(),
            &HashSet::new(),
            &HashMap::new(),
        );

        let all_insts: Vec<_> = func.blocks.iter().flat_map(|b| b.insts.iter()).collect();
        assert!(
            all_insts
                .iter()
                .any(|inst| inst.data
                    == InstData::CallExtern("__vow_process_get_stdout".to_string())),
            "expected process_get_stdout extern call"
        );
        assert!(
            all_insts
                .iter()
                .any(|inst| inst.data
                    == InstData::CallExtern("__vow_string_pin_to_root".to_string())),
            "direct pin_to_root(process_get_stdout()) must lower to string pin"
        );
    }

    #[test]
    fn pin_to_root_preserves_vec_element_metadata() {
        let vec_box_ty = Type::Generic {
            name: "Vec".to_string(),
            args: vec![Type::Named {
                name: "Box".to_string(),
                span: sp(),
            }],
            span: sp(),
        };
        let body = Block {
            stmts: vec![],
            trailing_expr: Some(Box::new(call_expr(
                "pin_to_root",
                vec![ident_expr("values")],
            ))),
            span: sp(),
        };
        let fn_def = make_fn(
            "pin_values",
            vec![make_param("values", vec_box_ty.clone())],
            vec_box_ty,
            body,
            vec![],
        );

        let (func, _, warnings) = lower_function(
            &fn_def,
            "",
            &HashMap::new(),
            HashMap::new(),
            HashMap::new(),
            &HashSet::new(),
            HashMap::new(),
            HashMap::new(),
            &HashSet::new(),
            &HashMap::new(),
        );

        assert!(warnings.is_empty(), "unexpected warnings: {warnings:?}");
        assert!(
            func.blocks
                .iter()
                .flat_map(|block| &block.insts)
                .any(|inst| {
                    inst.data == InstData::CallExtern("__vow_vec_pin_to_root_val".to_string())
                })
        );
    }

    #[test]
    fn ensures_emitted_before_explicit_return() {
        let ensures_clause = VowClause::Ensures {
            expr: bool_expr(true),
            span: sp(),
        };
        let vow_block = VowBlock {
            clauses: vec![ensures_clause],
            span: sp(),
        };
        let return_expr = Expr {
            kind: ExprKind::Return {
                value: Some(Box::new(int_expr(42))),
            },
            span: sp(),
        };
        let body = Block {
            stmts: vec![Stmt::Expr {
                expr: return_expr,
                has_semicolon: true,
                span: sp(),
            }],
            trailing_expr: None,
            span: sp(),
        };
        let fn_def = FnDef {
            vis: Visibility::Public,
            name: "explicit_return_fn".to_string(),
            params: vec![],
            return_ty: i64_ty(),
            effects: vec![],
            vow: Some(vow_block),
            body,
            span: sp(),
            is_declaration: false,
        };
        let (func, _, _) = lower_function(
            &fn_def,
            "",
            &HashMap::new(),
            HashMap::new(),
            HashMap::new(),
            &HashSet::new(),
            HashMap::new(),
            HashMap::new(),
            &HashSet::new(),
            &HashMap::new(),
        );

        let all_insts: Vec<_> = func.blocks.iter().flat_map(|b| b.insts.iter()).collect();
        let ens_pos = all_insts
            .iter()
            .position(|i| i.opcode == Opcode::VowEnsures)
            .expect("expected VowEnsures");
        let ret_pos = all_insts
            .iter()
            .position(|i| i.opcode == Opcode::Return)
            .expect("expected Return");
        assert!(
            ens_pos < ret_pos,
            "VowEnsures must appear before Return for explicit return"
        );
    }

    #[test]
    fn lower_while_loop_emits_phi_upsilon_and_backedge() {
        // fn countdown(n: i64) -> i64 { let mut i = n; while i > 0 { i = i - 1 }; i }
        let i64_ty = i64_ty();
        let param_n = make_param("n", i64_ty.clone());

        // let mut i = n
        let let_i = Stmt::Let {
            pattern: Pat {
                kind: PatKind::Ident {
                    name: "i".to_string(),
                    is_mut: true,
                },
                span: sp(),
            },
            ty: None,
            init: Box::new(ident_expr("n")),
            span: sp(),
        };

        // while body: i = i - 1
        let assign_stmt = Stmt::Expr {
            expr: Expr {
                kind: ExprKind::Assign {
                    lhs: Box::new(ident_expr("i")),
                    rhs: Box::new(Expr {
                        kind: ExprKind::BinaryOp {
                            op: BinOp::Sub,
                            lhs: Box::new(ident_expr("i")),
                            rhs: Box::new(int_expr(1)),
                        },
                        span: sp(),
                    }),
                },
                span: sp(),
            },
            has_semicolon: true,
            span: sp(),
        };
        let while_body = Block {
            stmts: vec![assign_stmt],
            trailing_expr: None,
            span: sp(),
        };

        // while i > 0 { ... }
        let while_expr = Expr {
            kind: ExprKind::While {
                condition: Box::new(Expr {
                    kind: ExprKind::BinaryOp {
                        op: BinOp::Gt,
                        lhs: Box::new(ident_expr("i")),
                        rhs: Box::new(int_expr(0)),
                    },
                    span: sp(),
                }),
                vow: None,
                body: Box::new(while_body),
            },
            span: sp(),
        };

        let body = Block {
            stmts: vec![
                let_i,
                Stmt::Expr {
                    expr: while_expr,
                    has_semicolon: true,
                    span: sp(),
                },
            ],
            trailing_expr: Some(Box::new(ident_expr("i"))),
            span: sp(),
        };

        let fn_def = make_fn("countdown", vec![param_n], i64_ty, body, vec![]);
        let (func, _, _) = lower_function(
            &fn_def,
            "",
            &HashMap::new(),
            HashMap::new(),
            HashMap::new(),
            &HashSet::new(),
            HashMap::new(),
            HashMap::new(),
            &HashSet::new(),
            &HashMap::new(),
        );

        let all_insts: Vec<_> = func.blocks.iter().flat_map(|b| b.insts.iter()).collect();

        // Must have a Phi (for loop var `i`)
        let phi = all_insts.iter().find(|i| i.opcode == Opcode::Phi);
        assert!(phi.is_some(), "expected Phi for loop variable");

        // Must have at least 2 Upsilons: pre-loop initial feed and back-edge feed
        let upsilons: Vec<_> = all_insts
            .iter()
            .filter(|i| i.opcode == Opcode::Upsilon)
            .collect();
        assert!(
            upsilons.len() >= 2,
            "expected at least 2 Upsilons for while loop"
        );

        // Must have a width-parametric signed 64-bit comparison for the condition.
        assert!(
            all_insts.iter().any(|i| {
                i.opcode == Opcode::Gt && i.data == InstData::Integer(IntegerType::I64)
            }),
            "expected Gt[i64] for while condition"
        );

        // Must have Branch
        assert!(
            all_insts.iter().any(|i| i.opcode == Opcode::Branch),
            "expected Branch for while loop"
        );

        // Must have at least 2 Jumps (pre-header -> header, body -> header)
        let jumps: Vec<_> = all_insts
            .iter()
            .filter(|i| i.opcode == Opcode::Jump)
            .collect();
        assert!(jumps.len() >= 2, "expected at least 2 Jumps for while loop");

        // Should produce at least 4 blocks: entry, header, body, exit
        assert!(
            func.blocks.len() >= 4,
            "expected entry+header+body+exit blocks"
        );
    }

    #[test]
    fn continue_in_while_emits_jump_to_header() {
        // fn f() { let mut i = 0; while i < 10 { i = i + 1; if i == 5 { continue; } } }
        let let_i = Stmt::Let {
            pattern: Pat {
                kind: PatKind::Ident {
                    name: "i".to_string(),
                    is_mut: true,
                },
                span: sp(),
            },
            ty: None,
            init: Box::new(int_expr(0)),
            span: sp(),
        };

        // i = i + 1
        let incr = Stmt::Expr {
            expr: Expr {
                kind: ExprKind::Assign {
                    lhs: Box::new(ident_expr("i")),
                    rhs: Box::new(Expr {
                        kind: ExprKind::BinaryOp {
                            op: BinOp::Add,
                            lhs: Box::new(ident_expr("i")),
                            rhs: Box::new(int_expr(1)),
                        },
                        span: sp(),
                    }),
                },
                span: sp(),
            },
            has_semicolon: true,
            span: sp(),
        };

        // if i == 5 { continue; }
        let if_continue = Stmt::Expr {
            expr: Expr {
                kind: ExprKind::If {
                    condition: Box::new(Expr {
                        kind: ExprKind::BinaryOp {
                            op: BinOp::Eq,
                            lhs: Box::new(ident_expr("i")),
                            rhs: Box::new(int_expr(5)),
                        },
                        span: sp(),
                    }),
                    then_branch: Box::new(Block {
                        stmts: vec![Stmt::Expr {
                            expr: Expr {
                                kind: ExprKind::Continue,
                                span: sp(),
                            },
                            has_semicolon: true,
                            span: sp(),
                        }],
                        trailing_expr: None,
                        span: sp(),
                    }),
                    else_branch: None,
                },
                span: sp(),
            },
            has_semicolon: true,
            span: sp(),
        };

        let while_body = Block {
            stmts: vec![incr, if_continue],
            trailing_expr: None,
            span: sp(),
        };

        let while_expr = Expr {
            kind: ExprKind::While {
                condition: Box::new(Expr {
                    kind: ExprKind::BinaryOp {
                        op: BinOp::Lt,
                        lhs: Box::new(ident_expr("i")),
                        rhs: Box::new(int_expr(10)),
                    },
                    span: sp(),
                }),
                vow: None,
                body: Box::new(while_body),
            },
            span: sp(),
        };

        let body = Block {
            stmts: vec![
                let_i,
                Stmt::Expr {
                    expr: while_expr,
                    has_semicolon: true,
                    span: sp(),
                },
            ],
            trailing_expr: None,
            span: sp(),
        };

        let fn_def = make_fn("f", vec![], unit_ty(), body, vec![]);
        let (func, _, _) = lower_function(
            &fn_def,
            "",
            &HashMap::new(),
            HashMap::new(),
            HashMap::new(),
            &HashSet::new(),
            HashMap::new(),
            HashMap::new(),
            &HashSet::new(),
            &HashMap::new(),
        );

        let all_insts: Vec<_> = func.blocks.iter().flat_map(|b| b.insts.iter()).collect();

        // continue produces an extra Jump to the header block (3 total: pre-header→header,
        // continue→header, end-of-body→header)
        let jumps: Vec<_> = all_insts
            .iter()
            .filter(|i| i.opcode == Opcode::Jump)
            .collect();
        assert!(
            jumps.len() >= 3,
            "expected at least 3 Jumps (pre-header, continue, back-edge), got {}",
            jumps.len()
        );

        // continue also produces Upsilons for the mutation variable before the jump
        let upsilons: Vec<_> = all_insts
            .iter()
            .filter(|i| i.opcode == Opcode::Upsilon)
            .collect();
        assert!(
            upsilons.len() >= 3,
            "expected at least 3 Upsilons (pre-header, continue, back-edge), got {}",
            upsilons.len()
        );
    }

    #[test]
    fn struct_alloc_includes_guard_slot() {
        let body = Block {
            stmts: vec![],
            trailing_expr: Some(Box::new(Expr {
                kind: ExprKind::StructLiteral {
                    name: "Point".to_string(),
                    fields: vec![
                        ("x".to_string(), int_expr(1)),
                        ("y".to_string(), int_expr(2)),
                        ("z".to_string(), int_expr(3)),
                    ],
                },
                span: sp(),
            })),
            span: sp(),
        };
        let fn_def = make_fn("make_point", vec![], i64_ty(), body, vec![]);
        let mut sfm = HashMap::new();
        sfm.insert(
            "Point".to_string(),
            vec!["x".to_string(), "y".to_string(), "z".to_string()],
        );
        let (func, _, _) = lower_function(
            &fn_def,
            "",
            &HashMap::new(),
            sfm,
            HashMap::new(),
            &HashSet::new(),
            HashMap::new(),
            HashMap::new(),
            &HashSet::new(),
            &HashMap::new(),
        );
        let alloc = func
            .blocks
            .iter()
            .flat_map(|b| b.insts.iter())
            .find(|i| i.opcode == Opcode::RegionAlloc)
            .expect("expected RegionAlloc");
        // 3 fields + 1 guard = 4 slots * 8 bytes = 32
        assert_eq!(alloc.data, InstData::AllocSize { size: 32, align: 8 });
    }

    #[test]
    fn enum_alloc_includes_guard_slot() {
        let body = Block {
            stmts: vec![],
            trailing_expr: Some(Box::new(Expr {
                kind: ExprKind::EnumConstruct {
                    path: vec!["Option".to_string(), "Some".to_string()],
                    fields: vec![int_expr(42)],
                },
                span: sp(),
            })),
            span: sp(),
        };
        let fn_def = make_fn("make_some", vec![], i64_ty(), body, vec![]);
        let (func, _, _) = lower_function(
            &fn_def,
            "",
            &HashMap::new(),
            HashMap::new(),
            HashMap::new(),
            &HashSet::new(),
            HashMap::new(),
            HashMap::new(),
            &HashSet::new(),
            &HashMap::new(),
        );
        let alloc = func
            .blocks
            .iter()
            .flat_map(|b| b.insts.iter())
            .find(|i| i.opcode == Opcode::RegionAlloc)
            .expect("expected RegionAlloc");
        // 1 discriminant + 1 payload + 1 guard = 3 slots * 8 bytes = 24
        assert_eq!(alloc.data, InstData::AllocSize { size: 24, align: 8 });
    }

    #[test]
    fn user_defined_call_results_keep_struct_tags_for_field_access() {
        let body = Block {
            stmts: vec![],
            trailing_expr: Some(Box::new(Expr {
                kind: ExprKind::FieldAccess {
                    base: Box::new(Expr {
                        kind: ExprKind::Call {
                            callee: Box::new(ident_expr("make_pair")),
                            args: vec![],
                        },
                        span: sp(),
                    }),
                    field: "a".to_string(),
                },
                span: sp(),
            })),
            span: sp(),
        };
        let fn_def = make_fn("caller", vec![], i64_ty(), body, vec![]);

        let mut func_index = HashMap::new();
        func_index.insert(
            "make_pair".to_string(),
            FuncSigInfo {
                id: FuncId(0),
                ret_ty: Ty::Ptr,
                ret_tag: Some("Pair".to_string()),
                ret_vec_elem: None,
                ret_option_elem: None,
                param_tys: vec![],
                param_ast_tys: vec![],
            },
        );

        let mut struct_field_map = HashMap::new();
        struct_field_map.insert("Pair".to_string(), vec!["a".to_string(), "b".to_string()]);

        let mut struct_field_type_names = HashMap::new();
        struct_field_type_names.insert(
            "Pair".to_string(),
            vec!["i64".to_string(), "i64".to_string()],
        );

        let (func, _, warnings) = lower_function(
            &fn_def,
            "",
            &func_index,
            struct_field_map,
            HashMap::new(),
            &HashSet::new(),
            struct_field_type_names,
            HashMap::new(),
            &HashSet::new(),
            &HashMap::new(),
        );

        assert!(
            warnings.is_empty(),
            "unexpected lowering warnings: {warnings:?}"
        );

        let field_get = func
            .blocks
            .iter()
            .flat_map(|b| b.insts.iter())
            .find(|i| i.opcode == Opcode::FieldGet)
            .expect("expected FieldGet");
        assert_eq!(field_get.data, InstData::FieldIndex(0));
    }

    #[test]
    fn user_defined_call_coerces_u8_literal_argument() {
        // fn caller() -> u8 { id_u8(1) }
        let body = Block {
            stmts: vec![],
            trailing_expr: Some(Box::new(call_expr("id_u8", vec![int_expr(1)]))),
            span: sp(),
        };
        let fn_def = make_fn("caller", vec![], u8_ty(), body, vec![]);

        let mut func_index = HashMap::new();
        func_index.insert(
            "id_u8".to_string(),
            FuncSigInfo {
                id: FuncId(0),
                ret_ty: Ty::U8,
                ret_tag: None,
                ret_vec_elem: None,
                ret_option_elem: None,
                param_tys: vec![Ty::U8],
                param_ast_tys: vec![u8_ty()],
            },
        );

        let (func, _, warnings) = lower_function(
            &fn_def,
            "",
            &func_index,
            HashMap::new(),
            HashMap::new(),
            &HashSet::new(),
            HashMap::new(),
            HashMap::new(),
            &HashSet::new(),
            &HashMap::new(),
        );

        assert!(
            warnings.is_empty(),
            "unexpected lowering warnings: {warnings:?}"
        );

        let const_u8 = func
            .blocks
            .iter()
            .flat_map(|b| b.insts.iter())
            .find(|i| i.opcode == Opcode::ConstU8)
            .expect("expected ConstU8 for literal u8 call argument, not a default i64 constant");
        assert_eq!(const_u8.ty, Ty::U8);
        assert_eq!(const_u8.data, InstData::ConstU8(1));

        let call = func
            .blocks
            .iter()
            .flat_map(|b| b.insts.iter())
            .find(|i| i.opcode == Opcode::Call)
            .expect("expected Call");
        assert_eq!(
            call.args,
            vec![const_u8.id],
            "Call argument must be the ConstU8 instruction, not a separately-lowered i64 constant"
        );
    }

    #[test]
    fn field_access_on_u8_field_uses_u8_ir_type() {
        // struct B { x: u8 }
        // fn byte() -> u8 { make_b().x }
        let body = Block {
            stmts: vec![],
            trailing_expr: Some(Box::new(Expr {
                kind: ExprKind::FieldAccess {
                    base: Box::new(Expr {
                        kind: ExprKind::Call {
                            callee: Box::new(ident_expr("make_b")),
                            args: vec![],
                        },
                        span: sp(),
                    }),
                    field: "x".to_string(),
                },
                span: sp(),
            })),
            span: sp(),
        };
        let fn_def = make_fn("byte", vec![], u8_ty(), body, vec![]);

        let mut func_index = HashMap::new();
        func_index.insert(
            "make_b".to_string(),
            FuncSigInfo {
                id: FuncId(0),
                ret_ty: Ty::Ptr,
                ret_tag: Some("B".to_string()),
                ret_vec_elem: None,
                ret_option_elem: None,
                param_tys: vec![],
                param_ast_tys: vec![],
            },
        );

        let mut struct_field_map = HashMap::new();
        struct_field_map.insert("B".to_string(), vec!["x".to_string()]);

        let mut struct_field_type_names = HashMap::new();
        struct_field_type_names.insert("B".to_string(), vec!["u8".to_string()]);

        let (func, _, warnings) = lower_function(
            &fn_def,
            "",
            &func_index,
            struct_field_map,
            HashMap::new(),
            &HashSet::new(),
            struct_field_type_names,
            HashMap::new(),
            &HashSet::new(),
            &HashMap::new(),
        );

        assert!(
            warnings.is_empty(),
            "unexpected lowering warnings: {warnings:?}"
        );

        let field_get = func
            .blocks
            .iter()
            .flat_map(|b| b.insts.iter())
            .find(|i| i.opcode == Opcode::FieldGet)
            .expect("expected FieldGet");
        assert_eq!(
            field_get.ty,
            Ty::U8,
            "FieldGet on a u8-typed field must carry Ty::U8, not the default Ty::I64"
        );
        assert_eq!(field_get.data, InstData::FieldIndex(0));
    }
}
