use std::collections::BTreeSet;

use vow_diag::{Blame, Diagnostic, DiagnosticEmitter, ErrorCode, Severity, SourceLocation};
use vow_syntax::ast::{
    BinOp, Block, Effect, Expr, ExprKind, FnDef, Item, Lit, Module, Pat, PatKind, Stmt, UnOp,
};
use vow_syntax::span::Span;

use crate::env::{EnumInfo, FnSig, StructInfo, TypeEnv, VariantInfo, VariantKind};
use crate::types::Ty;

pub struct Checker<'e> {
    pub(crate) env: TypeEnv,
    pub(crate) current_return_ty: Ty,
    pub(crate) current_fn_effects: BTreeSet<Effect>,
    pub(crate) error_count: usize,
    pub(crate) file: String,
    pub(crate) emitter: &'e mut dyn DiagnosticEmitter,
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
        }
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
        });
    }

    pub fn check_module(&mut self, module: &Module) {
        // Registration pass
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
                Item::Extern(block) => {
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
                Item::Trait(_) | Item::Impl(_) => {}
            }
        }

        // Check pass
        for item in &module.items {
            self.check_item(item);
        }
    }

    fn check_item(&mut self, item: &Item) {
        match item {
            Item::Fn(fn_def) => self.check_fn(fn_def),
            Item::Struct(s) => self.check_struct(s),
            Item::Enum(e) => self.check_enum(e),
            _ => {}
        }
    }

    fn check_struct(&mut self, s: &vow_syntax::ast::StructDef) {
        for f in &s.fields {
            if let Err(msg) = self.env.resolve(&f.ty) {
                self.emit_error(ErrorCode::TypeMismatch, msg, f.span);
            }
        }
    }

    fn check_enum(&mut self, e: &vow_syntax::ast::EnumDef) {
        for v in &e.variants {
            match &v.kind {
                vow_syntax::ast::VariantKind::Unit => {}
                vow_syntax::ast::VariantKind::Tuple(types) => {
                    for t in types {
                        if let Err(msg) = self.env.resolve(t) {
                            self.emit_error(ErrorCode::TypeMismatch, msg, t.span());
                        }
                    }
                }
                vow_syntax::ast::VariantKind::Struct(fields) => {
                    for f in fields {
                        if let Err(msg) = self.env.resolve(&f.ty) {
                            self.emit_error(ErrorCode::TypeMismatch, msg, f.span);
                        }
                    }
                }
            }
        }
    }

    fn check_fn(&mut self, fn_def: &FnDef) {
        let outer_effects = std::mem::replace(
            &mut self.current_fn_effects,
            fn_def.effects.iter().cloned().collect(),
        );
        let outer_return_ty = self.current_return_ty.clone();

        self.current_return_ty = match self.env.resolve(&fn_def.return_ty) {
            Ok(ty) => ty,
            Err(msg) => {
                self.emit_error(ErrorCode::TypeMismatch, msg, fn_def.return_ty.span());
                Ty::Unit
            }
        };

        self.env.push_scope();
        for param in &fn_def.params {
            let ty = match self.env.resolve(&param.ty) {
                Ok(ty) => ty,
                Err(msg) => {
                    self.emit_error(ErrorCode::TypeMismatch, msg, param.span);
                    Ty::Unit
                }
            };
            self.env.define(&param.name, ty);
        }

        let body_ty = self.check_block(&fn_def.body);

        let expected = self.current_return_ty.clone();
        let coercible = body_ty == expected
            || body_ty == Ty::Never
            || (body_ty == Ty::I32 && expected.is_integer());
        if !coercible {
            self.emit_error(
                ErrorCode::TypeMismatch,
                format!(
                    "function body has type `{body_ty}` but declared return type is `{expected}`"
                ),
                fn_def.body.span,
            );
        }

        self.env.pop_scope();

        // TODO: After Wave 2 merges, add:
        // crate::effects::check_fn_effects(fn_def, &self.env, &self.file, self.emitter);
        // crate::linear::check_linear_usage(fn_def, &self.env, &self.file, self.emitter);

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
                                self.emit_error(
                                    ErrorCode::TypeMismatch,
                                    format!(
                                        "let binding annotated as `{ann_ty}` but initializer has type `{init_ty}`"
                                    ),
                                    ann.span(),
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
        match &expr.kind {
            ExprKind::Lit(lit) => match lit {
                Lit::Int(_) => Ty::I32,
                Lit::Float(_) => Ty::F64,
                Lit::Bool(_) => Ty::Bool,
                Lit::String(_) => Ty::Str,
            },
            ExprKind::Ident(name) => match self.env.lookup(name) {
                Some(ty) => ty.clone(),
                None => {
                    self.emit_error(
                        ErrorCode::TypeMismatch,
                        format!("undefined variable `{name}`"),
                        expr.span,
                    );
                    Ty::Unit
                }
            },
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
                    | BinOp::RemChecked => {
                        let elem_ty = self.check_same_numeric(lhs_ty, rhs_ty, expr.span);
                        Ty::Applied(Box::new(Ty::Enum("Option".to_string())), vec![elem_ty])
                    }
                    BinOp::Eq | BinOp::Ne | BinOp::Lt | BinOp::Le | BinOp::Gt | BinOp::Ge => {
                        let coercible = (lhs_ty == Ty::I32 && rhs_ty.is_integer())
                            || (rhs_ty == Ty::I32 && lhs_ty.is_integer());
                        if lhs_ty != rhs_ty
                            && lhs_ty != Ty::Never
                            && rhs_ty != Ty::Never
                            && !coercible
                        {
                            self.emit_error(
                                ErrorCode::TypeMismatch,
                                format!(
                                    "comparison operands have different types: `{lhs_ty}` and `{rhs_ty}`"
                                ),
                                expr.span,
                            );
                        }
                        Ty::Bool
                    }
                    BinOp::And | BinOp::Or => {
                        if lhs_ty != Ty::Bool && lhs_ty != Ty::Never {
                            self.emit_error(
                                ErrorCode::TypeMismatch,
                                format!("logical operator requires `bool`, found `{lhs_ty}`"),
                                lhs.span,
                            );
                        }
                        if rhs_ty != Ty::Bool && rhs_ty != Ty::Never {
                            self.emit_error(
                                ErrorCode::TypeMismatch,
                                format!("logical operator requires `bool`, found `{rhs_ty}`"),
                                rhs.span,
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
                        if !operand_ty.is_numeric() && operand_ty != Ty::Never {
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
                let (param_tys, return_ty) = match self.env.lookup_fn(name) {
                    Some(sig) => (sig.params.clone(), sig.return_ty.clone()),
                    None => {
                        self.emit_error(
                            ErrorCode::TypeMismatch,
                            format!("undefined function `{name}`"),
                            callee.span,
                        );
                        for arg in args {
                            self.check_expr(arg);
                        }
                        return Ty::Unit;
                    }
                };
                if args.len() != param_tys.len() {
                    self.emit_error(
                        ErrorCode::TypeMismatch,
                        format!(
                            "function `{name}` expects {} arguments but got {}",
                            param_tys.len(),
                            args.len()
                        ),
                        expr.span,
                    );
                }
                for (arg, expected_ty) in args.iter().zip(param_tys.iter()) {
                    let arg_ty = self.check_expr(arg);
                    let coercible =
                        arg_ty == Ty::I32 && expected_ty.is_integer() && *expected_ty != Ty::I32;
                    if arg_ty != *expected_ty && arg_ty != Ty::Never && !coercible {
                        self.emit_error(
                            ErrorCode::TypeMismatch,
                            format!(
                                "argument has type `{arg_ty}` but function expects `{expected_ty}`"
                            ),
                            arg.span,
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
                if is_str {
                    match method.as_str() {
                        "len" => Ty::I64,
                        "push_str" => Ty::Unit,
                        "eq" => Ty::Bool,
                        "contains" => Ty::Bool,
                        "byte_at" => Ty::I64,
                        "push_byte" => Ty::Unit,
                        _ => Ty::Unit,
                    }
                } else if is_hashmap {
                    match method.as_str() {
                        "len" => Ty::I64,
                        "insert" => Ty::Unit,
                        "get" => Ty::I64,
                        "contains_key" => Ty::Bool,
                        "remove" => Ty::Unit,
                        _ => Ty::Unit,
                    }
                } else if is_vec {
                    match method.as_str() {
                        "len" => Ty::I64,
                        "push" => Ty::Unit,
                        "get" => Ty::Applied(
                            Box::new(Ty::Enum("Option".to_string())),
                            vec![if let Ty::Applied(_, args) = &recv_ty {
                                args.first().cloned().unwrap_or(Ty::I64)
                            } else {
                                Ty::I64
                            }],
                        ),
                        _ => Ty::Unit,
                    }
                } else {
                    Ty::Unit
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
                            self.emit_error(
                                ErrorCode::TypeMismatch,
                                format!("struct `{struct_name}` has no field `{field}`"),
                                expr.span,
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
                        self.emit_error(
                            ErrorCode::TypeMismatch,
                            format!("index operation on non-indexable type `{base_ty}`"),
                            expr.span,
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
                        result_ty = arm_ty;
                    } else if arm_ty != result_ty && arm_ty != Ty::Never {
                        self.emit_error(
                            ErrorCode::TypeMismatch,
                            format!(
                                "match arm has type `{arm_ty}` but previous arms have type `{result_ty}`"
                            ),
                            arm.span,
                        );
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
                    self.emit_error(
                        ErrorCode::TypeMismatch,
                        format!("if condition must be `bool`, found `{cond_ty}`"),
                        condition.span,
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
                            self.emit_error(
                                ErrorCode::TypeMismatch,
                                format!(
                                    "if branches have different types: `{then_ty}` vs `{else_ty}`"
                                ),
                                expr.span,
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
                condition, body, ..
            } => {
                self.check_expr(condition);
                self.check_block(body);
                Ty::Unit
            }
            ExprKind::Loop { body, .. } => {
                self.check_block(body);
                Ty::Unit
            }
            ExprKind::Break { .. } => Ty::Never,
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
                    self.emit_error(
                        ErrorCode::TypeMismatch,
                        format!(
                            "return type `{val_ty}` does not match declared return type `{expected}`"
                        ),
                        expr.span,
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
                        self.emit_error(
                            ErrorCode::TypeMismatch,
                            format!(
                                "the `?` operator requires `Option<T>` or `Result<T,E>`, found `{inner_ty}`"
                            ),
                            inner.span,
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
                        self.emit_error(
                            ErrorCode::TypeMismatch,
                            format!("unknown struct `{name}`"),
                            expr.span,
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
            self.emit_error(
                ErrorCode::TypeMismatch,
                format!("arithmetic operator requires a numeric type, found `{lhs}`"),
                op_span,
            );
            return Ty::Unit;
        }
        if lhs != rhs {
            self.emit_error(
                ErrorCode::TypeMismatch,
                format!("arithmetic operands have different types: `{lhs}` and `{rhs}`"),
                op_span,
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

    // --- Checked arithmetic ---

    #[test]
    fn checked_add_returns_option() {
        let mut emitter = TestEmitter(vec![]);
        let mut checker = new_checker(&mut emitter);
        let ty = checker.check_expr(&make_expr(ExprKind::BinaryOp {
            op: BinOp::AddChecked,
            lhs: Box::new(int_lit()),
            rhs: Box::new(int_lit()),
        }));
        assert_eq!(
            ty,
            Ty::Applied(Box::new(Ty::Enum("Option".to_string())), vec![Ty::I32])
        );
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
    fn method_call_returns_unit() {
        let mut emitter = TestEmitter(vec![]);
        let mut checker = new_checker(&mut emitter);
        let ty = checker.check_expr(&make_expr(ExprKind::MethodCall {
            receiver: Box::new(int_lit()),
            method: "to_string".to_string(),
            args: vec![],
        }));
        assert_eq!(ty, Ty::Unit);
        assert!(!checker.has_errors());
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
}
