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
                            generics: s.generics.iter().map(|g| g.name.clone()).collect(),
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
                    self.env.define_enum(
                        &e.name,
                        EnumInfo {
                            variants,
                            generics: e.generics.iter().map(|g| g.name.clone()).collect(),
                        },
                    );
                }
                Item::TypeAlias(a) => match self.env.resolve(&a.ty) {
                    Ok(ty) => self.env.define_alias(&a.name, ty),
                    Err(msg) => self.emit_error(ErrorCode::TypeMismatch, msg, a.ty.span()),
                },
                Item::Trait(_) | Item::Impl(_) | Item::Extern(_) => {}
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
        if body_ty != expected && body_ty != Ty::Never {
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
                if let Some(ann) = ty {
                    match self.env.resolve(ann) {
                        Ok(ann_ty) if ann_ty != init_ty && init_ty != Ty::Never => {
                            self.emit_error(
                                ErrorCode::TypeMismatch,
                                format!(
                                    "let binding annotated as `{ann_ty}` but initializer has type `{init_ty}`"
                                ),
                                ann.span(),
                            );
                        }
                        Err(msg) => self.emit_error(ErrorCode::TypeMismatch, msg, ann.span()),
                        _ => {}
                    }
                }
                self.bind_pattern(pattern, &init_ty);
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
                        if lhs_ty != rhs_ty && lhs_ty != Ty::Never && rhs_ty != Ty::Never {
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
                    if arg_ty != *expected_ty && arg_ty != Ty::Never {
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
            ExprKind::MethodCall { receiver, args, .. } => {
                self.check_expr(receiver);
                for arg in args {
                    self.check_expr(arg);
                }
                Ty::Unit
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
                        if then_ty != else_ty && then_ty != Ty::Never && else_ty != Ty::Never {
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
                if val_ty != expected && val_ty != Ty::Never {
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
                if lhs_ty != rhs_ty && lhs_ty != Ty::Never && rhs_ty != Ty::Never {
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
        }
    }

    fn check_same_numeric(&mut self, lhs: Ty, rhs: Ty, op_span: Span) -> Ty {
        if lhs == Ty::Never {
            return rhs;
        }
        if rhs == Ty::Never {
            return lhs;
        }
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
            generics: vec![],
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
}
