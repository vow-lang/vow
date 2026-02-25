use crate::ast::{
    EnumDef, EnumVariant, ExternBlock, ExternFn, FieldDef, FnDef, ImplBlock, Item, TraitDef,
    TraitMethod, StructDef, Type, TypeAlias, VariantKind, Visibility,
};
use crate::span::Span;
use crate::token::TokenKind;
use vow_diag::ErrorCode;

use super::Parser;

impl Parser {
    pub fn parse_struct(&mut self, vis: Visibility) -> Item {
        let start = self.current_span();
        self.advance();
        self.parse_struct_inner(vis, false, start)
    }

    pub fn parse_struct_inner(&mut self, vis: Visibility, is_linear: bool, start: Span) -> Item {
        let (name, _) = self.expect_ident().unwrap_or(("<error>".to_string(), start));
        let generics = self.parse_generics();
        self.expect(TokenKind::LBrace);
        let mut fields = Vec::new();
        while !self.at(&TokenKind::RBrace) && !self.at_end() {
            let field_start = self.current_span();
            let (field_name, _) = match self.expect_ident() {
                Some(v) => v,
                None => break,
            };
            self.expect(TokenKind::Colon);
            let ty = self.parse_type_required();
            let field_end = ty.span();
            fields.push(FieldDef {
                name: field_name,
                ty,
                span: field_start.merge(field_end),
            });
            if self.at(&TokenKind::Comma) {
                self.advance();
            } else {
                break;
            }
        }
        let end = self.current_span();
        self.expect(TokenKind::RBrace);
        Item::Struct(StructDef {
            vis,
            is_linear,
            name,
            generics,
            fields,
            span: start.merge(end),
        })
    }

    pub fn parse_enum(&mut self, vis: Visibility) -> Item {
        let start = self.current_span();
        self.advance();
        let (name, _) = self.expect_ident().unwrap_or(("<error>".to_string(), start));
        let generics = self.parse_generics();
        self.expect(TokenKind::LBrace);
        let mut variants = Vec::new();
        while !self.at(&TokenKind::RBrace) && !self.at_end() {
            let variant_start = self.current_span();
            let (variant_name, _) = match self.expect_ident() {
                Some(v) => v,
                None => break,
            };
            let kind = if self.at(&TokenKind::LParen) {
                self.advance();
                let mut types = Vec::new();
                while !self.at(&TokenKind::RParen) && !self.at_end() {
                    types.push(self.parse_type_required());
                    if self.at(&TokenKind::Comma) {
                        self.advance();
                    } else {
                        break;
                    }
                }
                self.expect(TokenKind::RParen);
                VariantKind::Tuple(types)
            } else if self.at(&TokenKind::LBrace) {
                self.advance();
                let mut fields = Vec::new();
                while !self.at(&TokenKind::RBrace) && !self.at_end() {
                    let field_start = self.current_span();
                    let (field_name, _) = match self.expect_ident() {
                        Some(v) => v,
                        None => break,
                    };
                    self.expect(TokenKind::Colon);
                    let ty = self.parse_type_required();
                    let field_end = ty.span();
                    fields.push(FieldDef {
                        name: field_name,
                        ty,
                        span: field_start.merge(field_end),
                    });
                    if self.at(&TokenKind::Comma) {
                        self.advance();
                    } else {
                        break;
                    }
                }
                self.expect(TokenKind::RBrace);
                VariantKind::Struct(fields)
            } else {
                VariantKind::Unit
            };
            let variant_end = self.current_span();
            variants.push(EnumVariant {
                name: variant_name,
                kind,
                span: variant_start.merge(variant_end),
            });
            if self.at(&TokenKind::Comma) {
                self.advance();
            } else {
                break;
            }
        }
        let end = self.current_span();
        self.expect(TokenKind::RBrace);
        Item::Enum(EnumDef {
            vis,
            name,
            generics,
            variants,
            span: start.merge(end),
        })
    }

    pub fn parse_trait(&mut self, vis: Visibility) -> Item {
        let start = self.current_span();
        self.advance();
        let (name, _) = self.expect_ident().unwrap_or(("<error>".to_string(), start));
        let generics = self.parse_generics();
        self.expect(TokenKind::LBrace);
        let mut methods = Vec::new();
        while !self.at(&TokenKind::RBrace) && !self.at_end() {
            let method_start = self.current_span();
            self.expect(TokenKind::KwFn);
            let (method_name, _) =
                self.expect_ident().unwrap_or(("<error>".to_string(), method_start));
            let params = self.parse_params();
            let return_ty = if self.at(&TokenKind::ThinArrow) {
                self.advance();
                self.parse_type_required()
            } else {
                Type::Unit { span: self.current_span() }
            };
            let effects = self.parse_effects();
            let method_end = self.current_span();
            self.expect(TokenKind::Semicolon);
            methods.push(TraitMethod {
                name: method_name,
                params,
                return_ty,
                effects,
                span: method_start.merge(method_end),
            });
        }
        let end = self.current_span();
        self.expect(TokenKind::RBrace);
        Item::Trait(TraitDef {
            vis,
            name,
            generics,
            methods,
            span: start.merge(end),
        })
    }

    pub fn parse_impl(&mut self) -> Item {
        let start = self.current_span();
        self.advance();
        let generics = self.parse_generics();

        let first_ty = self.parse_type_required();
        let (trait_name, self_ty) = if self.at(&TokenKind::KwFor) {
            self.advance();
            let self_ty = self.parse_type_required();
            let trait_name = match &first_ty {
                Type::Named { name, .. } => name.clone(),
                Type::Generic { name, .. } => name.clone(),
                _ => {
                    let span = first_ty.span();
                    self.push_error(
                        ErrorCode::UnexpectedToken,
                        "expected trait name in impl".to_string(),
                        span,
                    );
                    "<error>".to_string()
                }
            };
            (Some(trait_name), self_ty)
        } else {
            (None, first_ty)
        };

        self.expect(TokenKind::LBrace);
        let mut methods: Vec<FnDef> = Vec::new();
        while !self.at(&TokenKind::RBrace) && !self.at_end() {
            let method_start = self.current_span();
            let vis = if self.at(&TokenKind::KwPub) {
                self.advance();
                Visibility::Public
            } else {
                Visibility::Private
            };
            self.expect(TokenKind::KwFn);
            let (method_name, _) =
                self.expect_ident().unwrap_or(("<error>".to_string(), method_start));
            let method_generics = self.parse_generics();
            let params = self.parse_params();
            let return_ty = if self.at(&TokenKind::ThinArrow) {
                self.advance();
                self.parse_type_required()
            } else {
                Type::Unit { span: self.current_span() }
            };
            let effects = self.parse_effects();
            let vow = if self.at(&TokenKind::KwVow) { self.parse_vow_block() } else { None };
            let body = self.parse_block_required();
            let method_end = body.span;
            methods.push(FnDef {
                vis,
                name: method_name,
                generics: method_generics,
                params,
                return_ty,
                effects,
                vow,
                body,
                span: method_start.merge(method_end),
            });
        }
        let end = self.current_span();
        self.expect(TokenKind::RBrace);
        Item::Impl(ImplBlock {
            trait_name,
            generics,
            self_ty,
            methods,
            span: start.merge(end),
        })
    }

    pub fn parse_type_alias(&mut self, vis: Visibility) -> Item {
        let start = self.current_span();
        self.advance();
        let (name, _) = self.expect_ident().unwrap_or(("<error>".to_string(), start));
        let generics = self.parse_generics();
        self.expect(TokenKind::Eq);
        let ty = self.parse_type_required();
        let end = self.current_span();
        self.expect(TokenKind::Semicolon);
        Item::TypeAlias(TypeAlias {
            vis,
            name,
            generics,
            ty,
            span: start.merge(end),
        })
    }

    pub fn parse_extern(&mut self) -> Item {
        let start = self.current_span();
        self.advance();
        let abi_span = self.current_span();
        match self.expect_string_literal() {
            Some((abi, _)) if abi != "C" => {
                self.push_error(
                    ErrorCode::UnexpectedToken,
                    format!("only extern \"C\" is supported, got \"{}\"", abi),
                    abi_span,
                );
            }
            _ => {}
        }
        self.expect(TokenKind::LBrace);
        let vow = if self.at(&TokenKind::KwVow) { self.parse_vow_block() } else { None };
        let mut fns = Vec::new();
        while !self.at(&TokenKind::RBrace) && !self.at_end() {
            let fn_start = self.current_span();
            self.expect(TokenKind::KwFn);
            let (fn_name, _) = self.expect_ident().unwrap_or(("<error>".to_string(), fn_start));
            let params = self.parse_params();
            let return_ty = if self.at(&TokenKind::ThinArrow) {
                self.advance();
                self.parse_type_required()
            } else {
                Type::Unit { span: self.current_span() }
            };
            let effects = self.parse_effects();
            let fn_end = self.current_span();
            self.expect(TokenKind::Semicolon);
            fns.push(ExternFn {
                name: fn_name,
                params,
                return_ty,
                effects,
                span: fn_start.merge(fn_end),
            });
        }
        let end = self.current_span();
        self.expect(TokenKind::RBrace);
        Item::Extern(ExternBlock {
            vow,
            fns,
            span: start.merge(end),
        })
    }
}

#[cfg(test)]
mod tests {
    use crate::ast::{Item, VariantKind, Visibility};
    use crate::parser::parse_item_source;

    fn parse_item(src: &str) -> Item {
        let (item, diags) = parse_item_source(src);
        assert!(
            diags.is_empty(),
            "parse errors: {:?}",
            diags.iter().map(|e| &e.message).collect::<Vec<_>>()
        );
        item.expect("no item parsed")
    }

    #[test]
    fn parse_struct_two_fields() {
        let src = "struct Point { x: i32, y: i32 }";
        let item = parse_item(src);
        let s = match item {
            Item::Struct(s) => s,
            other => panic!("expected struct, got {:?}", other),
        };
        assert_eq!(s.name, "Point");
        assert!(!s.is_linear);
        assert_eq!(s.vis, Visibility::Private);
        assert_eq!(s.fields.len(), 2);
        assert_eq!(s.fields[0].name, "x");
        assert_eq!(s.fields[1].name, "y");
    }

    #[test]
    fn parse_linear_struct() {
        let src = "linear struct Handle { fd: i32 }";
        let item = parse_item(src);
        let s = match item {
            Item::Struct(s) => s,
            other => panic!("expected struct, got {:?}", other),
        };
        assert_eq!(s.name, "Handle");
        assert!(s.is_linear);
        assert_eq!(s.fields.len(), 1);
    }

    #[test]
    fn parse_generic_struct() {
        let src = "struct Foo<T> { value: T }";
        let item = parse_item(src);
        let s = match item {
            Item::Struct(s) => s,
            other => panic!("expected struct, got {:?}", other),
        };
        assert_eq!(s.name, "Foo");
        assert_eq!(s.generics.len(), 1);
        assert_eq!(s.generics[0].name, "T");
        assert_eq!(s.fields.len(), 1);
    }

    #[test]
    fn parse_enum_all_variant_kinds() {
        let src = "enum Shape { Unit, Tuple(i32, i32), Struct { x: i32 } }";
        let item = parse_item(src);
        let e = match item {
            Item::Enum(e) => e,
            other => panic!("expected enum, got {:?}", other),
        };
        assert_eq!(e.name, "Shape");
        assert_eq!(e.variants.len(), 3);
        assert_eq!(e.variants[0].name, "Unit");
        assert!(matches!(e.variants[0].kind, VariantKind::Unit));
        assert_eq!(e.variants[1].name, "Tuple");
        assert!(matches!(&e.variants[1].kind, VariantKind::Tuple(ts) if ts.len() == 2));
        assert_eq!(e.variants[2].name, "Struct");
        assert!(matches!(&e.variants[2].kind, VariantKind::Struct(fs) if fs.len() == 1));
    }

    #[test]
    fn parse_trait_two_methods() {
        let src = "trait Drawable { fn draw(self: Self) -> (); fn area(self: Self) -> i32; }";
        let item = parse_item(src);
        let t = match item {
            Item::Trait(t) => t,
            other => panic!("expected trait, got {:?}", other),
        };
        assert_eq!(t.name, "Drawable");
        assert_eq!(t.methods.len(), 2);
        assert_eq!(t.methods[0].name, "draw");
        assert_eq!(t.methods[1].name, "area");
    }

    #[test]
    fn parse_impl_no_trait() {
        let src = "impl Point { fn new(x: i32, y: i32) -> Point { } }";
        let item = parse_item(src);
        let i = match item {
            Item::Impl(i) => i,
            other => panic!("expected impl, got {:?}", other),
        };
        assert!(i.trait_name.is_none());
        assert_eq!(i.methods.len(), 1);
        assert_eq!(i.methods[0].name, "new");
    }

    #[test]
    fn parse_impl_with_trait() {
        let src = "impl Drawable for Circle { fn draw(self: Circle) -> () { } }";
        let item = parse_item(src);
        let i = match item {
            Item::Impl(i) => i,
            other => panic!("expected impl, got {:?}", other),
        };
        assert_eq!(i.trait_name.as_deref(), Some("Drawable"));
        assert_eq!(i.methods.len(), 1);
    }

    #[test]
    fn parse_type_alias_item() {
        let src = "type Meters = i32;";
        let item = parse_item(src);
        let t = match item {
            Item::TypeAlias(t) => t,
            other => panic!("expected type alias, got {:?}", other),
        };
        assert_eq!(t.name, "Meters");
        assert_eq!(t.vis, Visibility::Private);
    }

    #[test]
    fn parse_extern_block_one_fn() {
        let src = "extern \"C\" { fn printf(fmt: i32) -> i32; }";
        let item = parse_item(src);
        let e = match item {
            Item::Extern(e) => e,
            other => panic!("expected extern, got {:?}", other),
        };
        assert!(e.vow.is_none());
        assert_eq!(e.fns.len(), 1);
        assert_eq!(e.fns[0].name, "printf");
    }

    #[test]
    fn parse_pub_struct() {
        let src = "pub struct Foo { x: i32 }";
        let item = parse_item(src);
        let s = match item {
            Item::Struct(s) => s,
            other => panic!("expected struct, got {:?}", other),
        };
        assert_eq!(s.vis, Visibility::Public);
        assert_eq!(s.name, "Foo");
    }

    #[test]
    fn parse_pub_enum() {
        let src = "pub enum Color { Red, Green, Blue }";
        let item = parse_item(src);
        let e = match item {
            Item::Enum(e) => e,
            other => panic!("expected enum, got {:?}", other),
        };
        assert_eq!(e.vis, Visibility::Public);
        assert_eq!(e.name, "Color");
        assert_eq!(e.variants.len(), 3);
    }
}
