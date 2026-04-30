use std::collections::{BTreeSet, HashMap, HashSet};

use vow_diag::{Blame, Diagnostic, DiagnosticEmitter, ErrorCode, Severity, SourceLocation};
use vow_syntax::ast::{
    BinOp, Block, Effect, Expr, ExprKind, FnDef, Item, Lit, Module, Pat, PatKind, Stmt, UnOp,
    VowBlock, VowClause,
};
use vow_syntax::span::Span;

use crate::env::{EnumInfo, FnSig, StructInfo, TypeEnv, VariantInfo, VariantKind};
use crate::types::Ty;

/// Set of expression addresses (`*const Expr as usize`) whose resolved type is `Ty::Str`.
pub type StringExprSet = HashSet<usize>;

const MAX_HINT_CANDIDATES: usize = 256;
const MAX_HINT_IDENTIFIER_BYTES: usize = 128;

fn bounded_edit_distance(a: &str, b: &str, max_distance: usize) -> Option<usize> {
    let a_len = a.len();
    let n = b.len();
    if a_len.abs_diff(n) > max_distance {
        return None;
    }
    let mut prev = (0..=n).collect::<Vec<_>>();
    let mut curr = vec![0; n + 1];
    for (i, ca) in a.bytes().enumerate() {
        curr[0] = i + 1;
        let mut row_min = curr[0];
        for (j, cb) in b.bytes().enumerate() {
            let cost = if ca == cb { 0 } else { 1 };
            curr[j + 1] = (prev[j] + cost).min(prev[j + 1] + 1).min(curr[j] + 1);
            row_min = row_min.min(curr[j + 1]);
        }
        if row_min > max_distance {
            return None;
        }
        std::mem::swap(&mut prev, &mut curr);
    }
    let distance = prev[n];
    (distance <= max_distance).then_some(distance)
}

fn suggest_similar(name: &str, candidates: &[String], max_distance: usize) -> Option<String> {
    if name.len() > MAX_HINT_IDENTIFIER_BYTES {
        return None;
    }
    let mut best: Option<(usize, &str)> = None;
    for c in candidates.iter().take(MAX_HINT_CANDIDATES) {
        if c == name {
            continue;
        }
        if c.len() > MAX_HINT_IDENTIFIER_BYTES {
            continue;
        }
        let Some(d) = bounded_edit_distance(name, c, max_distance) else {
            continue;
        };
        if best.is_none_or(|(best_distance, _)| d < best_distance) {
            best = Some((d, c.as_str()));
        }
    }
    best.map(|(_, s)| s.to_string())
}

fn is_flat_slot_ty(ty: &Ty) -> bool {
    matches!(
        ty,
        Ty::I8
            | Ty::I16
            | Ty::I32
            | Ty::I64
            | Ty::I128
            | Ty::U8
            | Ty::U16
            | Ty::U32
            | Ty::U64
            | Ty::U128
            | Ty::F32
            | Ty::F64
            | Ty::Bool
    )
}

fn is_flat_vec_ty(ty: &Ty) -> bool {
    match ty {
        Ty::Applied(base, args) if matches!(base.as_ref(), Ty::Struct(name) if name == "Vec") => {
            args.first().is_some_and(is_flat_slot_ty)
        }
        _ => false,
    }
}

fn is_supported_pin_ty(ty: &Ty) -> bool {
    matches!(ty, Ty::Str) || is_flat_vec_ty(ty)
}

fn is_vec_raw_parts_copy_expr(expr: &vow_syntax::ast::Expr) -> bool {
    matches!(
        &expr.kind,
        ExprKind::EnumConstruct { path, .. }
            if path.first().is_some_and(|p| p == "Vec")
                && path.get(1).is_some_and(|p| p == "from_raw_parts_copy")
    )
}

pub struct Checker<'e> {
    pub(crate) env: TypeEnv,
    pub(crate) current_return_ty: Ty,
    pub(crate) current_fn_effects: BTreeSet<Effect>,
    pub(crate) error_count: usize,
    pub(crate) file: String,
    pub(crate) emitter: &'e mut dyn DiagnosticEmitter,
    string_exprs: StringExprSet,
    in_loop: u32,
    /// Stack of break-value type collectors. `Some(vec)` for `loop` (collects
    /// break types), `None` for `while` (break-with-value is an error).
    break_types_stack: Vec<Option<Vec<Ty>>>,
    pub const_values: HashMap<String, i64>,
    pub const_types: HashMap<String, Ty>,
}

impl<'e> Checker<'e> {
    pub fn new(file: impl Into<String>, emitter: &'e mut dyn DiagnosticEmitter) -> Self {
        Self {
            env: TypeEnv::new(),
            current_return_ty: Ty::Unit,
            current_fn_effects: BTreeSet::new(),
            error_count: 0,
            file: file.into(),
            emitter,
            string_exprs: HashSet::new(),
            in_loop: 0,
            break_types_stack: Vec::new(),
            const_values: HashMap::new(),
            const_types: HashMap::new(),
        }
    }

    pub fn into_string_exprs(self) -> StringExprSet {
        self.string_exprs
    }

    pub fn has_errors(&self) -> bool {
        self.error_count > 0
    }

    fn emit_error(&mut self, code: ErrorCode, msg: impl Into<String>, span: Span) {
        self.error_count += 1;
        self.emitter.emit(&Diagnostic {
            severity: Severity::Error,
            code,
            message: msg.into(),
            primary: SourceLocation {
                file: self.file.clone(),
                byte_offset: span.start,
                byte_len: span.len,
            },
            secondary: vec![],
            blame: Blame::None,
            hints: vec![],
        });
    }

    fn emit_error_with_hints(
        &mut self,
        code: ErrorCode,
        msg: impl Into<String>,
        span: Span,
        hints: Vec<String>,
    ) {
        self.error_count += 1;
        self.emitter.emit(&Diagnostic {
            severity: Severity::Error,
            code,
            message: msg.into(),
            primary: SourceLocation {
                file: self.file.clone(),
                byte_offset: span.start,
                byte_len: span.len,
            },
            secondary: vec![],
            blame: Blame::None,
            hints,
        });
    }

    pub fn check_module(&mut self, module: &Module) {
        // Pass 1a: Register type names (structs and enums, no fields yet)
        for item in &module.items {
            match item {
                Item::Struct(s) => {
                    self.env.define_struct(
                        &s.name,
                        StructInfo {
                            fields: vec![],
                            is_linear: s.is_linear,
                        },
                    );
                }
                Item::Enum(e) => {
                    self.env.define_enum(&e.name, EnumInfo { variants: vec![] });
                }
                _ => {}
            }
        }

        // Pass 1b: Resolve struct fields, enum variants, and type aliases
        for item in &module.items {
            match item {
                Item::Struct(s) => {
                    let fields: Vec<(String, Ty)> = s
                        .fields
                        .iter()
                        .map(|f| {
                            let ty = match self.env.resolve(&f.ty) {
                                Ok(ty) => ty,
                                Err(msg) => {
                                    self.emit_error(ErrorCode::TypeMismatch, msg, f.span);
                                    Ty::Unit
                                }
                            };
                            (f.name.clone(), ty)
                        })
                        .collect();
                    self.env.define_struct(
                        &s.name,
                        StructInfo {
                            fields,
                            is_linear: s.is_linear,
                        },
                    );
                }
                Item::Enum(e) => {
                    let variants: Vec<VariantInfo> = e
                        .variants
                        .iter()
                        .map(|v| {
                            let kind = match &v.kind {
                                vow_syntax::ast::VariantKind::Unit => VariantKind::Unit,
                                vow_syntax::ast::VariantKind::Tuple(types) => {
                                    let resolved: Vec<Ty> = types
                                        .iter()
                                        .map(|t| match self.env.resolve(t) {
                                            Ok(ty) => ty,
                                            Err(msg) => {
                                                self.emit_error(
                                                    ErrorCode::TypeMismatch,
                                                    msg,
                                                    t.span(),
                                                );
                                                Ty::Unit
                                            }
                                        })
                                        .collect();
                                    VariantKind::Tuple(resolved)
                                }
                                vow_syntax::ast::VariantKind::Struct(fields) => {
                                    let resolved: Vec<(String, Ty)> = fields
                                        .iter()
                                        .map(|f| {
                                            let ty = match self.env.resolve(&f.ty) {
                                                Ok(ty) => ty,
                                                Err(msg) => {
                                                    self.emit_error(
                                                        ErrorCode::TypeMismatch,
                                                        msg,
                                                        f.span,
                                                    );
                                                    Ty::Unit
                                                }
                                            };
                                            (f.name.clone(), ty)
                                        })
                                        .collect();
                                    VariantKind::Struct(resolved)
                                }
                            };
                            VariantInfo {
                                name: v.name.clone(),
                                kind,
                            }
                        })
                        .collect();
                    self.env.define_enum(&e.name, EnumInfo { variants });
                }
                Item::TypeAlias(a) => match self.env.resolve(&a.ty) {
                    Ok(ty) => self.env.define_alias(&a.name, ty),
                    Err(msg) => self.emit_error(ErrorCode::TypeMismatch, msg, a.ty.span()),
                },
                _ => {}
            }
        }

        // Pass 1b2: Register constants
        for item in &module.items {
            if let Item::Const(c) = item {
                let ty = match self.env.resolve(&c.ty) {
                    Ok(ty) => ty,
                    Err(msg) => {
                        self.emit_error(ErrorCode::TypeMismatch, msg, c.ty.span());
                        continue;
                    }
                };
                match &c.value.kind {
                    ExprKind::Lit(Lit::Int(v)) => {
                        if ty != Ty::I64 && ty != Ty::I32 {
                            self.emit_error(
                                ErrorCode::TypeMismatch,
                                format!(
                                    "const `{}` has type `{}`, expected integer type",
                                    c.name, ty
                                ),
                                c.span,
                            );
                        }
                        self.const_values.insert(c.name.clone(), *v as i64);
                        self.const_types.insert(c.name.clone(), ty.clone());
                    }
                    ExprKind::Lit(Lit::Bool(b)) => {
                        if ty != Ty::Bool {
                            self.emit_error(
                                ErrorCode::TypeMismatch,
                                format!("const `{}` has type `{}`, expected bool", c.name, ty),
                                c.span,
                            );
                        }
                        self.const_values.insert(c.name.clone(), *b as i64);
                        self.const_types.insert(c.name.clone(), ty.clone());
                    }
                    ExprKind::UnaryOp {
                        op: UnOp::Neg,
                        operand,
                    } => {
                        if ty != Ty::I64 && ty != Ty::I32 {
                            self.emit_error(
                                ErrorCode::TypeMismatch,
                                format!(
                                    "const `{}` has type `{}`, expected integer type",
                                    c.name, ty
                                ),
                                c.span,
                            );
                        }
                        if let ExprKind::Lit(Lit::Int(v)) = &operand.kind {
                            self.const_values.insert(c.name.clone(), -(*v as i64));
                            self.const_types.insert(c.name.clone(), ty.clone());
                        } else {
                            self.emit_error(
                                ErrorCode::TypeMismatch,
                                format!("const `{}` value must be a literal", c.name),
                                c.value.span,
                            );
                        }
                    }
                    _ => {
                        self.emit_error(
                            ErrorCode::TypeMismatch,
                            format!("const `{}` value must be a literal", c.name),
                            c.value.span,
                        );
                    }
                }
            }
        }

        // Pass 1c: Register function signatures (all types now resolvable)
        for item in &module.items {
            match item {
                Item::Fn(fn_def) => {
                    let params: Vec<Ty> = fn_def
                        .params
                        .iter()
                        .map(|p| match self.env.resolve(&p.ty) {
                            Ok(ty) => ty,
                            Err(msg) => {
                                self.emit_error(ErrorCode::TypeMismatch, msg, p.span);
                                Ty::Unit
                            }
                        })
                        .collect();
                    let return_ty = match self.env.resolve(&fn_def.return_ty) {
                        Ok(ty) => ty,
                        Err(msg) => {
                            self.emit_error(ErrorCode::TypeMismatch, msg, fn_def.return_ty.span());
                            Ty::Unit
                        }
                    };
                    self.env.define_fn(
                        &fn_def.name,
                        FnSig {
                            params,
                            return_ty,
                            effects: fn_def.effects.iter().cloned().collect(),
                        },
                    );
                }
                Item::Extern(block) => {
                    if block.vow.is_none() && !block.fns.is_empty() {
                        self.emit_error_with_hints(
                            ErrorCode::MissingContract,
                            "extern block requires a vow contract",
                            block.span,
                            vec![
                                "add a `vow { ... }` block specifying contracts for foreign functions".to_string(),
                            ],
                        );
                    }
                    for f in &block.fns {
                        let params = f
                            .params
                            .iter()
                            .map(|p| self.env.resolve(&p.ty).unwrap_or(Ty::Unit))
                            .collect();
                        let return_ty = self.env.resolve(&f.return_ty).unwrap_or(Ty::Unit);
                        let effects: BTreeSet<Effect> = f.effects.iter().cloned().collect();
                        self.env.define_fn(
                            &f.name,
                            FnSig {
                                params,
                                return_ty,
                                effects,
                            },
                        );
                    }
                }
                _ => {}
            }
        }

        // Pass 2: Check function bodies
        for item in &module.items {
            self.check_item(item);
        }
    }

    fn check_item(&mut self, item: &Item) {
        match item {
            Item::Fn(fn_def) if !fn_def.is_declaration => {
                self.check_fn(fn_def);
            }
            Item::Trait(t) => {
                self.emit_error(
                    ErrorCode::UnsupportedFeature,
                    "trait blocks are not supported in Vow",
                    t.span,
                );
            }
            Item::Impl(i) => {
                self.emit_error(
                    ErrorCode::UnsupportedFeature,
                    "impl blocks are not supported in Vow",
                    i.span,
                );
            }
            _ => {}
        }
    }

    fn check_vow_clauses(&mut self, vow: &VowBlock, context: &str) {
        for clause in &vow.clauses {
            let (expr, span, kind) = match clause {
                VowClause::Requires { expr, span } => (expr, *span, "requires"),
                VowClause::Ensures { expr, span } => (expr, *span, "ensures"),
                VowClause::Invariant { expr, span } => (expr, *span, "invariant"),
            };
            let ty = self.check_expr(expr);
            if ty != Ty::Bool && ty != Ty::Never {
                self.emit_error_with_hints(
                    ErrorCode::ContractTypeMismatch,
                    format!("`{kind}` clause has type `{ty}` but must be `bool`"),
                    span,
                    vec![format!(
                        "{context} `{kind}` clauses must evaluate to `bool`"
                    )],
                );
            }
        }
    }

    fn check_fn(&mut self, fn_def: &FnDef) {
        let outer_effects = std::mem::replace(
            &mut self.current_fn_effects,
            fn_def.effects.iter().cloned().collect(),
        );
        let outer_return_ty = self.current_return_ty.clone();

        let sig = self.env.lookup_fn(&fn_def.name).cloned();
        self.current_return_ty = sig
            .as_ref()
            .map(|s| s.return_ty.clone())
            .unwrap_or(Ty::Unit);

        self.env.push_scope();
        for (i, param) in fn_def.params.iter().enumerate() {
            let ty = sig
                .as_ref()
                .and_then(|s| s.params.get(i).cloned())
                .unwrap_or(Ty::Unit);
            self.env.define(&param.name, ty);
        }

        if let Some(ref vow) = fn_def.vow {
            for clause in &vow.clauses {
                if let VowClause::Requires { expr, span } = clause {
                    let ty = self.check_expr(expr);
                    if ty != Ty::Bool && ty != Ty::Never {
                        self.emit_error_with_hints(
                            ErrorCode::ContractTypeMismatch,
                            format!("`requires` clause has type `{ty}` but must be `bool`"),
                            *span,
                            vec!["function `requires` clauses must evaluate to `bool`".to_string()],
                        );
                    }
                }
            }
        }

        let body_ty = self.check_block(&fn_def.body);

        let expected = self.current_return_ty.clone();
        let coercible = body_ty == expected
            || body_ty == Ty::Never
            || (body_ty == Ty::I32 && expected.is_integer());
        if !coercible {
            self.emit_error_with_hints(
                ErrorCode::TypeMismatch,
                format!(
                    "function body has type `{body_ty}` but declared return type is `{expected}`"
                ),
                fn_def.body.span,
                vec![format!(
                    "function declares return type `{expected}`, but body evaluates to `{body_ty}`"
                )],
            );
        }

        if let Some(ref vow) = fn_def.vow {
            self.env.push_scope();
            self.env.define("result", self.current_return_ty.clone());
            for clause in &vow.clauses {
                if let VowClause::Ensures { expr, span } = clause {
                    let ty = self.check_expr(expr);
                    if ty != Ty::Bool && ty != Ty::Never {
                        self.emit_error_with_hints(
                            ErrorCode::ContractTypeMismatch,
                            format!("`ensures` clause has type `{ty}` but must be `bool`"),
                            *span,
                            vec!["function `ensures` clauses must evaluate to `bool`".to_string()],
                        );
                    }
                }
            }
            self.env.pop_scope();
        }

        self.env.pop_scope();

        crate::effects::check_fn_effects(fn_def, &self.env, &self.file, self.emitter);
        crate::linear::check_linear_usage(fn_def, &self.env, &self.file, self.emitter);

        self.current_fn_effects = outer_effects;
        self.current_return_ty = outer_return_ty;
    }

    fn check_block(&mut self, block: &Block) -> Ty {
        self.env.push_scope();
        for stmt in &block.stmts {
            self.check_stmt(stmt);
        }
        let ty = match &block.trailing_expr {
            Some(expr) => self.check_expr(expr),
            None => Ty::Unit,
        };
        self.env.pop_scope();
        ty
    }

    fn check_stmt(&mut self, stmt: &Stmt) {
        match stmt {
            Stmt::Let {
                pattern, ty, init, ..
            } => {
                let init_ty = self.check_expr(init);
                let binding_ty = if let Some(ann) = ty {
                    match self.env.resolve(ann) {
                        Ok(ann_ty) => {
                            let coercible = init_ty == Ty::Never
                                || ann_ty == init_ty
                                || (init_ty == Ty::I32 && ann_ty.is_integer());
                            if !coercible {
                                self.emit_error_with_hints(
                                    ErrorCode::TypeMismatch,
                                    format!(
                                        "let binding annotated as `{ann_ty}` but initializer has type `{init_ty}`"
                                    ),
                                    ann.span(),
                                    vec![format!(
                                        "annotation is `{ann_ty}`, but initializer has type `{init_ty}`"
                                    )],
                                );
                            }
                            if is_vec_raw_parts_copy_expr(init) && !is_flat_vec_ty(&ann_ty) {
                                self.emit_error_with_hints(
                                    ErrorCode::TypeMismatch,
                                    format!(
                                        "Vec::from_raw_parts_copy requires a flat scalar Vec<T>, found `{ann_ty}`"
                                    ),
                                    ann.span(),
                                    vec![
                                        "pointer-containing Vec payloads need a hand-written deep-copy wrapper".to_string(),
                                    ],
                                );
                            }
                            ann_ty
                        }
                        Err(msg) => {
                            self.emit_error(ErrorCode::TypeMismatch, msg, ann.span());
                            init_ty
                        }
                    }
                } else {
                    init_ty
                };
                self.bind_pattern(pattern, &binding_ty);
            }
            Stmt::Expr { expr, .. } => {
                self.check_expr(expr);
            }
        }
    }

    fn bind_pattern(&mut self, pat: &Pat, ty: &Ty) {
        match &pat.kind {
            PatKind::Ident { name, .. } => {
                self.env.define(name, ty.clone());
            }
            PatKind::Wildcard => {}
            PatKind::Tuple(pats) => {
                if let Ty::Tuple(tys) = ty
                    && pats.len() == tys.len()
                {
                    for (p, t) in pats.iter().zip(tys.iter()) {
                        self.bind_pattern(p, t);
                    }
                }
            }
            _ => {}
        }
    }

    fn check_expr(&mut self, expr: &Expr) -> Ty {
        let ty = self.check_expr_inner(expr);
        if ty == Ty::Str {
            self.string_exprs.insert(expr as *const Expr as usize);
        }
        ty
    }

    fn check_expr_inner(&mut self, expr: &Expr) -> Ty {
        match &expr.kind {
            ExprKind::Lit(lit) => match lit {
                Lit::Int(_) => Ty::I32,
                Lit::Float(_) => Ty::F64,
                Lit::Bool(_) => Ty::Bool,
                Lit::String(_) => Ty::Str,
            },
            ExprKind::Ident(name) => {
                if let Some(ty) = self.const_types.get(name.as_str()) {
                    return ty.clone();
                }
                match self.env.lookup(name) {
                    Some(ty) => ty.clone(),
                    None => {
                        let mut hints = Vec::new();
                        let candidates = self
                            .env
                            .all_var_names(MAX_HINT_CANDIDATES, MAX_HINT_IDENTIFIER_BYTES);
                        if let Some(suggestion) = suggest_similar(name, &candidates, 3) {
                            hints.push(format!("did you mean `{suggestion}`?"));
                        }
                        self.emit_error_with_hints(
                            ErrorCode::TypeMismatch,
                            format!("undefined variable `{name}`"),
                            expr.span,
                            hints,
                        );
                        Ty::Unit
                    }
                }
            }
            ExprKind::BinaryOp { op, lhs, rhs } => {
                let lhs_ty = self.check_expr(lhs);
                let rhs_ty = self.check_expr(rhs);
                match op {
                    BinOp::Add | BinOp::Sub | BinOp::Mul | BinOp::Div | BinOp::Rem => {
                        self.check_same_numeric(lhs_ty, rhs_ty, expr.span)
                    }
                    BinOp::AddChecked
                    | BinOp::SubChecked
                    | BinOp::MulChecked
                    | BinOp::DivChecked
                    | BinOp::RemChecked => self.check_same_numeric(lhs_ty, rhs_ty, expr.span),
                    BinOp::Eq | BinOp::Ne | BinOp::Lt | BinOp::Le | BinOp::Gt | BinOp::Ge => {
                        let coercible = (lhs_ty == Ty::I32 && rhs_ty.is_integer())
                            || (rhs_ty == Ty::I32 && lhs_ty.is_integer());
                        if lhs_ty != rhs_ty
                            && lhs_ty != Ty::Never
                            && rhs_ty != Ty::Never
                            && !coercible
                        {
                            self.emit_error_with_hints(
                                ErrorCode::TypeMismatch,
                                format!(
                                    "comparison operands have different types: `{lhs_ty}` and `{rhs_ty}`"
                                ),
                                expr.span,
                                vec![format!(
                                    "convert one operand so both sides have the same type"
                                )],
                            );
                        }
                        Ty::Bool
                    }
                    BinOp::BitAnd | BinOp::BitOr | BinOp::BitXor | BinOp::Shl | BinOp::Shr => {
                        self.check_same_integer(lhs_ty, rhs_ty, expr.span)
                    }
                    BinOp::And | BinOp::Or => {
                        if lhs_ty != Ty::Bool && lhs_ty != Ty::Never {
                            self.emit_error_with_hints(
                                ErrorCode::TypeMismatch,
                                format!("logical operator requires `bool`, found `{lhs_ty}`"),
                                lhs.span,
                                vec!["use `!= 0` to convert an integer to bool".to_string()],
                            );
                        }
                        if rhs_ty != Ty::Bool && rhs_ty != Ty::Never {
                            self.emit_error_with_hints(
                                ErrorCode::TypeMismatch,
                                format!("logical operator requires `bool`, found `{rhs_ty}`"),
                                rhs.span,
                                vec!["use `!= 0` to convert an integer to bool".to_string()],
                            );
                        }
                        Ty::Bool
                    }
                }
            }
            ExprKind::UnaryOp { op, operand } => {
                let operand_ty = self.check_expr(operand);
                match op {
                    UnOp::Neg => {
                        if operand_ty.is_unsigned() {
                            self.emit_error(
                                ErrorCode::TypeMismatch,
                                format!(
                                    "unary negation is not allowed on unsigned type `{operand_ty}`"
                                ),
                                operand.span,
                            );
                            Ty::Unit
                        } else if !operand_ty.is_numeric() && operand_ty != Ty::Never {
                            self.emit_error(
                                ErrorCode::TypeMismatch,
                                format!(
                                    "unary negation requires a numeric type, found `{operand_ty}`"
                                ),
                                operand.span,
                            );
                            Ty::Unit
                        } else {
                            operand_ty
                        }
                    }
                    UnOp::Not => {
                        if operand_ty != Ty::Bool && operand_ty != Ty::Never {
                            self.emit_error(
                                ErrorCode::TypeMismatch,
                                format!("logical not requires `bool`, found `{operand_ty}`"),
                                operand.span,
                            );
                        }
                        Ty::Bool
                    }
                }
            }
            ExprKind::Call { callee, args } => {
                let name = match &callee.kind {
                    ExprKind::Ident(n) => n.as_str(),
                    _ => {
                        for arg in args {
                            self.check_expr(arg);
                        }
                        return Ty::Unit;
                    }
                };
                if name == "pin_to_root" {
                    if args.len() != 1 {
                        self.emit_error_with_hints(
                            ErrorCode::TypeMismatch,
                            format!(
                                "function `pin_to_root` expects 1 argument but got {}",
                                args.len()
                            ),
                            expr.span,
                            vec!["expected signature: (heap_value)".to_string()],
                        );
                        for arg in args {
                            self.check_expr(arg);
                        }
                        return Ty::Unit;
                    }
                    let arg_ty = self.check_expr(&args[0]);
                    if !is_supported_pin_ty(&arg_ty) && arg_ty != Ty::Never {
                        self.emit_error_with_hints(
                            ErrorCode::TypeMismatch,
                            format!("pin_to_root does not support `{arg_ty}`"),
                            args[0].span,
                            vec![
                                "supported forms are String and Vec<T> where T is a flat scalar slot; pointer-containing values need a hand-written deep-copy wrapper".to_string(),
                            ],
                        );
                    }
                    return arg_ty;
                }
                let (param_tys, return_ty) = match self.env.lookup_fn(name) {
                    Some(sig) => (sig.params.clone(), sig.return_ty.clone()),
                    None => {
                        let mut hints = Vec::new();
                        let candidates = self
                            .env
                            .all_fn_names(MAX_HINT_CANDIDATES, MAX_HINT_IDENTIFIER_BYTES);
                        if let Some(suggestion) = suggest_similar(name, &candidates, 3) {
                            hints.push(format!("did you mean `{suggestion}`?"));
                        }
                        self.emit_error_with_hints(
                            ErrorCode::TypeMismatch,
                            format!("undefined function `{name}`"),
                            callee.span,
                            hints,
                        );
                        for arg in args {
                            self.check_expr(arg);
                        }
                        return Ty::Unit;
                    }
                };
                if args.len() != param_tys.len() {
                    let sig_str = param_tys
                        .iter()
                        .map(|t| format!("`{t}`"))
                        .collect::<Vec<_>>()
                        .join(", ");
                    self.emit_error_with_hints(
                        ErrorCode::TypeMismatch,
                        format!(
                            "function `{name}` expects {} arguments but got {}",
                            param_tys.len(),
                            args.len()
                        ),
                        expr.span,
                        vec![format!("expected signature: ({sig_str})")],
                    );
                }
                for (arg, expected_ty) in args.iter().zip(param_tys.iter()) {
                    let arg_ty = self.check_expr(arg);
                    let coercible =
                        arg_ty == Ty::I32 && expected_ty.is_integer() && *expected_ty != Ty::I32;
                    if arg_ty != *expected_ty && arg_ty != Ty::Never && !coercible {
                        self.emit_error_with_hints(
                            ErrorCode::TypeMismatch,
                            format!(
                                "argument has type `{arg_ty}` but function expects `{expected_ty}`"
                            ),
                            arg.span,
                            vec![format!("parameter expects `{expected_ty}`, got `{arg_ty}`")],
                        );
                    }
                }
                return_ty
            }
            ExprKind::MethodCall {
                receiver,
                method,
                args,
            } => {
                let recv_ty = self.check_expr(receiver);
                for arg in args {
                    self.check_expr(arg);
                }
                let is_str = matches!(recv_ty, Ty::Str);
                let is_vec = matches!(&recv_ty,
                    Ty::Applied(base, _) if matches!(base.as_ref(), Ty::Struct(n) if n == "Vec")
                );
                let is_hashmap = matches!(&recv_ty,
                    Ty::Applied(base, _) if matches!(base.as_ref(), Ty::Struct(n) if n == "HashMap")
                );
                let (known_methods, result_ty): (&[&str], Option<Ty>) = if is_str {
                    let methods: &[&str] = &[
                        "len",
                        "push_str",
                        "eq",
                        "contains",
                        "byte_at",
                        "push_byte",
                        "substring",
                        "parse_i64",
                        "parse_u64",
                        "clear",
                    ];
                    let ty = match method.as_str() {
                        "len" => Some(Ty::I64),
                        "push_str" => Some(Ty::Unit),
                        "clear" => Some(Ty::Unit),
                        "eq" => Some(Ty::Bool),
                        "contains" => Some(Ty::Bool),
                        "byte_at" => Some(Ty::I64),
                        "push_byte" => Some(Ty::Unit),
                        "substring" => Some(Ty::Str),
                        "parse_i64" => Some(Ty::Applied(
                            Box::new(Ty::Enum("Option".to_string())),
                            vec![Ty::I64],
                        )),
                        "parse_u64" => Some(Ty::Applied(
                            Box::new(Ty::Enum("Option".to_string())),
                            vec![Ty::U64],
                        )),
                        _ => None,
                    };
                    (methods, ty)
                } else if is_hashmap {
                    let methods: &[&str] = &["len", "insert", "get", "contains_key", "remove"];
                    let ty = match method.as_str() {
                        "len" => Some(Ty::I64),
                        "insert" => Some(Ty::Unit),
                        "get" => Some(Ty::I64),
                        "contains_key" => Some(Ty::Bool),
                        "remove" => Some(Ty::Unit),
                        _ => None,
                    };
                    (methods, ty)
                } else if is_vec {
                    let methods: &[&str] = &["len", "push", "pop", "get", "clear", "truncate"];
                    let ty = match method.as_str() {
                        "len" => Some(Ty::I64),
                        "push" => Some(Ty::Unit),
                        "pop" => Some(Ty::Unit),
                        "clear" => Some(Ty::Unit),
                        "truncate" => Some(Ty::Unit),
                        "get" => Some(Ty::Applied(
                            Box::new(Ty::Enum("Option".to_string())),
                            vec![if let Ty::Applied(_, args) = &recv_ty {
                                args.first().cloned().unwrap_or(Ty::I64)
                            } else {
                                Ty::I64
                            }],
                        )),
                        _ => None,
                    };
                    (methods, ty)
                } else if let Ty::Applied(base, type_args) = &recv_ty {
                    let is_option_or_result = matches!(
                        base.as_ref(),
                        Ty::Enum(n) if n == "Option" || n == "Result"
                    );
                    if is_option_or_result {
                        let methods: &[&str] = &["unwrap"];
                        let ty = match method.as_str() {
                            "unwrap" => Some(type_args.first().cloned().unwrap_or(Ty::Unit)),
                            _ => None,
                        };
                        (methods, ty)
                    } else {
                        (&[] as &[&str], None)
                    }
                } else {
                    (&[] as &[&str], None)
                };
                match result_ty {
                    Some(ty) => ty,
                    None => {
                        let type_name = if is_str {
                            "String".to_string()
                        } else if is_hashmap {
                            "HashMap".to_string()
                        } else if is_vec {
                            "Vec".to_string()
                        } else if matches!(&recv_ty, Ty::Applied(base, _) if matches!(base.as_ref(), Ty::Enum(n) if n == "Option"))
                        {
                            "Option".to_string()
                        } else if matches!(&recv_ty, Ty::Applied(base, _) if matches!(base.as_ref(), Ty::Enum(n) if n == "Result"))
                        {
                            "Result".to_string()
                        } else {
                            format!("{recv_ty}")
                        };
                        let candidates: Vec<String> =
                            known_methods.iter().map(|s| s.to_string()).collect();
                        let mut hints = Vec::new();
                        if let Some(s) = suggest_similar(method, &candidates, 3) {
                            hints.push(format!("did you mean `{s}`?"));
                        } else if !candidates.is_empty() {
                            hints.push(format!("available methods: {}", candidates.join(", ")));
                        }
                        self.emit_error_with_hints(
                            ErrorCode::UnknownMethod,
                            format!("unknown method `{method}` on type `{type_name}`"),
                            expr.span,
                            hints,
                        );
                        Ty::Unit
                    }
                }
            }
            ExprKind::FieldAccess { base, field } => {
                let base_ty = self.check_expr(base);
                let struct_name = match &base_ty {
                    Ty::Struct(n) => n.clone(),
                    Ty::Reference(inner) => match inner.as_ref() {
                        Ty::Struct(n) => n.clone(),
                        _ => {
                            self.emit_error(
                                ErrorCode::TypeMismatch,
                                "field access on non-struct type",
                                expr.span,
                            );
                            return Ty::Unit;
                        }
                    },
                    _ => {
                        self.emit_error(
                            ErrorCode::TypeMismatch,
                            format!("field access on non-struct type `{base_ty}`"),
                            expr.span,
                        );
                        return Ty::Unit;
                    }
                };
                match self.env.lookup_struct(&struct_name) {
                    Some(info) => match info.fields.iter().find(|(n, _)| n == field) {
                        Some((_, ty)) => ty.clone(),
                        None => {
                            let field_names: Vec<String> =
                                info.fields.iter().map(|(n, _)| n.clone()).collect();
                            let mut hints = Vec::new();
                            if let Some(s) = suggest_similar(field, &field_names, 3) {
                                hints.push(format!("did you mean `{s}`?"));
                            } else if !field_names.is_empty() {
                                hints.push(format!("available fields: {}", field_names.join(", ")));
                            }
                            self.emit_error_with_hints(
                                ErrorCode::TypeMismatch,
                                format!("struct `{struct_name}` has no field `{field}`"),
                                expr.span,
                                hints,
                            );
                            Ty::Unit
                        }
                    },
                    None => {
                        self.emit_error(
                            ErrorCode::TypeMismatch,
                            format!("unknown struct `{struct_name}`"),
                            expr.span,
                        );
                        Ty::Unit
                    }
                }
            }
            ExprKind::Index { base, index } => {
                let base_ty = self.check_expr(base);
                self.check_expr(index);
                match &base_ty {
                    Ty::Applied(_, args) => args.first().cloned().unwrap_or(Ty::Unit),
                    _ => {
                        self.emit_error_with_hints(
                            ErrorCode::TypeMismatch,
                            format!("index operation on non-indexable type `{base_ty}`"),
                            expr.span,
                            vec![
                                "indexing is supported on Vec<T>, HashMap<K,V>, and String"
                                    .to_string(),
                            ],
                        );
                        Ty::Unit
                    }
                }
            }
            ExprKind::Match { scrutinee, arms } => {
                let scrutinee_ty = self.check_expr(scrutinee);
                crate::exhaustiveness::check_exhaustive(
                    &scrutinee_ty,
                    arms,
                    &self.env,
                    expr.span,
                    &self.file,
                    self.emitter,
                );
                let mut result_ty = Ty::Unit;
                for (i, arm) in arms.iter().enumerate() {
                    self.env.push_scope();
                    self.bind_arm_pattern(&arm.pattern, &scrutinee_ty);
                    let arm_ty = self.check_expr(&arm.body);
                    self.env.pop_scope();
                    if i == 0 {
                        result_ty = arm_ty.clone();
                    } else if arm_ty != result_ty && arm_ty != Ty::Never && result_ty != Ty::Never {
                        let coercible = (arm_ty == Ty::I32 && result_ty.is_integer())
                            || (result_ty == Ty::I32 && arm_ty.is_integer());
                        if coercible {
                            if result_ty == Ty::I32 && arm_ty.is_integer() {
                                result_ty = arm_ty.clone();
                            }
                        } else {
                            self.emit_error(
                                ErrorCode::TypeMismatch,
                                format!(
                                    "match arm has type `{arm_ty}` but previous arms have type `{result_ty}`"
                                ),
                                arm.span,
                            );
                        }
                    }
                    if result_ty == Ty::Never && arm_ty != Ty::Never {
                        result_ty = arm_ty;
                    }
                }
                result_ty
            }
            ExprKind::If {
                condition,
                then_branch,
                else_branch,
            } => {
                let cond_ty = self.check_expr(condition);
                if cond_ty != Ty::Bool && cond_ty != Ty::Never {
                    self.emit_error_with_hints(
                        ErrorCode::TypeMismatch,
                        format!("if condition must be `bool`, found `{cond_ty}`"),
                        condition.span,
                        vec!["if condition must be `bool`".to_string()],
                    );
                }
                let then_ty = self.check_block(then_branch);
                match else_branch {
                    Some(else_expr) => {
                        let else_ty = self.check_expr(else_expr);
                        let compatible = then_ty == else_ty
                            || then_ty == Ty::Never
                            || else_ty == Ty::Never
                            || (then_ty == Ty::I32 && else_ty.is_integer())
                            || (else_ty == Ty::I32 && then_ty.is_integer());
                        if !compatible {
                            self.emit_error_with_hints(
                                ErrorCode::TypeMismatch,
                                format!(
                                    "if branches have different types: `{then_ty}` vs `{else_ty}`"
                                ),
                                expr.span,
                                vec![format!(
                                    "then branch has type `{then_ty}`, but else branch has type `{else_ty}`"
                                )],
                            );
                        }
                        if then_ty == Ty::Never {
                            else_ty
                        } else if else_ty == Ty::Never {
                            then_ty.clone()
                        } else if then_ty == Ty::I32 && else_ty.is_integer() {
                            else_ty
                        } else if else_ty == Ty::I32 && then_ty.is_integer() {
                            then_ty.clone()
                        } else {
                            then_ty
                        }
                    }
                    None => Ty::Unit,
                }
            }
            ExprKind::While {
                condition,
                vow,
                body,
            } => {
                self.check_expr(condition);
                if let Some(vow) = vow {
                    self.check_vow_clauses(vow, "while loop");
                }
                self.in_loop += 1;
                self.break_types_stack.push(None);
                self.check_block(body);
                self.break_types_stack.pop();
                self.in_loop -= 1;
                Ty::Unit
            }
            ExprKind::ForEach {
                binding,
                iterable,
                vow,
                body,
            } => {
                let iter_ty = self.check_expr(iterable);
                let elem_ty = match &iter_ty {
                    Ty::Applied(base, args) if **base == Ty::Struct("Vec".to_string()) => {
                        args.first().cloned().unwrap_or(Ty::I64)
                    }
                    Ty::Never => Ty::Never,
                    _ => {
                        self.emit_error(
                            ErrorCode::TypeMismatch,
                            format!("for-each requires `Vec<T>` iterable, got `{iter_ty}`"),
                            expr.span,
                        );
                        Ty::I64
                    }
                };
                self.env.push_scope();
                self.env.define(binding, elem_ty);
                if let Some(vow) = vow {
                    self.check_vow_clauses(vow, "for-each loop");
                }
                self.in_loop += 1;
                self.check_block(body);
                self.in_loop -= 1;
                self.env.pop_scope();
                Ty::Unit
            }
            ExprKind::Loop { vow, body } => {
                if let Some(vow) = vow {
                    self.check_vow_clauses(vow, "loop");
                }
                self.in_loop += 1;
                self.break_types_stack.push(Some(Vec::new()));
                self.check_block(body);
                let break_tys = self.break_types_stack.pop().unwrap();
                self.in_loop -= 1;
                if let Some(tys) = break_tys {
                    let mut result_ty = Ty::Unit;
                    let mut found = false;
                    for ty in &tys {
                        if *ty == Ty::Never {
                            continue;
                        }
                        if !found {
                            result_ty = ty.clone();
                            found = true;
                        } else {
                            let ok = *ty == result_ty
                                || (*ty == Ty::I32 && result_ty.is_integer())
                                || (result_ty == Ty::I32 && ty.is_integer());
                            if !ok {
                                self.emit_error(
                                    ErrorCode::TypeMismatch,
                                    format!(
                                        "break type mismatch: expected `{result_ty}`, found `{ty}`"
                                    ),
                                    expr.span,
                                );
                                break;
                            }
                        }
                    }
                    result_ty
                } else {
                    Ty::Unit
                }
            }
            ExprKind::Break { value } => {
                if self.in_loop == 0 {
                    self.emit_error(
                        ErrorCode::TypeMismatch,
                        "`break` outside of a loop",
                        expr.span,
                    );
                }
                let val_ty = if let Some(v) = value {
                    let ty = self.check_expr(v);
                    // break-with-value only allowed inside `loop`, not `while`
                    if let Some(top) = self.break_types_stack.last()
                        && top.is_none()
                    {
                        self.emit_error(
                            ErrorCode::TypeMismatch,
                            "`break` with a value is only allowed inside `loop`, not `while`",
                            expr.span,
                        );
                    }
                    ty
                } else {
                    Ty::Unit
                };
                if let Some(Some(tys)) = self.break_types_stack.last_mut() {
                    tys.push(val_ty);
                }
                Ty::Never
            }
            ExprKind::Continue => {
                if self.in_loop == 0 {
                    self.emit_error(
                        ErrorCode::TypeMismatch,
                        "`continue` outside of a loop",
                        expr.span,
                    );
                }
                Ty::Never
            }
            ExprKind::Return { value } => {
                let val_ty = match value {
                    Some(v) => self.check_expr(v),
                    None => Ty::Unit,
                };
                let expected = self.current_return_ty.clone();
                let coercible = val_ty == expected
                    || val_ty == Ty::Never
                    || (val_ty == Ty::I32 && expected.is_integer());
                if !coercible {
                    self.emit_error_with_hints(
                        ErrorCode::TypeMismatch,
                        format!(
                            "return type `{val_ty}` does not match declared return type `{expected}`"
                        ),
                        expr.span,
                        vec![format!("function return type is `{expected}`")],
                    );
                }
                Ty::Never
            }
            ExprKind::Block(block) => self.check_block(block),
            ExprKind::Borrow { expr: inner } => {
                let inner_ty = self.check_expr(inner);
                Ty::Reference(Box::new(inner_ty))
            }
            ExprKind::Question { expr: inner } => {
                let inner_ty = self.check_expr(inner);
                match &inner_ty {
                    Ty::Applied(base, args) if matches!(base.as_ref(), Ty::Enum(n) if n == "Option") => {
                        args.first().cloned().unwrap_or(Ty::Unit)
                    }
                    Ty::Applied(base, args) if matches!(base.as_ref(), Ty::Enum(n) if n == "Result") => {
                        args.first().cloned().unwrap_or(Ty::Unit)
                    }
                    Ty::Never => Ty::Never,
                    _ => {
                        self.emit_error_with_hints(
                            ErrorCode::TypeMismatch,
                            format!(
                                "the `?` operator requires `Option<T>` or `Result<T,E>`, found `{inner_ty}`"
                            ),
                            inner.span,
                            vec![
                                "`?` unwraps Option or Result, propagating None/Err to the caller"
                                    .to_string(),
                            ],
                        );
                        Ty::Unit
                    }
                }
            }
            ExprKind::Assign { lhs, rhs } => {
                let lhs_ty = self.check_expr(lhs);
                let rhs_ty = self.check_expr(rhs);
                let coercible = (rhs_ty == Ty::I32 && lhs_ty.is_integer())
                    || lhs_ty == rhs_ty
                    || lhs_ty == Ty::Never
                    || rhs_ty == Ty::Never;
                if !coercible {
                    self.emit_error(
                        ErrorCode::TypeMismatch,
                        format!(
                            "assignment type mismatch: left is `{lhs_ty}` but right is `{rhs_ty}`"
                        ),
                        expr.span,
                    );
                }
                Ty::Unit
            }
            ExprKind::Tuple(elems) => {
                let elem_tys: Vec<Ty> = elems.iter().map(|e| self.check_expr(e)).collect();
                Ty::Tuple(elem_tys)
            }
            ExprKind::Result => self.current_return_ty.clone(),
            ExprKind::StructLiteral { name, fields } => {
                let info = self.env.lookup_struct(name).cloned();
                match info {
                    None => {
                        let candidates = self
                            .env
                            .all_struct_names(MAX_HINT_CANDIDATES, MAX_HINT_IDENTIFIER_BYTES);
                        let mut hints = Vec::new();
                        if let Some(s) = suggest_similar(name, &candidates, 3) {
                            hints.push(format!("did you mean `{s}`?"));
                        }
                        self.emit_error_with_hints(
                            ErrorCode::TypeMismatch,
                            format!("unknown struct `{name}`"),
                            expr.span,
                            hints,
                        );
                        for (_, e) in fields {
                            self.check_expr(e);
                        }
                        Ty::Unit
                    }
                    Some(info) => {
                        for (field_name, field_expr) in fields {
                            let actual_ty = self.check_expr(field_expr);
                            if let Some((_, expected_ty)) =
                                info.fields.iter().find(|(n, _)| n == field_name)
                            {
                                if actual_ty != *expected_ty
                                    && actual_ty != Ty::Never
                                    && actual_ty != Ty::I32
                                {
                                    self.emit_error(
                                        ErrorCode::TypeMismatch,
                                        format!(
                                            "field `{field_name}` of struct `{name}` expects `{expected_ty}`, found `{actual_ty}`"
                                        ),
                                        field_expr.span,
                                    );
                                }
                            } else {
                                self.emit_error(
                                    ErrorCode::TypeMismatch,
                                    format!("struct `{name}` has no field `{field_name}`"),
                                    field_expr.span,
                                );
                            }
                        }
                        Ty::Struct(name.clone())
                    }
                }
            }
            ExprKind::Cast { expr, target_ty } => {
                let src_ty = self.check_expr(expr);
                let tgt_ty = match target_ty.as_ref() {
                    vow_syntax::ast::Type::Named { name, .. } => {
                        Ty::from_primitive_name(name).unwrap_or(Ty::Unit)
                    }
                    _ => Ty::Unit,
                };
                let valid = matches!(
                    (&src_ty, &tgt_ty),
                    (Ty::I64, Ty::U64)
                        | (Ty::U64, Ty::I64)
                        | (Ty::I32, Ty::U64)
                        | (Ty::I32, Ty::I64)
                );
                if !valid && src_ty != Ty::Never {
                    self.emit_error_with_hints(
                        ErrorCode::TypeMismatch,
                        format!("cannot cast `{src_ty}` to `{tgt_ty}`"),
                        expr.span,
                        vec!["only i64 <-> u64 casts are allowed".to_string()],
                    );
                }
                tgt_ty
            }
            ExprKind::EnumConstruct { path, fields } => {
                let enum_name = path.first().map(|s| s.as_str()).unwrap_or("");
                let variant_name = path.get(1).map(|s| s.as_str()).unwrap_or("");
                // Handle compiler-known builtins: Option, Result, Vec, String, HashMap
                match (enum_name, variant_name) {
                    ("String", "from") => {
                        if let Some(arg) = fields.first() {
                            self.check_expr(arg);
                        }
                        return Ty::Str;
                    }
                    ("String", "from_raw_parts_copy") => {
                        if fields.len() != 2 {
                            self.emit_error_with_hints(
                                ErrorCode::TypeMismatch,
                                format!(
                                    "String::from_raw_parts_copy expects 2 arguments but got {}",
                                    fields.len()
                                ),
                                expr.span,
                                vec![
                                    "expected signature: (ptr: i64, len: i64) -> String"
                                        .to_string(),
                                ],
                            );
                        }
                        for field in fields {
                            let arg_ty = self.check_expr(field);
                            if arg_ty != Ty::I64 && arg_ty != Ty::I32 && arg_ty != Ty::Never {
                                self.emit_error_with_hints(
                                    ErrorCode::TypeMismatch,
                                    format!(
                                        "String::from_raw_parts_copy argument has type `{arg_ty}` but expects `i64`"
                                    ),
                                    field.span,
                                    vec![
                                        "raw pointers and lengths cross the FFI boundary as i64"
                                            .to_string(),
                                    ],
                                );
                            }
                        }
                        return Ty::Str;
                    }
                    ("String", "new") => {
                        return Ty::Str;
                    }
                    ("HashMap", "new") => {
                        return Ty::Never;
                    }
                    ("Option", "None") => {
                        // None has type Never (bottom) so it unifies with any Option<T>
                        return Ty::Never;
                    }
                    ("Option", "Some") => {
                        let payload_ty = fields
                            .first()
                            .map(|e| self.check_expr(e))
                            .unwrap_or(Ty::Unit);
                        return Ty::Applied(
                            Box::new(Ty::Enum("Option".to_string())),
                            vec![payload_ty],
                        );
                    }
                    ("Result", "Ok") => {
                        let payload_ty = fields
                            .first()
                            .map(|e| self.check_expr(e))
                            .unwrap_or(Ty::Unit);
                        return Ty::Applied(
                            Box::new(Ty::Enum("Result".to_string())),
                            vec![payload_ty, Ty::Unit],
                        );
                    }
                    ("Result", "Err") => {
                        let payload_ty = fields
                            .first()
                            .map(|e| self.check_expr(e))
                            .unwrap_or(Ty::Unit);
                        // Err has unknown Ok type; use Never so it unifies with any Result<T,E>
                        return Ty::Applied(
                            Box::new(Ty::Enum("Result".to_string())),
                            vec![Ty::Never, payload_ty],
                        );
                    }
                    ("Vec", "new") => {
                        // Vec::new() returns Vec<T> with unknown element type (Never)
                        return Ty::Never;
                    }
                    ("Vec", "from_raw_parts_copy") => {
                        if fields.len() != 2 {
                            self.emit_error_with_hints(
                                ErrorCode::TypeMismatch,
                                format!(
                                    "Vec::from_raw_parts_copy expects 2 arguments but got {}",
                                    fields.len()
                                ),
                                expr.span,
                                vec![
                                    "expected signature: (ptr: i64, len: i64) -> Vec<T>"
                                        .to_string(),
                                ],
                            );
                        }
                        for field in fields {
                            let arg_ty = self.check_expr(field);
                            if arg_ty != Ty::I64 && arg_ty != Ty::I32 && arg_ty != Ty::Never {
                                self.emit_error_with_hints(
                                    ErrorCode::TypeMismatch,
                                    format!(
                                        "Vec::from_raw_parts_copy argument has type `{arg_ty}` but expects `i64`"
                                    ),
                                    field.span,
                                    vec![
                                        "raw pointers and lengths cross the FFI boundary as i64"
                                            .to_string(),
                                    ],
                                );
                            }
                        }
                        return Ty::Never;
                    }
                    _ => {}
                }
                let info = self.env.lookup_enum(enum_name).cloned();
                match info {
                    None => {
                        self.emit_error(
                            ErrorCode::TypeMismatch,
                            format!("unknown enum `{enum_name}`"),
                            expr.span,
                        );
                        for e in fields {
                            self.check_expr(e);
                        }
                        Ty::Unit
                    }
                    Some(info) => {
                        let variant = info
                            .variants
                            .iter()
                            .find(|v| v.name == variant_name)
                            .cloned();
                        match variant {
                            None => {
                                self.emit_error(
                                    ErrorCode::TypeMismatch,
                                    format!("enum `{enum_name}` has no variant `{variant_name}`"),
                                    expr.span,
                                );
                                for e in fields {
                                    self.check_expr(e);
                                }
                            }
                            Some(variant) => {
                                use crate::env::VariantKind;
                                let expected_tys: Vec<Ty> = match &variant.kind {
                                    VariantKind::Unit => vec![],
                                    VariantKind::Tuple(tys) => tys.clone(),
                                    VariantKind::Struct(fields) => {
                                        fields.iter().map(|(_, ty)| ty.clone()).collect()
                                    }
                                };
                                for (i, field_expr) in fields.iter().enumerate() {
                                    let actual_ty = self.check_expr(field_expr);
                                    if let Some(expected_ty) = expected_tys.get(i)
                                        && actual_ty != *expected_ty
                                        && actual_ty != Ty::Never
                                        && actual_ty != Ty::I32
                                    {
                                        self.emit_error(
                                            ErrorCode::TypeMismatch,
                                            format!(
                                                "variant `{variant_name}` field {i} expects `{expected_ty}`, found `{actual_ty}`"
                                            ),
                                            field_expr.span,
                                        );
                                    }
                                }
                            }
                        }
                        Ty::Enum(enum_name.to_string())
                    }
                }
            }
        }
    }

    fn check_same_numeric(&mut self, lhs: Ty, rhs: Ty, op_span: Span) -> Ty {
        if lhs == Ty::Never {
            return rhs;
        }
        if rhs == Ty::Never {
            return lhs;
        }
        // Integer literal (I32) coerces to the other integer type
        let (lhs, rhs) = if lhs == Ty::I32 && rhs.is_integer() {
            (rhs.clone(), rhs)
        } else if rhs == Ty::I32 && lhs.is_integer() {
            (lhs.clone(), lhs)
        } else {
            (lhs, rhs)
        };
        if !lhs.is_numeric() {
            self.emit_error_with_hints(
                ErrorCode::TypeMismatch,
                format!("arithmetic operator requires a numeric type, found `{lhs}`"),
                op_span,
                vec!["arithmetic operators require numeric operands".to_string()],
            );
            return Ty::Unit;
        }
        if lhs != rhs {
            self.emit_error_with_hints(
                ErrorCode::TypeMismatch,
                format!("arithmetic operands have different types: `{lhs}` and `{rhs}`"),
                op_span,
                vec!["operator requires matching types".to_string()],
            );
            return Ty::Unit;
        }
        lhs
    }

    fn check_same_integer(&mut self, lhs: Ty, rhs: Ty, op_span: Span) -> Ty {
        if lhs == Ty::Never {
            return rhs;
        }
        if rhs == Ty::Never {
            return lhs;
        }
        let (lhs, rhs) = if lhs == Ty::I32 && rhs.is_integer() {
            (rhs.clone(), rhs)
        } else if rhs == Ty::I32 && lhs.is_integer() {
            (lhs.clone(), lhs)
        } else {
            (lhs, rhs)
        };
        if !lhs.is_integer() {
            self.emit_error_with_hints(
                ErrorCode::TypeMismatch,
                format!("bitwise operator requires an integer type, found `{lhs}`"),
                op_span,
                vec!["bitwise operators require integer operands".to_string()],
            );
            return Ty::Unit;
        }
        if lhs != rhs {
            self.emit_error_with_hints(
                ErrorCode::TypeMismatch,
                format!("bitwise operands have different types: `{lhs}` and `{rhs}`"),
                op_span,
                vec!["operator requires matching integer types".to_string()],
            );
            return Ty::Unit;
        }
        lhs
    }

    fn bind_arm_pattern(&mut self, pat: &Pat, scrutinee_ty: &Ty) {
        match &pat.kind {
            PatKind::Ident { name, .. } => {
                self.env.define(name, scrutinee_ty.clone());
            }
            PatKind::Wildcard => {}
            PatKind::Tuple(pats) => {
                if let Ty::Tuple(tys) = scrutinee_ty
                    && pats.len() == tys.len()
                {
                    for (p, t) in pats.iter().zip(tys.iter()) {
                        self.bind_arm_pattern(p, t);
                    }
                }
            }
            PatKind::EnumVariant { path, inner } => {
                let variant_name = match path.last() {
                    Some(n) => n.as_str(),
                    None => return,
                };
                let enum_name = match scrutinee_ty {
                    Ty::Enum(n) => n.clone(),
                    Ty::Applied(base, _) => match base.as_ref() {
                        Ty::Enum(n) => n.clone(),
                        _ => return,
                    },
                    _ => return,
                };
                let variant_tys: Vec<Ty> = self
                    .env
                    .lookup_enum(&enum_name)
                    .and_then(|info| {
                        info.variants
                            .iter()
                            .find(|v| v.name.as_str() == variant_name)
                            .map(|v| match &v.kind {
                                VariantKind::Tuple(tys) => tys.clone(),
                                _ => vec![],
                            })
                    })
                    .unwrap_or_default();
                for (p, t) in inner.iter().zip(variant_tys.iter()) {
                    self.bind_arm_pattern(p, t);
                }
            }
            PatKind::Struct { name, fields } => {
                let struct_ty = Ty::Struct(name.clone());
                for (field_name, field_pat) in fields {
                    let field_ty = match self.env.lookup_struct(name) {
                        Some(info) => info
                            .fields
                            .iter()
                            .find(|(n, _)| n == field_name)
                            .map(|(_, t)| t.clone())
                            .unwrap_or(Ty::Unit),
                        None => Ty::Unit,
                    };
                    self.bind_arm_pattern(field_pat, &field_ty);
                }
                let _ = struct_ty;
            }
            PatKind::Or(pats) => {
                for p in pats {
                    self.bind_arm_pattern(p, scrutinee_ty);
                }
            }
            PatKind::Lit(_) => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use vow_diag::Diagnostic;
    use vow_syntax::ast::{
        BinOp, Block, Expr, ExprKind, FnDef, Item, Lit, Module, Param, Type, Visibility,
    };
    use vow_syntax::span::Span;

    struct TestEmitter(Vec<Diagnostic>);

    impl DiagnosticEmitter for TestEmitter {
        fn emit(&mut self, d: &Diagnostic) {
            self.0.push(d.clone());
        }
        fn finish(&mut self) {}
    }

    fn dummy_span() -> Span {
        Span::new(0, 1)
    }

    fn make_expr(kind: ExprKind) -> Expr {
        Expr {
            kind,
            span: dummy_span(),
        }
    }

    #[test]
    fn type_check_i32_literal() {
        let mut emitter = TestEmitter(vec![]);
        let mut checker = Checker::new("test.vow", &mut emitter);
        checker.env.push_scope();
        let ty = checker.check_expr(&make_expr(ExprKind::Lit(Lit::Int(42))));
        assert_eq!(ty, Ty::I32);
        assert!(!checker.has_errors());
    }

    #[test]
    fn type_check_bool_literal() {
        let mut emitter = TestEmitter(vec![]);
        let mut checker = Checker::new("test.vow", &mut emitter);
        checker.env.push_scope();
        let ty = checker.check_expr(&make_expr(ExprKind::Lit(Lit::Bool(true))));
        assert_eq!(ty, Ty::Bool);
        assert!(!checker.has_errors());
    }

    #[test]
    fn type_check_undefined_variable() {
        let mut emitter = TestEmitter(vec![]);
        let mut checker = Checker::new("test.vow", &mut emitter);
        checker.env.push_scope();
        let ty = checker.check_expr(&make_expr(ExprKind::Ident("x".to_string())));
        assert_eq!(ty, Ty::Unit);
        assert!(checker.has_errors());
        assert_eq!(emitter.0[0].code, ErrorCode::TypeMismatch);
        assert!(emitter.0[0].message.contains("undefined variable"));
    }

    #[test]
    fn type_check_binary_add() {
        let mut emitter = TestEmitter(vec![]);
        let mut checker = Checker::new("test.vow", &mut emitter);
        checker.env.push_scope();
        let expr = make_expr(ExprKind::BinaryOp {
            op: BinOp::Add,
            lhs: Box::new(make_expr(ExprKind::Lit(Lit::Int(1)))),
            rhs: Box::new(make_expr(ExprKind::Lit(Lit::Int(2)))),
        });
        let ty = checker.check_expr(&expr);
        assert_eq!(ty, Ty::I32);
        assert!(!checker.has_errors());
    }

    #[test]
    fn type_check_binary_add_type_mismatch() {
        let mut emitter = TestEmitter(vec![]);
        let mut checker = Checker::new("test.vow", &mut emitter);
        checker.env.push_scope();
        let expr = make_expr(ExprKind::BinaryOp {
            op: BinOp::Add,
            lhs: Box::new(make_expr(ExprKind::Lit(Lit::Int(1)))),
            rhs: Box::new(make_expr(ExprKind::Lit(Lit::Bool(true)))),
        });
        checker.check_expr(&expr);
        assert!(checker.has_errors());
        assert_eq!(emitter.0[0].code, ErrorCode::TypeMismatch);
    }

    #[test]
    fn type_check_if_else_type_mismatch() {
        let mut emitter = TestEmitter(vec![]);
        let mut checker = Checker::new("test.vow", &mut emitter);
        checker.env.push_scope();
        let then_block = Block {
            stmts: vec![],
            trailing_expr: Some(Box::new(make_expr(ExprKind::Lit(Lit::Int(1))))),
            span: dummy_span(),
        };
        let else_expr = make_expr(ExprKind::Lit(Lit::Bool(true)));
        let expr = make_expr(ExprKind::If {
            condition: Box::new(make_expr(ExprKind::Lit(Lit::Bool(true)))),
            then_branch: Box::new(then_block),
            else_branch: Some(Box::new(else_expr)),
        });
        checker.check_expr(&expr);
        assert!(checker.has_errors());
        assert_eq!(emitter.0[0].code, ErrorCode::TypeMismatch);
    }

    #[test]
    fn type_check_simple_fn() {
        let mut emitter = TestEmitter(vec![]);
        let mut checker = Checker::new("test.vow", &mut emitter);

        let body = Block {
            stmts: vec![],
            trailing_expr: Some(Box::new(make_expr(ExprKind::BinaryOp {
                op: BinOp::Add,
                lhs: Box::new(make_expr(ExprKind::Ident("x".to_string()))),
                rhs: Box::new(make_expr(ExprKind::Ident("y".to_string()))),
            }))),
            span: dummy_span(),
        };

        let fn_def = FnDef {
            vis: Visibility::Public,
            name: "add".to_string(),
            params: vec![
                Param {
                    name: "x".to_string(),
                    ty: Type::Named {
                        name: "i32".to_string(),
                        span: dummy_span(),
                    },
                    refinement: None,
                    span: dummy_span(),
                },
                Param {
                    name: "y".to_string(),
                    ty: Type::Named {
                        name: "i32".to_string(),
                        span: dummy_span(),
                    },
                    refinement: None,
                    span: dummy_span(),
                },
            ],
            return_ty: Type::Named {
                name: "i32".to_string(),
                span: dummy_span(),
            },
            effects: vec![],
            vow: None,
            is_declaration: false,
            body,
            span: dummy_span(),
        };

        let module = Module {
            name: "test".to_string(),
            uses: vec![],
            items: vec![Item::Fn(fn_def)],
            span: dummy_span(),
        };

        checker.check_module(&module);
        assert!(!checker.has_errors());
    }

    // --- helpers for the extended tests ---

    fn make_block(trailing: Expr) -> Block {
        Block {
            stmts: vec![],
            trailing_expr: Some(Box::new(trailing)),
            span: dummy_span(),
        }
    }

    fn unit_block() -> Block {
        Block {
            stmts: vec![],
            trailing_expr: None,
            span: dummy_span(),
        }
    }

    fn make_param(name: &str, ty_name: &str) -> Param {
        Param {
            name: name.to_string(),
            ty: Type::Named {
                name: ty_name.to_string(),
                span: dummy_span(),
            },
            refinement: None,
            span: dummy_span(),
        }
    }

    fn int_lit() -> Expr {
        make_expr(ExprKind::Lit(Lit::Int(1)))
    }

    fn bool_lit() -> Expr {
        make_expr(ExprKind::Lit(Lit::Bool(true)))
    }

    fn float_lit() -> Expr {
        make_expr(ExprKind::Lit(Lit::Float(1.0)))
    }

    fn ident(name: &str) -> Expr {
        make_expr(ExprKind::Ident(name.to_string()))
    }

    fn new_checker<'e>(emitter: &'e mut TestEmitter) -> Checker<'e> {
        let mut c = Checker::new("test.vow", emitter);
        c.env.push_scope();
        c
    }

    // --- UnaryOp ---

    #[test]
    fn unary_neg_numeric_ok() {
        let mut emitter = TestEmitter(vec![]);
        let mut checker = new_checker(&mut emitter);
        let ty = checker.check_expr(&make_expr(ExprKind::UnaryOp {
            op: vow_syntax::ast::UnOp::Neg,
            operand: Box::new(int_lit()),
        }));
        assert_eq!(ty, Ty::I32);
        assert!(!checker.has_errors());
    }

    #[test]
    fn unary_neg_non_numeric_error() {
        let mut emitter = TestEmitter(vec![]);
        let mut checker = new_checker(&mut emitter);
        checker.check_expr(&make_expr(ExprKind::UnaryOp {
            op: vow_syntax::ast::UnOp::Neg,
            operand: Box::new(bool_lit()),
        }));
        assert!(checker.has_errors());
        assert!(emitter.0[0].message.contains("numeric"));
    }

    #[test]
    fn unary_not_bool_ok() {
        let mut emitter = TestEmitter(vec![]);
        let mut checker = new_checker(&mut emitter);
        let ty = checker.check_expr(&make_expr(ExprKind::UnaryOp {
            op: vow_syntax::ast::UnOp::Not,
            operand: Box::new(bool_lit()),
        }));
        assert_eq!(ty, Ty::Bool);
        assert!(!checker.has_errors());
    }

    #[test]
    fn unary_not_non_bool_error() {
        let mut emitter = TestEmitter(vec![]);
        let mut checker = new_checker(&mut emitter);
        checker.check_expr(&make_expr(ExprKind::UnaryOp {
            op: vow_syntax::ast::UnOp::Not,
            operand: Box::new(int_lit()),
        }));
        assert!(checker.has_errors());
    }

    // --- Comparison and logical operators ---

    #[test]
    fn comparison_eq_same_type_ok() {
        let mut emitter = TestEmitter(vec![]);
        let mut checker = new_checker(&mut emitter);
        let ty = checker.check_expr(&make_expr(ExprKind::BinaryOp {
            op: BinOp::Eq,
            lhs: Box::new(int_lit()),
            rhs: Box::new(int_lit()),
        }));
        assert_eq!(ty, Ty::Bool);
        assert!(!checker.has_errors());
    }

    #[test]
    fn comparison_eq_type_mismatch_error() {
        let mut emitter = TestEmitter(vec![]);
        let mut checker = new_checker(&mut emitter);
        checker.check_expr(&make_expr(ExprKind::BinaryOp {
            op: BinOp::Eq,
            lhs: Box::new(int_lit()),
            rhs: Box::new(bool_lit()),
        }));
        assert!(checker.has_errors());
    }

    #[test]
    fn logical_and_bool_ok() {
        let mut emitter = TestEmitter(vec![]);
        let mut checker = new_checker(&mut emitter);
        let ty = checker.check_expr(&make_expr(ExprKind::BinaryOp {
            op: BinOp::And,
            lhs: Box::new(bool_lit()),
            rhs: Box::new(bool_lit()),
        }));
        assert_eq!(ty, Ty::Bool);
        assert!(!checker.has_errors());
    }

    #[test]
    fn logical_and_non_bool_error() {
        let mut emitter = TestEmitter(vec![]);
        let mut checker = new_checker(&mut emitter);
        checker.check_expr(&make_expr(ExprKind::BinaryOp {
            op: BinOp::And,
            lhs: Box::new(int_lit()),
            rhs: Box::new(bool_lit()),
        }));
        assert!(checker.has_errors());
    }

    #[test]
    fn bitwise_and_integer_ok() {
        let mut emitter = TestEmitter(vec![]);
        let mut checker = new_checker(&mut emitter);
        let ty = checker.check_expr(&make_expr(ExprKind::BinaryOp {
            op: BinOp::BitAnd,
            lhs: Box::new(int_lit()),
            rhs: Box::new(int_lit()),
        }));
        assert_eq!(ty, Ty::I32);
        assert!(!checker.has_errors());
    }

    #[test]
    fn bitwise_or_float_error() {
        let mut emitter = TestEmitter(vec![]);
        let mut checker = new_checker(&mut emitter);
        checker.check_expr(&make_expr(ExprKind::BinaryOp {
            op: BinOp::BitOr,
            lhs: Box::new(float_lit()),
            rhs: Box::new(float_lit()),
        }));
        assert!(checker.has_errors());
    }

    #[test]
    fn shift_returns_integer_type() {
        let mut emitter = TestEmitter(vec![]);
        let mut checker = new_checker(&mut emitter);
        let ty = checker.check_expr(&make_expr(ExprKind::BinaryOp {
            op: BinOp::Shl,
            lhs: Box::new(int_lit()),
            rhs: Box::new(int_lit()),
        }));
        assert_eq!(ty, Ty::I32);
        assert!(!checker.has_errors());
    }

    // --- Checked arithmetic ---

    #[test]
    fn checked_add_returns_integer_type() {
        let mut emitter = TestEmitter(vec![]);
        let mut checker = new_checker(&mut emitter);
        let ty = checker.check_expr(&make_expr(ExprKind::BinaryOp {
            op: BinOp::AddChecked,
            lhs: Box::new(int_lit()),
            rhs: Box::new(int_lit()),
        }));
        assert_eq!(ty, Ty::I32);
        assert!(!checker.has_errors());
    }

    // --- Call expression ---

    #[test]
    fn call_unknown_function_error() {
        let mut emitter = TestEmitter(vec![]);
        let mut checker = new_checker(&mut emitter);
        checker.check_expr(&make_expr(ExprKind::Call {
            callee: Box::new(ident("unknown_fn")),
            args: vec![],
        }));
        assert!(checker.has_errors());
        assert!(emitter.0[0].message.contains("undefined function"));
    }

    #[test]
    fn call_wrong_arg_count_error() {
        let mut emitter = TestEmitter(vec![]);
        let mut checker = Checker::new("test.vow", &mut emitter);
        use crate::env::FnSig;
        use std::collections::BTreeSet;
        checker.env.define_fn(
            "my_fn",
            FnSig {
                params: vec![Ty::I32],
                return_ty: Ty::Bool,
                effects: BTreeSet::new(),
            },
        );
        checker.env.push_scope();
        checker.check_expr(&make_expr(ExprKind::Call {
            callee: Box::new(ident("my_fn")),
            args: vec![],
        }));
        assert!(checker.has_errors());
        assert!(emitter.0[0].message.contains("expects"));
    }

    #[test]
    fn call_arg_type_mismatch_error() {
        let mut emitter = TestEmitter(vec![]);
        let mut checker = Checker::new("test.vow", &mut emitter);
        use crate::env::FnSig;
        use std::collections::BTreeSet;
        checker.env.define_fn(
            "my_fn",
            FnSig {
                params: vec![Ty::I32],
                return_ty: Ty::Unit,
                effects: BTreeSet::new(),
            },
        );
        checker.env.push_scope();
        checker.check_expr(&make_expr(ExprKind::Call {
            callee: Box::new(ident("my_fn")),
            args: vec![bool_lit()],
        }));
        assert!(checker.has_errors());
        assert!(emitter.0[0].message.contains("argument has type"));
    }

    #[test]
    fn call_correct_args_returns_return_type() {
        let mut emitter = TestEmitter(vec![]);
        let mut checker = Checker::new("test.vow", &mut emitter);
        use crate::env::FnSig;
        use std::collections::BTreeSet;
        checker.env.define_fn(
            "my_fn",
            FnSig {
                params: vec![Ty::I32],
                return_ty: Ty::Bool,
                effects: BTreeSet::new(),
            },
        );
        checker.env.push_scope();
        let ty = checker.check_expr(&make_expr(ExprKind::Call {
            callee: Box::new(ident("my_fn")),
            args: vec![int_lit()],
        }));
        assert_eq!(ty, Ty::Bool);
        assert!(!checker.has_errors());
    }

    // --- MethodCall ---

    #[test]
    fn method_call_unknown_method_errors() {
        let mut emitter = TestEmitter(vec![]);
        let mut checker = new_checker(&mut emitter);
        let ty = checker.check_expr(&make_expr(ExprKind::MethodCall {
            receiver: Box::new(int_lit()),
            method: "to_string".to_string(),
            args: vec![],
        }));
        assert_eq!(ty, Ty::Unit);
        assert!(checker.has_errors());
    }

    // --- FieldAccess ---

    #[test]
    fn field_access_struct_ok() {
        let mut emitter = TestEmitter(vec![]);
        let mut checker = Checker::new("test.vow", &mut emitter);
        use crate::env::StructInfo;
        checker.env.define_struct(
            "Point",
            StructInfo {
                fields: vec![("x".to_string(), Ty::I32), ("y".to_string(), Ty::I32)],
                is_linear: false,
            },
        );
        checker.env.push_scope();
        checker.env.define("p", Ty::Struct("Point".to_string()));
        let ty = checker.check_expr(&make_expr(ExprKind::FieldAccess {
            base: Box::new(ident("p")),
            field: "x".to_string(),
        }));
        assert_eq!(ty, Ty::I32);
        assert!(!checker.has_errors());
    }

    #[test]
    fn field_access_missing_field_error() {
        let mut emitter = TestEmitter(vec![]);
        let mut checker = Checker::new("test.vow", &mut emitter);
        use crate::env::StructInfo;
        checker.env.define_struct(
            "Point",
            StructInfo {
                fields: vec![("x".to_string(), Ty::I32)],
                is_linear: false,
            },
        );
        checker.env.push_scope();
        checker.env.define("p", Ty::Struct("Point".to_string()));
        checker.check_expr(&make_expr(ExprKind::FieldAccess {
            base: Box::new(ident("p")),
            field: "z".to_string(),
        }));
        assert!(checker.has_errors());
        assert!(emitter.0[0].message.contains("no field"));
    }

    #[test]
    fn field_access_non_struct_error() {
        let mut emitter = TestEmitter(vec![]);
        let mut checker = new_checker(&mut emitter);
        checker.check_expr(&make_expr(ExprKind::FieldAccess {
            base: Box::new(int_lit()),
            field: "x".to_string(),
        }));
        assert!(checker.has_errors());
        assert!(emitter.0[0].message.contains("non-struct"));
    }

    // --- Index ---

    #[test]
    fn index_applied_returns_element_type() {
        let mut emitter = TestEmitter(vec![]);
        let mut checker = Checker::new("test.vow", &mut emitter);
        use crate::env::StructInfo;
        checker.env.define_struct(
            "Vec",
            StructInfo {
                fields: vec![],
                is_linear: false,
            },
        );
        checker.env.push_scope();
        let vec_i32 = Ty::Applied(Box::new(Ty::Struct("Vec".to_string())), vec![Ty::I32]);
        checker.env.define("v", vec_i32);
        let ty = checker.check_expr(&make_expr(ExprKind::Index {
            base: Box::new(ident("v")),
            index: Box::new(int_lit()),
        }));
        assert_eq!(ty, Ty::I32);
        assert!(!checker.has_errors());
    }

    #[test]
    fn index_non_indexable_error() {
        let mut emitter = TestEmitter(vec![]);
        let mut checker = new_checker(&mut emitter);
        checker.check_expr(&make_expr(ExprKind::Index {
            base: Box::new(bool_lit()),
            index: Box::new(int_lit()),
        }));
        assert!(checker.has_errors());
        assert!(emitter.0[0].message.contains("non-indexable"));
    }

    // --- Borrow ---

    #[test]
    fn borrow_produces_reference_type() {
        let mut emitter = TestEmitter(vec![]);
        let mut checker = new_checker(&mut emitter);
        let ty = checker.check_expr(&make_expr(ExprKind::Borrow {
            expr: Box::new(int_lit()),
        }));
        assert_eq!(ty, Ty::Reference(Box::new(Ty::I32)));
        assert!(!checker.has_errors());
    }

    // --- Assign ---

    #[test]
    fn assign_same_type_ok() {
        let mut emitter = TestEmitter(vec![]);
        let mut checker = new_checker(&mut emitter);
        checker.env.define("x", Ty::I32);
        let ty = checker.check_expr(&make_expr(ExprKind::Assign {
            lhs: Box::new(ident("x")),
            rhs: Box::new(int_lit()),
        }));
        assert_eq!(ty, Ty::Unit);
        assert!(!checker.has_errors());
    }

    #[test]
    fn assign_type_mismatch_error() {
        let mut emitter = TestEmitter(vec![]);
        let mut checker = new_checker(&mut emitter);
        checker.env.define("x", Ty::I32);
        checker.check_expr(&make_expr(ExprKind::Assign {
            lhs: Box::new(ident("x")),
            rhs: Box::new(bool_lit()),
        }));
        assert!(checker.has_errors());
        assert!(emitter.0[0].message.contains("assignment type mismatch"));
    }

    // --- Tuple ---

    #[test]
    fn tuple_expr_ok() {
        let mut emitter = TestEmitter(vec![]);
        let mut checker = new_checker(&mut emitter);
        let ty = checker.check_expr(&make_expr(ExprKind::Tuple(vec![int_lit(), bool_lit()])));
        assert_eq!(ty, Ty::Tuple(vec![Ty::I32, Ty::Bool]));
        assert!(!checker.has_errors());
    }

    // --- While / Loop / Break ---

    #[test]
    fn while_loop_returns_unit() {
        let mut emitter = TestEmitter(vec![]);
        let mut checker = new_checker(&mut emitter);
        let ty = checker.check_expr(&make_expr(ExprKind::While {
            condition: Box::new(bool_lit()),
            body: Box::new(unit_block()),
            vow: None,
        }));
        assert_eq!(ty, Ty::Unit);
        assert!(!checker.has_errors());
    }

    #[test]
    fn loop_expr_returns_unit() {
        let mut emitter = TestEmitter(vec![]);
        let mut checker = new_checker(&mut emitter);
        let ty = checker.check_expr(&make_expr(ExprKind::Loop {
            vow: None,
            body: Box::new(unit_block()),
        }));
        assert_eq!(ty, Ty::Unit);
        assert!(!checker.has_errors());
    }

    #[test]
    fn break_is_never() {
        let mut emitter = TestEmitter(vec![]);
        let mut checker = new_checker(&mut emitter);
        let ty = checker.check_expr(&make_expr(ExprKind::Break { value: None }));
        assert_eq!(ty, Ty::Never);
    }

    #[test]
    fn continue_is_never() {
        let mut emitter = TestEmitter(vec![]);
        let mut checker = new_checker(&mut emitter);
        let ty = checker.check_expr(&make_expr(ExprKind::Continue));
        assert_eq!(ty, Ty::Never);
    }

    #[test]
    fn continue_outside_loop_is_error() {
        let mut emitter = TestEmitter(vec![]);
        let mut checker = new_checker(&mut emitter);
        checker.check_expr(&make_expr(ExprKind::Continue));
        assert!(checker.has_errors());
    }

    #[test]
    fn continue_inside_while_is_ok() {
        let mut emitter = TestEmitter(vec![]);
        let mut checker = new_checker(&mut emitter);
        checker.check_expr(&make_expr(ExprKind::While {
            condition: Box::new(bool_lit()),
            vow: None,
            body: Box::new(Block {
                stmts: vec![],
                trailing_expr: Some(Box::new(make_expr(ExprKind::Continue))),
                span: dummy_span(),
            }),
        }));
        assert!(!checker.has_errors());
    }

    #[test]
    fn loop_with_break_value_returns_break_type() {
        let mut emitter = TestEmitter(vec![]);
        let mut checker = new_checker(&mut emitter);
        // loop { break 42; }  →  should have type I32
        let ty = checker.check_expr(&make_expr(ExprKind::Loop {
            vow: None,
            body: Box::new(Block {
                stmts: vec![],
                trailing_expr: Some(Box::new(make_expr(ExprKind::Break {
                    value: Some(Box::new(int_lit())),
                }))),
                span: dummy_span(),
            }),
        }));
        assert_eq!(ty, Ty::I32);
        assert!(!checker.has_errors());
    }

    #[test]
    fn break_with_value_in_while_is_error() {
        let mut emitter = TestEmitter(vec![]);
        let mut checker = new_checker(&mut emitter);
        checker.check_expr(&make_expr(ExprKind::While {
            condition: Box::new(bool_lit()),
            vow: None,
            body: Box::new(Block {
                stmts: vec![],
                trailing_expr: Some(Box::new(make_expr(ExprKind::Break {
                    value: Some(Box::new(int_lit())),
                }))),
                span: dummy_span(),
            }),
        }));
        assert!(checker.has_errors());
    }

    #[test]
    fn break_type_mismatch_is_error() {
        let mut emitter = TestEmitter(vec![]);
        let mut checker = new_checker(&mut emitter);
        // loop { break 42; break true; } — mismatched break types
        checker.check_expr(&make_expr(ExprKind::Loop {
            vow: None,
            body: Box::new(Block {
                stmts: vec![Stmt::Expr {
                    expr: make_expr(ExprKind::Break {
                        value: Some(Box::new(int_lit())),
                    }),
                    has_semicolon: true,
                    span: dummy_span(),
                }],
                trailing_expr: Some(Box::new(make_expr(ExprKind::Break {
                    value: Some(Box::new(bool_lit())),
                }))),
                span: dummy_span(),
            }),
        }));
        assert!(checker.has_errors());
        assert!(
            emitter
                .0
                .iter()
                .any(|d| d.message.contains("break type mismatch"))
        );
    }

    // --- Return ---

    #[test]
    fn return_matching_type_ok() {
        let mut emitter = TestEmitter(vec![]);
        let mut checker = Checker::new("test.vow", &mut emitter);
        checker.current_return_ty = Ty::I32;
        checker.env.push_scope();
        let ty = checker.check_expr(&make_expr(ExprKind::Return {
            value: Some(Box::new(int_lit())),
        }));
        assert_eq!(ty, Ty::Never);
        assert!(!checker.has_errors());
    }

    #[test]
    fn return_wrong_type_error() {
        let mut emitter = TestEmitter(vec![]);
        let mut checker = Checker::new("test.vow", &mut emitter);
        checker.current_return_ty = Ty::I32;
        checker.env.push_scope();
        checker.check_expr(&make_expr(ExprKind::Return {
            value: Some(Box::new(bool_lit())),
        }));
        assert!(checker.has_errors());
        assert!(emitter.0[0].message.contains("return type"));
    }

    // --- Block expression ---

    #[test]
    fn block_expr_returns_trailing_type() {
        let mut emitter = TestEmitter(vec![]);
        let mut checker = new_checker(&mut emitter);
        let ty = checker.check_expr(&make_expr(ExprKind::Block(Box::new(make_block(int_lit())))));
        assert_eq!(ty, Ty::I32);
        assert!(!checker.has_errors());
    }

    // --- Question operator ---

    #[test]
    fn question_on_option_unwraps() {
        let mut emitter = TestEmitter(vec![]);
        let mut checker = new_checker(&mut emitter);
        let opt_i32 = Ty::Applied(Box::new(Ty::Enum("Option".to_string())), vec![Ty::I32]);
        checker.env.define("v", opt_i32);
        let ty = checker.check_expr(&make_expr(ExprKind::Question {
            expr: Box::new(ident("v")),
        }));
        assert_eq!(ty, Ty::I32);
        assert!(!checker.has_errors());
    }

    #[test]
    fn question_on_result_unwraps() {
        let mut emitter = TestEmitter(vec![]);
        let mut checker = new_checker(&mut emitter);
        let res_i32_str = Ty::Applied(
            Box::new(Ty::Enum("Result".to_string())),
            vec![Ty::I32, Ty::Str],
        );
        checker.env.define("v", res_i32_str);
        let ty = checker.check_expr(&make_expr(ExprKind::Question {
            expr: Box::new(ident("v")),
        }));
        assert_eq!(ty, Ty::I32);
        assert!(!checker.has_errors());
    }

    #[test]
    fn question_on_non_option_error() {
        let mut emitter = TestEmitter(vec![]);
        let mut checker = new_checker(&mut emitter);
        checker.check_expr(&make_expr(ExprKind::Question {
            expr: Box::new(int_lit()),
        }));
        assert!(checker.has_errors());
        assert!(emitter.0[0].message.contains("?"));
    }

    // --- Result expression ---

    #[test]
    fn result_expr_returns_current_return_type() {
        let mut emitter = TestEmitter(vec![]);
        let mut checker = Checker::new("test.vow", &mut emitter);
        checker.current_return_ty = Ty::Bool;
        checker.env.push_scope();
        let ty = checker.check_expr(&make_expr(ExprKind::Result));
        assert_eq!(ty, Ty::Bool);
    }

    // --- If without else ---

    #[test]
    fn if_without_else_returns_unit() {
        let mut emitter = TestEmitter(vec![]);
        let mut checker = new_checker(&mut emitter);
        let ty = checker.check_expr(&make_expr(ExprKind::If {
            condition: Box::new(bool_lit()),
            then_branch: Box::new(unit_block()),
            else_branch: None,
        }));
        assert_eq!(ty, Ty::Unit);
        assert!(!checker.has_errors());
    }

    #[test]
    fn if_else_same_type_ok() {
        let mut emitter = TestEmitter(vec![]);
        let mut checker = new_checker(&mut emitter);
        let ty = checker.check_expr(&make_expr(ExprKind::If {
            condition: Box::new(bool_lit()),
            then_branch: Box::new(make_block(int_lit())),
            else_branch: Some(Box::new(int_lit())),
        }));
        assert_eq!(ty, Ty::I32);
        assert!(!checker.has_errors());
    }

    #[test]
    fn if_non_bool_condition_error() {
        let mut emitter = TestEmitter(vec![]);
        let mut checker = new_checker(&mut emitter);
        checker.check_expr(&make_expr(ExprKind::If {
            condition: Box::new(int_lit()),
            then_branch: Box::new(unit_block()),
            else_branch: None,
        }));
        assert!(checker.has_errors());
        assert!(emitter.0[0].message.contains("bool"));
    }

    // --- check_stmt with let binding ---

    #[test]
    fn check_stmt_let_infers_type() {
        use vow_syntax::ast::{Pat, PatKind, Stmt};
        let mut emitter = TestEmitter(vec![]);
        let mut checker = new_checker(&mut emitter);
        let stmt = Stmt::Let {
            pattern: Pat {
                kind: PatKind::Ident {
                    name: "x".to_string(),
                    is_mut: false,
                },
                span: dummy_span(),
            },
            ty: None,
            init: Box::new(int_lit()),
            span: dummy_span(),
        };
        checker.check_stmt(&stmt);
        assert_eq!(checker.env.lookup("x"), Some(&Ty::I32));
        assert!(!checker.has_errors());
    }

    #[test]
    fn check_stmt_let_annotation_mismatch_error() {
        use vow_syntax::ast::{Pat, PatKind, Stmt};
        let mut emitter = TestEmitter(vec![]);
        let mut checker = new_checker(&mut emitter);
        let stmt = Stmt::Let {
            pattern: Pat {
                kind: PatKind::Ident {
                    name: "x".to_string(),
                    is_mut: false,
                },
                span: dummy_span(),
            },
            ty: Some(Type::Named {
                name: "bool".to_string(),
                span: dummy_span(),
            }),
            init: Box::new(int_lit()),
            span: dummy_span(),
        };
        checker.check_stmt(&stmt);
        assert!(checker.has_errors());
        assert!(emitter.0[0].message.contains("let binding"));
    }

    // --- check_fn return type mismatch ---

    #[test]
    fn fn_return_type_mismatch_error() {
        let mut emitter = TestEmitter(vec![]);
        let mut checker = Checker::new("test.vow", &mut emitter);
        let fn_def = FnDef {
            vis: Visibility::Private,
            name: "f".to_string(),
            params: vec![],
            return_ty: Type::Named {
                name: "bool".to_string(),
                span: dummy_span(),
            },
            effects: vec![],
            vow: None,
            body: Block {
                stmts: vec![],
                trailing_expr: Some(Box::new(int_lit())),
                span: dummy_span(),
            },
            span: dummy_span(),
            is_declaration: false,
        };
        let module = Module {
            name: "test".to_string(),
            uses: vec![],
            items: vec![Item::Fn(fn_def)],
            span: dummy_span(),
        };
        checker.check_module(&module);
        assert!(checker.has_errors());
        assert!(emitter.0[0].message.contains("return type"));
    }

    // --- check_module with Struct and Enum ---

    #[test]
    fn check_module_registers_struct() {
        use vow_syntax::ast::{FieldDef, Item, StructDef};
        let mut emitter = TestEmitter(vec![]);
        let mut checker = Checker::new("test.vow", &mut emitter);
        let module = Module {
            name: "test".to_string(),
            uses: vec![],
            items: vec![Item::Struct(StructDef {
                vis: Visibility::Public,
                is_linear: false,
                name: "Point".to_string(),
                fields: vec![FieldDef {
                    name: "x".to_string(),
                    ty: Type::Named {
                        name: "i32".to_string(),
                        span: dummy_span(),
                    },
                    span: dummy_span(),
                }],
                span: dummy_span(),
            })],
            span: dummy_span(),
        };
        checker.check_module(&module);
        assert!(!checker.has_errors());
        assert!(checker.env.lookup_struct("Point").is_some());
    }

    #[test]
    fn check_module_registers_enum() {
        use vow_syntax::ast::{EnumDef, EnumVariant, Item, VariantKind};
        let mut emitter = TestEmitter(vec![]);
        let mut checker = Checker::new("test.vow", &mut emitter);
        let module = Module {
            name: "test".to_string(),
            uses: vec![],
            items: vec![Item::Enum(EnumDef {
                vis: Visibility::Public,
                name: "Color".to_string(),
                variants: vec![
                    EnumVariant {
                        name: "Red".to_string(),
                        kind: VariantKind::Unit,
                        span: dummy_span(),
                    },
                    EnumVariant {
                        name: "Green".to_string(),
                        kind: VariantKind::Unit,
                        span: dummy_span(),
                    },
                ],
                span: dummy_span(),
            })],
            span: dummy_span(),
        };
        checker.check_module(&module);
        assert!(!checker.has_errors());
        assert!(checker.env.lookup_enum("Color").is_some());
    }

    // --- Match expression ---

    #[test]
    fn match_expr_all_arms_same_type_ok() {
        use vow_syntax::ast::{MatchArm, Pat, PatKind};
        let mut emitter = TestEmitter(vec![]);
        let mut checker = new_checker(&mut emitter);
        let expr = make_expr(ExprKind::Match {
            scrutinee: Box::new(bool_lit()),
            arms: vec![
                MatchArm {
                    pattern: Pat {
                        kind: PatKind::Lit(Lit::Bool(true)),
                        span: dummy_span(),
                    },
                    body: int_lit(),
                    span: dummy_span(),
                },
                MatchArm {
                    pattern: Pat {
                        kind: PatKind::Lit(Lit::Bool(false)),
                        span: dummy_span(),
                    },
                    body: int_lit(),
                    span: dummy_span(),
                },
            ],
        });
        let ty = checker.check_expr(&expr);
        assert_eq!(ty, Ty::I32);
        assert!(!checker.has_errors());
    }

    #[test]
    fn match_expr_arm_type_mismatch_error() {
        use vow_syntax::ast::{MatchArm, Pat, PatKind};
        let mut emitter = TestEmitter(vec![]);
        let mut checker = new_checker(&mut emitter);
        let expr = make_expr(ExprKind::Match {
            scrutinee: Box::new(bool_lit()),
            arms: vec![
                MatchArm {
                    pattern: Pat {
                        kind: PatKind::Lit(Lit::Bool(true)),
                        span: dummy_span(),
                    },
                    body: int_lit(),
                    span: dummy_span(),
                },
                MatchArm {
                    pattern: Pat {
                        kind: PatKind::Lit(Lit::Bool(false)),
                        span: dummy_span(),
                    },
                    body: bool_lit(),
                    span: dummy_span(),
                },
            ],
        });
        checker.check_expr(&expr);
        assert!(checker.has_errors());
    }

    // --- Float literal ---

    #[test]
    fn float_literal_type() {
        let mut emitter = TestEmitter(vec![]);
        let mut checker = new_checker(&mut emitter);
        let ty = checker.check_expr(&float_lit());
        assert_eq!(ty, Ty::F64);
        assert!(!checker.has_errors());
    }

    // --- String literal ---

    #[test]
    fn string_literal_type() {
        let mut emitter = TestEmitter(vec![]);
        let mut checker = new_checker(&mut emitter);
        let ty = checker.check_expr(&make_expr(ExprKind::Lit(Lit::String("hello".to_string()))));
        assert_eq!(ty, Ty::Str);
        assert!(!checker.has_errors());
    }

    // --- StructLiteral ---

    #[test]
    fn struct_literal_ok() {
        use crate::env::StructInfo;
        let mut emitter = TestEmitter(vec![]);
        let mut checker = Checker::new("test.vow", &mut emitter);
        checker.env.define_struct(
            "Point",
            StructInfo {
                fields: vec![("x".to_string(), Ty::I32), ("y".to_string(), Ty::I32)],
                is_linear: false,
            },
        );
        checker.env.push_scope();
        let ty = checker.check_expr(&make_expr(ExprKind::StructLiteral {
            name: "Point".to_string(),
            fields: vec![("x".to_string(), int_lit()), ("y".to_string(), int_lit())],
        }));
        assert_eq!(ty, Ty::Struct("Point".to_string()));
        assert!(!checker.has_errors());
    }

    #[test]
    fn struct_literal_wrong_field_type_error() {
        use crate::env::StructInfo;
        let mut emitter = TestEmitter(vec![]);
        let mut checker = Checker::new("test.vow", &mut emitter);
        checker.env.define_struct(
            "Point",
            StructInfo {
                fields: vec![("x".to_string(), Ty::I32)],
                is_linear: false,
            },
        );
        checker.env.push_scope();
        checker.check_expr(&make_expr(ExprKind::StructLiteral {
            name: "Point".to_string(),
            fields: vec![("x".to_string(), bool_lit())],
        }));
        assert!(checker.has_errors());
    }

    // --- EnumConstruct builtins ---

    #[test]
    fn enum_construct_option_none_is_never() {
        let mut emitter = TestEmitter(vec![]);
        let mut checker = new_checker(&mut emitter);
        let ty = checker.check_expr(&make_expr(ExprKind::EnumConstruct {
            path: vec!["Option".to_string(), "None".to_string()],
            fields: vec![],
        }));
        assert_eq!(ty, Ty::Never);
        assert!(!checker.has_errors());
    }

    #[test]
    fn enum_construct_option_some_wraps_payload() {
        let mut emitter = TestEmitter(vec![]);
        let mut checker = new_checker(&mut emitter);
        let ty = checker.check_expr(&make_expr(ExprKind::EnumConstruct {
            path: vec!["Option".to_string(), "Some".to_string()],
            fields: vec![int_lit()],
        }));
        assert_eq!(
            ty,
            Ty::Applied(Box::new(Ty::Enum("Option".to_string())), vec![Ty::I32])
        );
        assert!(!checker.has_errors());
    }

    #[test]
    fn enum_construct_result_ok_wraps_payload() {
        let mut emitter = TestEmitter(vec![]);
        let mut checker = new_checker(&mut emitter);
        let ty = checker.check_expr(&make_expr(ExprKind::EnumConstruct {
            path: vec!["Result".to_string(), "Ok".to_string()],
            fields: vec![int_lit()],
        }));
        assert_eq!(
            ty,
            Ty::Applied(
                Box::new(Ty::Enum("Result".to_string())),
                vec![Ty::I32, Ty::Unit]
            )
        );
        assert!(!checker.has_errors());
    }

    #[test]
    fn enum_construct_result_err_wraps_payload() {
        let mut emitter = TestEmitter(vec![]);
        let mut checker = new_checker(&mut emitter);
        let ty = checker.check_expr(&make_expr(ExprKind::EnumConstruct {
            path: vec!["Result".to_string(), "Err".to_string()],
            fields: vec![bool_lit()],
        }));
        assert_eq!(
            ty,
            Ty::Applied(
                Box::new(Ty::Enum("Result".to_string())),
                vec![Ty::Never, Ty::Bool]
            )
        );
        assert!(!checker.has_errors());
    }

    // --- Let binding annotation coercion ---

    #[test]
    fn let_binding_i32_lit_coerces_to_i64_annotation() {
        use vow_syntax::ast::{Pat, PatKind, Stmt};
        let mut emitter = TestEmitter(vec![]);
        let mut checker = new_checker(&mut emitter);
        let stmt = Stmt::Let {
            pattern: Pat {
                kind: PatKind::Ident {
                    name: "x".to_string(),
                    is_mut: false,
                },
                span: dummy_span(),
            },
            ty: Some(Type::Named {
                name: "i64".to_string(),
                span: dummy_span(),
            }),
            init: Box::new(int_lit()),
            span: dummy_span(),
        };
        checker.check_stmt(&stmt);
        assert_eq!(checker.env.lookup("x"), Some(&Ty::I64));
        assert!(!checker.has_errors());
    }

    // --- bounded_edit_distance tests ---

    #[test]
    fn edit_distance_identical() {
        assert_eq!(bounded_edit_distance("hello", "hello", 3), Some(0));
    }

    #[test]
    fn edit_distance_single_char() {
        assert_eq!(bounded_edit_distance("cat", "bat", 3), Some(1));
    }

    #[test]
    fn edit_distance_completely_different() {
        assert_eq!(bounded_edit_distance("abc", "xyz", 3), Some(3));
    }

    #[test]
    fn edit_distance_empty_strings() {
        assert_eq!(bounded_edit_distance("", "", 3), Some(0));
        assert_eq!(bounded_edit_distance("abc", "", 3), Some(3));
        assert_eq!(bounded_edit_distance("", "abc", 3), Some(3));
    }

    #[test]
    fn edit_distance_short_circuits_when_too_far() {
        // Length-difference fast path: |1 - 8| = 7 > 2, returns None before
        // touching the DP table.
        assert_eq!(bounded_edit_distance("a", "zzzzzzzz", 2), None);
        // Row-min pruning path: same-length strings whose distance exceeds
        // the cap, so the inter-row check rejects before the final row.
        assert_eq!(bounded_edit_distance("abcd", "wxyz", 2), None);
    }

    // --- suggest_similar tests ---

    #[test]
    fn suggest_similar_finds_close_match() {
        let candidates = vec!["counter".to_string(), "display".to_string()];
        assert_eq!(
            suggest_similar("coutner", &candidates, 3),
            Some("counter".to_string())
        );
    }

    #[test]
    fn suggest_similar_returns_none_if_too_far() {
        let candidates = vec!["counter".to_string()];
        assert_eq!(suggest_similar("xyz", &candidates, 2), None);
    }

    #[test]
    fn suggest_similar_skips_exact_match() {
        let candidates = vec!["foo".to_string()];
        assert_eq!(suggest_similar("foo", &candidates, 3), None);
    }

    // --- all_var_names / all_fn_names tests ---

    #[test]
    fn all_var_names_returns_defined_vars() {
        let mut env = TypeEnv::new();
        env.define("x", Ty::I64);
        env.define("y", Ty::Bool);
        let names = env.all_var_names(64, 64);
        assert!(names.contains(&"x".to_string()));
        assert!(names.contains(&"y".to_string()));
    }

    #[test]
    fn all_fn_names_returns_defined_fns() {
        let env = TypeEnv::new();
        let names = env.all_fn_names(64, 64);
        assert!(names.contains(&"print_str".to_string()));
        assert!(names.contains(&"print_i64".to_string()));
    }

    #[test]
    fn all_var_names_returns_bounded_subset_when_capped() {
        // Single-scope sanity check: with more bindings than `max_names`, the
        // helper must return exactly `max_names` entries — the
        // lexicographically smallest of the qualifying keys — and the result
        // must be reproducible across runs.
        let mut env = TypeEnv::new();
        for i in 0..1000 {
            env.define(format!("var_{i:04}").as_str(), Ty::I64);
        }
        let names = env.all_var_names(8, 64);
        assert_eq!(names.len(), 8);
        assert_eq!(
            names,
            vec![
                "var_0000".to_string(),
                "var_0001".to_string(),
                "var_0002".to_string(),
                "var_0003".to_string(),
                "var_0004".to_string(),
                "var_0005".to_string(),
                "var_0006".to_string(),
                "var_0007".to_string(),
            ]
        );
    }

    #[test]
    fn all_var_names_prioritizes_inner_scopes_under_cap() {
        // Multi-scope: when the cap can be filled from the innermost scope, no
        // outer-scope names appear, even if they're lexicographically smaller.
        // Within each scope, the lex-smallest entries are picked.
        let mut env = TypeEnv::new();
        env.define("aardvark", Ty::I64); // outer
        env.define("alpaca", Ty::I64); // outer
        env.push_scope();
        env.define("zebra", Ty::I64); // inner
        env.define("yak", Ty::I64); // inner
        let names = env.all_var_names(2, 64);
        assert_eq!(names, vec!["yak".to_string(), "zebra".to_string()]);

        // When the cap exceeds the inner scope, the remaining slots are filled
        // from the outer scope in lex order.
        let names = env.all_var_names(3, 64);
        assert_eq!(
            names,
            vec![
                "yak".to_string(),
                "zebra".to_string(),
                "aardvark".to_string()
            ]
        );
    }

    // --- hint integration tests ---

    #[test]
    fn undefined_variable_includes_did_you_mean_hint() {
        let mut emitter = TestEmitter(vec![]);
        let mut checker = Checker::new("test.vow", &mut emitter);
        checker.env.define("counter", Ty::I64);
        let expr = make_expr(ExprKind::Ident("coutner".to_string()));
        checker.check_expr(&expr);
        assert_eq!(emitter.0.len(), 1);
        assert!(emitter.0[0].hints.iter().any(|h| h.contains("counter")));
    }

    #[test]
    fn undefined_function_includes_did_you_mean_hint() {
        let mut emitter = TestEmitter(vec![]);
        let mut checker = Checker::new("test.vow", &mut emitter);
        let callee = make_expr(ExprKind::Ident("prnt_str".to_string()));
        let expr = make_expr(ExprKind::Call {
            callee: Box::new(callee),
            args: vec![],
        });
        checker.check_expr(&expr);
        assert!(!emitter.0.is_empty());
        let has_hint = emitter
            .0
            .iter()
            .any(|d| d.hints.iter().any(|h| h.contains("print_str")));
        assert!(has_hint, "expected 'did you mean' hint for print_str");
    }
}
