use crate::ast::{
    BinOp, Block, Effect, EnumDef, EnumVariant, Expr, ExprKind, ExternBlock, ExternFn, FnDef,
    ImplBlock, Item, Lit, MatchArm, Module, Param, Pat, PatKind, Stmt, StructDef, TraitDef,
    TraitMethod, Type, TypeAlias, UnOp, VariantKind, Visibility, VowBlock, VowClause,
};

pub fn print_module(module: &Module) -> String {
    let mut out = String::new();
    out.push_str(&format!("module {}\n", module.name));
    for u in &module.uses {
        out.push_str(&format!("use {}\n", u.path.join(".")));
    }
    if !module.items.is_empty() {
        out.push('\n');
    }
    let mut items = module.items.iter().peekable();
    while let Some(item) = items.next() {
        out.push_str(&print_item(item, 0));
        if items.peek().is_some() {
            out.push('\n');
        }
    }
    out
}

fn indent(level: usize) -> String {
    "    ".repeat(level)
}

fn print_item(item: &Item, level: usize) -> String {
    match item {
        Item::Fn(f) => print_fn(f, level),
        Item::Struct(s) => print_struct(s, level),
        Item::Enum(e) => print_enum(e, level),
        Item::Trait(t) => print_trait(t, level),
        Item::Impl(i) => print_impl(i, level),
        Item::TypeAlias(a) => print_type_alias(a, level),
        Item::Extern(e) => print_extern(e, level),
    }
}

fn print_visibility(vis: &Visibility) -> &'static str {
    match vis {
        Visibility::Public => "pub ",
        Visibility::Private => "",
    }
}

fn print_effects(effects: &[Effect]) -> String {
    if effects.is_empty() {
        return String::new();
    }
    let mut names: Vec<&str> = effects
        .iter()
        .map(|e| match e {
            Effect::Read => "read",
            Effect::Write => "write",
            Effect::IO => "io",
            Effect::Panic => "panic",
            Effect::Unsafe => "unsafe",
        })
        .collect();
    names.sort_unstable();
    format!(" [{}]", names.join(", "))
}

fn print_params(params: &[Param]) -> String {
    params
        .iter()
        .map(|p| {
            let ty_str = print_type(&p.ty);
            match &p.refinement {
                None => format!("{}: {}", p.name, ty_str),
                Some(pred) => format!("{}: {} where {}", p.name, ty_str, print_expr(pred)),
            }
        })
        .collect::<Vec<_>>()
        .join(", ")
}

fn print_vow_block(vow: &VowBlock, level: usize) -> String {
    let ind = indent(level);
    let inner = indent(level + 1);
    let mut out = format!("{}vow {{\n", ind);
    for clause in &vow.clauses {
        match clause {
            VowClause::Requires { expr, .. } => {
                out.push_str(&format!("{}requires: {}\n", inner, print_expr(expr)));
            }
            VowClause::Ensures { expr, .. } => {
                out.push_str(&format!("{}ensures: {}\n", inner, print_expr(expr)));
            }
            VowClause::Invariant { expr, .. } => {
                out.push_str(&format!("{}invariant: {}\n", inner, print_expr(expr)));
            }
        }
    }
    out.push_str(&format!("{}}}\n", ind));
    out
}

fn print_fn(f: &FnDef, level: usize) -> String {
    let ind = indent(level);
    let vis = print_visibility(&f.vis);
    let params = print_params(&f.params);
    let ret = print_type(&f.return_ty);
    let effects = print_effects(&f.effects);

    let ret_part = match &f.return_ty {
        Type::Unit { .. } => String::new(),
        _ => format!(" -> {}", ret),
    };

    let mut out = format!(
        "{}{}fn {}({}){}{}",
        ind, vis, f.name, params, ret_part, effects
    );

    if let Some(vow) = &f.vow {
        out.push_str(" vow {\n");
        for clause in &vow.clauses {
            let inner = indent(level + 1);
            match clause {
                VowClause::Requires { expr, .. } => {
                    out.push_str(&format!("{}requires: {}\n", inner, print_expr(expr)));
                }
                VowClause::Ensures { expr, .. } => {
                    out.push_str(&format!("{}ensures: {}\n", inner, print_expr(expr)));
                }
                VowClause::Invariant { expr, .. } => {
                    out.push_str(&format!("{}invariant: {}\n", inner, print_expr(expr)));
                }
            }
        }
        out.push_str(&format!("{}}} {{\n", ind));
    } else {
        out.push_str(" {\n");
    }

    out.push_str(&print_block_body(&f.body, level + 1));
    out.push_str(&format!("{}}}\n", ind));
    out
}

fn print_block_body(block: &Block, level: usize) -> String {
    let mut out = String::new();
    for stmt in &block.stmts {
        out.push_str(&print_stmt(stmt, level));
    }
    if let Some(expr) = &block.trailing_expr {
        out.push_str(&format!("{}{}\n", indent(level), print_expr(expr)));
    }
    out
}

fn print_block(block: &Block, level: usize) -> String {
    let ind = indent(level);
    let mut out = "{\n".to_string();
    out.push_str(&print_block_body(block, level + 1));
    out.push_str(&format!("{}}}", ind));
    out
}

fn print_stmt(stmt: &Stmt, level: usize) -> String {
    let ind = indent(level);
    match stmt {
        Stmt::Let {
            pattern, ty, init, ..
        } => {
            let pat_str = print_pat(pattern);
            let ty_str = match ty {
                Some(t) => format!(": {}", print_type(t)),
                None => String::new(),
            };
            format!("{}let {}{} = {};\n", ind, pat_str, ty_str, print_expr(init))
        }
        Stmt::Expr {
            expr,
            has_semicolon,
            ..
        } => {
            if *has_semicolon {
                format!("{}{};\n", ind, print_expr(expr))
            } else {
                format!("{}{}\n", ind, print_expr(expr))
            }
        }
    }
}

fn print_struct(s: &StructDef, level: usize) -> String {
    let ind = indent(level);
    let vis = print_visibility(&s.vis);
    let linear = if s.is_linear { "linear " } else { "" };
    let mut out = format!("{}{}{}struct {} {{\n", ind, vis, linear, s.name);
    for field in &s.fields {
        out.push_str(&format!(
            "{}    {}: {},\n",
            ind,
            field.name,
            print_type(&field.ty)
        ));
    }
    out.push_str(&format!("{}}}\n", ind));
    out
}

fn print_enum(e: &EnumDef, level: usize) -> String {
    let ind = indent(level);
    let vis = print_visibility(&e.vis);
    let mut out = format!("{}{}enum {} {{\n", ind, vis, e.name);
    for variant in &e.variants {
        out.push_str(&print_enum_variant(variant, level + 1));
    }
    out.push_str(&format!("{}}}\n", ind));
    out
}

fn print_enum_variant(v: &EnumVariant, level: usize) -> String {
    let ind = indent(level);
    match &v.kind {
        VariantKind::Unit => format!("{}{},\n", ind, v.name),
        VariantKind::Tuple(types) => {
            let tys: Vec<String> = types.iter().map(print_type).collect();
            format!("{}{}({}),\n", ind, v.name, tys.join(", "))
        }
        VariantKind::Struct(fields) => {
            let field_strs: Vec<String> = fields
                .iter()
                .map(|f| format!("{}: {}", f.name, print_type(&f.ty)))
                .collect();
            format!("{}{} {{ {} }},\n", ind, v.name, field_strs.join(", "))
        }
    }
}

fn print_trait(t: &TraitDef, level: usize) -> String {
    let ind = indent(level);
    let vis = print_visibility(&t.vis);
    let mut out = format!("{}{}trait {} {{\n", ind, vis, t.name);
    for method in &t.methods {
        out.push_str(&print_trait_method(method, level + 1));
    }
    out.push_str(&format!("{}}}\n", ind));
    out
}

fn print_trait_method(m: &TraitMethod, level: usize) -> String {
    let ind = indent(level);
    let params = print_params(&m.params);
    let ret = print_type(&m.return_ty);
    let effects = print_effects(&m.effects);

    let ret_part = match &m.return_ty {
        Type::Unit { .. } => String::new(),
        _ => format!(" -> {}", ret),
    };

    format!(
        "{}fn {}({}){}{};  \n",
        ind, m.name, params, ret_part, effects
    )
    .replace(";  \n", ";\n")
}

fn print_impl(i: &ImplBlock, level: usize) -> String {
    let ind = indent(level);
    let self_ty = print_type(&i.self_ty);
    let header = match &i.trait_name {
        Some(tr) => format!("{}impl {} for {} {{\n", ind, tr, self_ty),
        None => format!("{}impl {} {{\n", ind, self_ty),
    };
    let mut out = header;
    for method in &i.methods {
        out.push_str(&print_fn(method, level + 1));
    }
    out.push_str(&format!("{}}}\n", ind));
    out
}

fn print_type_alias(a: &TypeAlias, level: usize) -> String {
    let ind = indent(level);
    let vis = print_visibility(&a.vis);
    format!("{}{}type {} = {};\n", ind, vis, a.name, print_type(&a.ty))
}

fn print_extern(e: &ExternBlock, level: usize) -> String {
    let ind = indent(level);
    let mut out = format!("{}extern \"C\" {{\n", ind);
    if let Some(vow) = &e.vow {
        out.push_str(&print_vow_block(vow, level + 1));
    }
    for f in &e.fns {
        out.push_str(&print_extern_fn(f, level + 1));
    }
    out.push_str(&format!("{}}}\n", ind));
    out
}

fn print_extern_fn(f: &ExternFn, level: usize) -> String {
    let ind = indent(level);
    let params = print_params(&f.params);
    let effects = print_effects(&f.effects);

    let ret_part = match &f.return_ty {
        Type::Unit { .. } => String::new(),
        _ => format!(" -> {}", print_type(&f.return_ty)),
    };

    format!(
        "{}fn {}({}){}{};  \n",
        ind, f.name, params, ret_part, effects
    )
    .replace(";  \n", ";\n")
}

pub fn print_type(ty: &Type) -> String {
    match ty {
        Type::Named { name, .. } => name.clone(),
        Type::Generic { name, args, .. } => {
            let arg_strs: Vec<String> = args.iter().map(print_type).collect();
            format!("{}<{}>", name, arg_strs.join(", "))
        }
        Type::Refinement {
            binding,
            base,
            predicate,
            ..
        } => {
            format!(
                "{{ {}: {} | {} }}",
                binding,
                print_type(base),
                print_expr(predicate)
            )
        }
        Type::Reference { inner, .. } => format!("&{}", print_type(inner)),
        Type::Slice { inner, .. } => format!("[{}]", print_type(inner)),
        Type::Tuple { elems, .. } => {
            if elems.is_empty() {
                "()".to_string()
            } else {
                let elem_strs: Vec<String> = elems.iter().map(print_type).collect();
                format!("({})", elem_strs.join(", "))
            }
        }
        Type::Unit { .. } => "()".to_string(),
        Type::Never { .. } => "!".to_string(),
    }
}

fn binop_precedence(op: BinOp) -> u8 {
    match op {
        BinOp::Or => 1,
        BinOp::And => 2,
        BinOp::Eq | BinOp::Ne | BinOp::Lt | BinOp::Le | BinOp::Gt | BinOp::Ge => 3,
        BinOp::Add | BinOp::Sub | BinOp::AddChecked | BinOp::SubChecked => 4,
        BinOp::Mul
        | BinOp::Div
        | BinOp::Rem
        | BinOp::MulChecked
        | BinOp::DivChecked
        | BinOp::RemChecked => 5,
    }
}

fn binop_str(op: BinOp) -> &'static str {
    match op {
        BinOp::Add => "+",
        BinOp::Sub => "-",
        BinOp::Mul => "*",
        BinOp::Div => "/",
        BinOp::Rem => "%",
        BinOp::AddChecked => "+!",
        BinOp::SubChecked => "-!",
        BinOp::MulChecked => "*!",
        BinOp::DivChecked => "/!",
        BinOp::RemChecked => "%!",
        BinOp::Eq => "==",
        BinOp::Ne => "!=",
        BinOp::Lt => "<",
        BinOp::Le => "<=",
        BinOp::Gt => ">",
        BinOp::Ge => ">=",
        BinOp::And => "&&",
        BinOp::Or => "||",
    }
}

fn expr_precedence(expr: &Expr) -> u8 {
    match &expr.kind {
        ExprKind::BinaryOp { op, .. } => binop_precedence(*op),
        ExprKind::UnaryOp { .. } => 6,
        _ => 255,
    }
}

fn print_expr_with_parens(expr: &Expr, parent_prec: u8, is_right: bool) -> String {
    let child_prec = expr_precedence(expr);
    let needs_parens = match &expr.kind {
        ExprKind::BinaryOp { .. } => {
            if is_right {
                child_prec <= parent_prec
            } else {
                child_prec < parent_prec
            }
        }
        _ => false,
    };
    if needs_parens {
        format!("({})", print_expr(expr))
    } else {
        print_expr(expr)
    }
}

pub fn print_expr(expr: &Expr) -> String {
    match &expr.kind {
        ExprKind::Lit(lit) => print_lit(lit),
        ExprKind::Ident(name) => name.clone(),
        ExprKind::BinaryOp { op, lhs, rhs } => {
            let prec = binop_precedence(*op);
            let lhs_str = print_expr_with_parens(lhs, prec, false);
            let rhs_str = print_expr_with_parens(rhs, prec, true);
            format!("{} {} {}", lhs_str, binop_str(*op), rhs_str)
        }
        ExprKind::UnaryOp { op, operand } => {
            let op_str = match op {
                UnOp::Neg => "-",
                UnOp::Not => "!",
            };
            let inner = match &operand.kind {
                ExprKind::BinaryOp { .. } => format!("({})", print_expr(operand)),
                _ => print_expr(operand),
            };
            format!("{}{}", op_str, inner)
        }
        ExprKind::Call { callee, args } => {
            let args_str: Vec<String> = args.iter().map(print_expr).collect();
            format!("{}({})", print_expr(callee), args_str.join(", "))
        }
        ExprKind::MethodCall {
            receiver,
            method,
            args,
        } => {
            let args_str: Vec<String> = args.iter().map(print_expr).collect();
            format!(
                "{}.{}({})",
                print_expr(receiver),
                method,
                args_str.join(", ")
            )
        }
        ExprKind::FieldAccess { base, field } => {
            format!("{}.{}", print_expr(base), field)
        }
        ExprKind::Index { base, index } => {
            format!("{}[{}]", print_expr(base), print_expr(index))
        }
        ExprKind::Match { scrutinee, arms } => {
            let mut out = format!("match {} {{\n", print_expr(scrutinee));
            for arm in arms {
                out.push_str(&print_match_arm(arm, 1));
            }
            out.push('}');
            out
        }
        ExprKind::If {
            condition,
            then_branch,
            else_branch,
        } => {
            let mut out = format!(
                "if {} {}",
                print_expr(condition),
                print_block(then_branch, 0)
            );
            if let Some(else_expr) = else_branch {
                match &else_expr.kind {
                    ExprKind::If { .. } => {
                        out.push_str(&format!(" else {}", print_expr(else_expr)));
                    }
                    ExprKind::Block(b) => {
                        out.push_str(&format!(" else {}", print_block(b, 0)));
                    }
                    _ => {
                        out.push_str(&format!(" else {}", print_expr(else_expr)));
                    }
                }
            }
            out
        }
        ExprKind::While {
            condition,
            vow,
            body,
        } => {
            let mut out = format!("while {}", print_expr(condition));
            if let Some(v) = vow {
                out.push_str(" vow {\n");
                for clause in &v.clauses {
                    match clause {
                        VowClause::Requires { expr, .. } => {
                            out.push_str(&format!("{}requires: {}\n", indent(1), print_expr(expr)));
                        }
                        VowClause::Ensures { expr, .. } => {
                            out.push_str(&format!("{}ensures: {}\n", indent(1), print_expr(expr)));
                        }
                        VowClause::Invariant { expr, .. } => {
                            out.push_str(&format!(
                                "{}invariant: {}\n",
                                indent(1),
                                print_expr(expr)
                            ));
                        }
                    }
                }
                out.push_str("} ");
            } else {
                out.push(' ');
            }
            out.push_str(&print_block(body, 0));
            out
        }
        ExprKind::Loop { vow, body } => {
            let mut out = "loop".to_string();
            if let Some(v) = vow {
                out.push_str(" vow {\n");
                for clause in &v.clauses {
                    match clause {
                        VowClause::Requires { expr, .. } => {
                            out.push_str(&format!("{}requires: {}\n", indent(1), print_expr(expr)));
                        }
                        VowClause::Ensures { expr, .. } => {
                            out.push_str(&format!("{}ensures: {}\n", indent(1), print_expr(expr)));
                        }
                        VowClause::Invariant { expr, .. } => {
                            out.push_str(&format!(
                                "{}invariant: {}\n",
                                indent(1),
                                print_expr(expr)
                            ));
                        }
                    }
                }
                out.push_str("} ");
            } else {
                out.push(' ');
            }
            out.push_str(&print_block(body, 0));
            out
        }
        ExprKind::Break { value } => match value {
            Some(v) => format!("break {}", print_expr(v)),
            None => "break".to_string(),
        },
        ExprKind::Return { value } => match value {
            Some(v) => format!("return {}", print_expr(v)),
            None => "return".to_string(),
        },
        ExprKind::Block(b) => print_block(b, 0),
        ExprKind::Borrow { expr } => format!("&{}", print_expr(expr)),
        ExprKind::Question { expr } => format!("{}?", print_expr(expr)),
        ExprKind::Assign { lhs, rhs } => {
            format!("{} = {}", print_expr(lhs), print_expr(rhs))
        }
        ExprKind::Tuple(elems) => {
            let elem_strs: Vec<String> = elems.iter().map(print_expr).collect();
            format!("({})", elem_strs.join(", "))
        }
        ExprKind::Result => "result".to_string(),
    }
}

fn print_lit(lit: &Lit) -> String {
    match lit {
        Lit::Int(n) => n.to_string(),
        Lit::Float(f) => {
            let s = format!("{}", f);
            if s.contains('.') {
                s
            } else {
                format!("{}.0", s)
            }
        }
        Lit::Bool(b) => b.to_string(),
        Lit::String(s) => format!("\"{}\"", s),
    }
}

fn print_match_arm(arm: &MatchArm, level: usize) -> String {
    let ind = indent(level);
    format!(
        "{}{} => {},\n",
        ind,
        print_pat(&arm.pattern),
        print_expr(&arm.body)
    )
}

fn print_pat(pat: &Pat) -> String {
    match &pat.kind {
        PatKind::Wildcard => "_".to_string(),
        PatKind::Ident { name, is_mut } => {
            if *is_mut {
                format!("mut {}", name)
            } else {
                name.clone()
            }
        }
        PatKind::Lit(lit) => print_lit(lit),
        PatKind::Tuple(pats) => {
            let pat_strs: Vec<String> = pats.iter().map(print_pat).collect();
            format!("({})", pat_strs.join(", "))
        }
        PatKind::Struct { name, fields } => {
            let field_strs: Vec<String> = fields
                .iter()
                .map(|(fname, fpat)| format!("{}: {}", fname, print_pat(fpat)))
                .collect();
            format!("{} {{ {} }}", name, field_strs.join(", "))
        }
        PatKind::EnumVariant { path, inner } => {
            let path_str = path.join("::");
            if inner.is_empty() {
                path_str
            } else {
                let inner_strs: Vec<String> = inner.iter().map(print_pat).collect();
                format!("{}({})", path_str, inner_strs.join(", "))
            }
        }
        PatKind::Or(pats) => {
            let pat_strs: Vec<String> = pats.iter().map(print_pat).collect();
            pat_strs.join(" | ")
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ast::{
        BinOp, Block, Effect, EnumDef, EnumVariant, Expr, ExprKind, ExternBlock, ExternFn, FnDef,
        ImplBlock, Item, Lit, Module, Param, StructDef, TraitDef, TraitMethod, Type, TypeAlias,
        UseDecl, VariantKind, Visibility, VowBlock, VowClause,
    };
    use crate::span::Span;

    fn s() -> Span {
        Span::new(0, 1)
    }

    fn lit_expr(l: Lit) -> Expr {
        Expr {
            kind: ExprKind::Lit(l),
            span: s(),
        }
    }

    fn ident_expr(name: &str) -> Expr {
        Expr {
            kind: ExprKind::Ident(name.to_string()),
            span: s(),
        }
    }

    fn binop_expr(op: BinOp, lhs: Expr, rhs: Expr) -> Expr {
        Expr {
            kind: ExprKind::BinaryOp {
                op,
                lhs: Box::new(lhs),
                rhs: Box::new(rhs),
            },
            span: s(),
        }
    }

    fn empty_block() -> Block {
        Block {
            stmts: vec![],
            trailing_expr: None,
            span: s(),
        }
    }

    fn named_ty(name: &str) -> Type {
        Type::Named {
            name: name.to_string(),
            span: s(),
        }
    }

    fn unit_ty() -> Type {
        Type::Unit { span: s() }
    }

    fn empty_module(name: &str) -> Module {
        Module {
            name: name.to_string(),
            uses: vec![],
            items: vec![],
            span: s(),
        }
    }

    #[test]
    fn test_empty_module() {
        let m = empty_module("Foo");
        assert_eq!(print_module(&m), "module Foo\n");
    }

    #[test]
    fn test_module_with_use() {
        let m = Module {
            name: "Bar".to_string(),
            uses: vec![UseDecl {
                path: vec!["a".to_string(), "b".to_string(), "c".to_string()],
                span: s(),
            }],
            items: vec![],
            span: s(),
        };
        assert_eq!(print_module(&m), "module Bar\nuse a.b.c\n");
    }

    #[test]
    fn test_pure_function() {
        let f = FnDef {
            vis: Visibility::Private,
            name: "add".to_string(),
            params: vec![
                Param {
                    name: "x".to_string(),
                    ty: named_ty("i64"),
                    refinement: None,
                    span: s(),
                },
                Param {
                    name: "y".to_string(),
                    ty: named_ty("i64"),
                    refinement: None,
                    span: s(),
                },
            ],
            return_ty: named_ty("i64"),
            effects: vec![],
            vow: None,
            body: Block {
                stmts: vec![],
                trailing_expr: Some(Box::new(binop_expr(
                    BinOp::Add,
                    ident_expr("x"),
                    ident_expr("y"),
                ))),
                span: s(),
            },
            span: s(),
        };
        let m = Module {
            name: "M".to_string(),
            uses: vec![],
            items: vec![Item::Fn(f)],
            span: s(),
        };
        let out = print_module(&m);
        assert_eq!(
            out,
            "module M\n\nfn add(x: i64, y: i64) -> i64 {\n    x + y\n}\n"
        );
    }

    #[test]
    fn test_function_effects_sorted() {
        let f = FnDef {
            vis: Visibility::Public,
            name: "work".to_string(),
            params: vec![],
            return_ty: unit_ty(),
            effects: vec![Effect::Write, Effect::Read],
            vow: None,
            body: empty_block(),
            span: s(),
        };
        let m = Module {
            name: "M".to_string(),
            uses: vec![],
            items: vec![Item::Fn(f)],
            span: s(),
        };
        let out = print_module(&m);
        assert!(
            out.contains("[read, write]"),
            "effects must be sorted: {}",
            out
        );
    }

    #[test]
    fn test_function_with_vow() {
        let requires_expr = binop_expr(BinOp::Gt, ident_expr("x"), lit_expr(Lit::Int(0)));
        let ensures_expr = binop_expr(BinOp::Gt, ident_expr("result"), ident_expr("x"));
        let vow = VowBlock {
            clauses: vec![
                VowClause::Requires {
                    expr: requires_expr,
                    span: s(),
                },
                VowClause::Ensures {
                    expr: ensures_expr,
                    span: s(),
                },
            ],
            span: s(),
        };
        let f = FnDef {
            vis: Visibility::Public,
            name: "positive".to_string(),
            params: vec![Param {
                name: "x".to_string(),
                ty: named_ty("i64"),
                refinement: None,
                span: s(),
            }],
            return_ty: named_ty("i64"),
            effects: vec![],
            vow: Some(vow),
            body: Block {
                stmts: vec![],
                trailing_expr: Some(Box::new(ident_expr("x"))),
                span: s(),
            },
            span: s(),
        };
        let out = print_fn(&f, 0);
        assert!(out.contains("vow {"), "must contain vow block: {}", out);
        assert!(
            out.contains("requires: x > 0"),
            "must contain requires: {}",
            out
        );
        assert!(
            out.contains("ensures: result > x"),
            "must contain ensures: {}",
            out
        );
    }

    #[test]
    fn test_struct() {
        let s_def = StructDef {
            vis: Visibility::Public,
            is_linear: false,
            name: "Point".to_string(),
            fields: vec![
                crate::ast::FieldDef {
                    name: "x".to_string(),
                    ty: named_ty("i64"),
                    span: s(),
                },
                crate::ast::FieldDef {
                    name: "y".to_string(),
                    ty: named_ty("i64"),
                    span: s(),
                },
            ],
            span: s(),
        };
        let out = print_struct(&s_def, 0);
        assert_eq!(out, "pub struct Point {\n    x: i64,\n    y: i64,\n}\n");
    }

    #[test]
    fn test_linear_struct() {
        let s_def = StructDef {
            vis: Visibility::Public,
            is_linear: true,
            name: "Handle".to_string(),
            fields: vec![],
            span: s(),
        };
        let out = print_struct(&s_def, 0);
        assert_eq!(out, "pub linear struct Handle {\n}\n");
    }

    #[test]
    fn test_enum_all_variant_kinds() {
        let e = EnumDef {
            vis: Visibility::Public,
            name: "Shape".to_string(),
            variants: vec![
                EnumVariant {
                    name: "Circle".to_string(),
                    kind: VariantKind::Struct(vec![crate::ast::FieldDef {
                        name: "radius".to_string(),
                        ty: named_ty("f64"),
                        span: s(),
                    }]),
                    span: s(),
                },
                EnumVariant {
                    name: "Rect".to_string(),
                    kind: VariantKind::Tuple(vec![named_ty("f64"), named_ty("f64")]),
                    span: s(),
                },
                EnumVariant {
                    name: "Point".to_string(),
                    kind: VariantKind::Unit,
                    span: s(),
                },
            ],
            span: s(),
        };
        let out = print_enum(&e, 0);
        assert!(
            out.contains("Circle { radius: f64 },"),
            "struct variant: {}",
            out
        );
        assert!(out.contains("Rect(f64, f64),"), "tuple variant: {}", out);
        assert!(out.contains("Point,"), "unit variant: {}", out);
    }

    #[test]
    fn test_type_alias() {
        let a = TypeAlias {
            vis: Visibility::Public,
            name: "MyInt".to_string(),
            ty: named_ty("i64"),
            span: s(),
        };
        let out = print_type_alias(&a, 0);
        assert_eq!(out, "pub type MyInt = i64;\n");
    }

    #[test]
    fn test_binop_precedence_mul_over_add() {
        // (a + b) * c — the lhs needs parens because + has lower prec than *
        let add = binop_expr(BinOp::Add, ident_expr("a"), ident_expr("b"));
        let mul = binop_expr(BinOp::Mul, add, ident_expr("c"));
        assert_eq!(print_expr(&mul), "(a + b) * c");
    }

    #[test]
    fn test_binop_no_parens_same_prec_left_assoc() {
        // a + b + c — left-associative, no parens needed
        let add1 = binop_expr(BinOp::Add, ident_expr("a"), ident_expr("b"));
        let add2 = binop_expr(BinOp::Add, add1, ident_expr("c"));
        assert_eq!(print_expr(&add2), "a + b + c");
    }

    #[test]
    fn test_binop_parens_on_rhs_same_prec() {
        // a - (b - c) — rhs at same prec needs parens to preserve right-assoc meaning
        let sub_inner = binop_expr(BinOp::Sub, ident_expr("b"), ident_expr("c"));
        let sub_outer = binop_expr(BinOp::Sub, ident_expr("a"), sub_inner);
        assert_eq!(print_expr(&sub_outer), "a - (b - c)");
    }

    #[test]
    fn test_binop_and_over_or() {
        // (a && b) || c — no parens needed since && binds tighter
        let and = binop_expr(BinOp::And, ident_expr("a"), ident_expr("b"));
        let or = binop_expr(BinOp::Or, and, ident_expr("c"));
        assert_eq!(print_expr(&or), "a && b || c");
    }

    #[test]
    fn test_binop_or_inside_and_needs_parens() {
        // a && (b || c) — or inside and needs parens
        let or = binop_expr(BinOp::Or, ident_expr("b"), ident_expr("c"));
        let and = binop_expr(BinOp::And, ident_expr("a"), or);
        assert_eq!(print_expr(&and), "a && (b || c)");
    }

    #[test]
    fn test_all_binop_strings() {
        let pairs: &[(BinOp, &str)] = &[
            (BinOp::Add, "+"),
            (BinOp::Sub, "-"),
            (BinOp::Mul, "*"),
            (BinOp::Div, "/"),
            (BinOp::Rem, "%"),
            (BinOp::AddChecked, "+!"),
            (BinOp::SubChecked, "-!"),
            (BinOp::MulChecked, "*!"),
            (BinOp::DivChecked, "/!"),
            (BinOp::RemChecked, "%!"),
            (BinOp::Eq, "=="),
            (BinOp::Ne, "!="),
            (BinOp::Lt, "<"),
            (BinOp::Le, "<="),
            (BinOp::Gt, ">"),
            (BinOp::Ge, ">="),
            (BinOp::And, "&&"),
            (BinOp::Or, "||"),
        ];
        for (op, expected_op_str) in pairs {
            let expr = binop_expr(*op, ident_expr("a"), ident_expr("b"));
            let out = print_expr(&expr);
            assert!(
                out.contains(expected_op_str),
                "op {:?} should produce '{}', got '{}'",
                op,
                expected_op_str,
                out
            );
        }
    }

    #[test]
    fn test_type_named() {
        assert_eq!(print_type(&named_ty("i64")), "i64");
    }

    #[test]
    fn test_type_generic() {
        let ty = Type::Generic {
            name: "Vec".to_string(),
            args: vec![named_ty("i64")],
            span: s(),
        };
        assert_eq!(print_type(&ty), "Vec<i64>");
    }

    #[test]
    fn test_type_reference() {
        let ty = Type::Reference {
            inner: Box::new(named_ty("i64")),
            span: s(),
        };
        assert_eq!(print_type(&ty), "&i64");
    }

    #[test]
    fn test_type_slice() {
        let ty = Type::Slice {
            inner: Box::new(named_ty("u8")),
            span: s(),
        };
        assert_eq!(print_type(&ty), "[u8]");
    }

    #[test]
    fn test_type_tuple_empty() {
        let ty = Type::Tuple {
            elems: vec![],
            span: s(),
        };
        assert_eq!(print_type(&ty), "()");
    }

    #[test]
    fn test_type_tuple_multi() {
        let ty = Type::Tuple {
            elems: vec![named_ty("i64"), named_ty("bool")],
            span: s(),
        };
        assert_eq!(print_type(&ty), "(i64, bool)");
    }

    #[test]
    fn test_type_unit() {
        assert_eq!(print_type(&unit_ty()), "()");
    }

    #[test]
    fn test_type_never() {
        let ty = Type::Never { span: s() };
        assert_eq!(print_type(&ty), "!");
    }

    #[test]
    fn test_type_refinement() {
        let ty = Type::Refinement {
            binding: "x".to_string(),
            base: Box::new(named_ty("i64")),
            predicate: Box::new(binop_expr(
                BinOp::Gt,
                ident_expr("x"),
                lit_expr(Lit::Int(0)),
            )),
            span: s(),
        };
        assert_eq!(print_type(&ty), "{ x: i64 | x > 0 }");
    }

    #[test]
    fn test_extern_block() {
        let vow = VowBlock {
            clauses: vec![VowClause::Requires {
                expr: lit_expr(Lit::Bool(true)),
                span: s(),
            }],
            span: s(),
        };
        let eb = ExternBlock {
            vow: Some(vow),
            fns: vec![ExternFn {
                name: "malloc".to_string(),
                params: vec![Param {
                    name: "size".to_string(),
                    ty: named_ty("usize"),
                    refinement: None,
                    span: s(),
                }],
                return_ty: Type::Reference {
                    inner: Box::new(named_ty("u8")),
                    span: s(),
                },
                effects: vec![Effect::Unsafe],
                span: s(),
            }],
            span: s(),
        };
        let out = print_extern(&eb, 0);
        assert!(out.starts_with("extern \"C\" {"), "header: {}", out);
        assert!(out.contains("vow {"), "vow block: {}", out);
        assert!(
            out.contains("fn malloc(size: usize) -> &u8 [unsafe];"),
            "fn sig: {}",
            out
        );
    }

    #[test]
    fn test_param_with_refinement() {
        let param = Param {
            name: "n".to_string(),
            ty: named_ty("i64"),
            refinement: Some(Box::new(binop_expr(
                BinOp::Ne,
                ident_expr("n"),
                lit_expr(Lit::Int(0)),
            ))),
            span: s(),
        };
        let out = print_params(&[param]);
        assert_eq!(out, "n: i64 where n != 0");
    }

    #[test]
    fn test_trait_definition() {
        let tr = TraitDef {
            vis: Visibility::Public,
            name: "Display".to_string(),
            methods: vec![TraitMethod {
                name: "fmt".to_string(),
                params: vec![Param {
                    name: "self".to_string(),
                    ty: Type::Reference {
                        inner: Box::new(named_ty("Self")),
                        span: s(),
                    },
                    refinement: None,
                    span: s(),
                }],
                return_ty: unit_ty(),
                effects: vec![Effect::IO],
                span: s(),
            }],
            span: s(),
        };
        let out = print_trait(&tr, 0);
        assert!(out.starts_with("pub trait Display {"), "header: {}", out);
        assert!(out.contains("fn fmt(self: &Self) [io];"), "method: {}", out);
    }

    #[test]
    fn test_impl_block() {
        let f = FnDef {
            vis: Visibility::Public,
            name: "new".to_string(),
            params: vec![],
            return_ty: named_ty("Self"),
            effects: vec![],
            vow: None,
            body: empty_block(),
            span: s(),
        };
        let i = ImplBlock {
            trait_name: Some("MyTrait".to_string()),
            self_ty: named_ty("MyStruct"),
            methods: vec![f],
            span: s(),
        };
        let out = print_impl(&i, 0);
        assert!(
            out.starts_with("impl MyTrait for MyStruct {"),
            "header: {}",
            out
        );
        assert!(out.contains("pub fn new() -> Self {"), "method: {}", out);
    }
}
