use std::fmt;

/// Normalized type representation used by the type checker.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum Ty {
    // Frontend-only marker for unsuffixed integer literals. This is not a
    // runtime integer type and must be consumed before IR lowering.
    LitInt,
    // Signed integers
    I8,
    I16,
    I32,
    I64,
    I128,
    // Unsigned integers
    U8,
    U16,
    U32,
    U64,
    U128,
    // Floats
    F32,
    F64,
    // Other primitives
    Bool,
    Str,
    // User-defined nominal types (by name)
    Struct(String),
    Enum(String),
    // Generic application: e.g. Vec<i32> = Applied(Box::new(Struct("Vec")), vec![I32])
    Applied(Box<Ty>, Vec<Ty>),
    // Reference (&T — borrows do not consume linear values)
    Reference(Box<Ty>),
    // Product types
    Tuple(Vec<Ty>),
    // Unit and bottom
    Unit,
    Never,
}

impl Ty {
    pub fn is_numeric(&self) -> bool {
        self.is_integer() || self.is_float()
    }

    pub fn is_lit_int(&self) -> bool {
        matches!(self, Ty::LitInt)
    }

    pub fn is_integer(&self) -> bool {
        matches!(
            self,
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
        )
    }

    pub fn is_float(&self) -> bool {
        matches!(self, Ty::F32 | Ty::F64)
    }

    pub fn is_unsigned(&self) -> bool {
        matches!(self, Ty::U8 | Ty::U16 | Ty::U32 | Ty::U64 | Ty::U128)
    }

    pub fn from_primitive_name(name: &str) -> Option<Ty> {
        match name {
            "i8" => Some(Ty::I8),
            "i16" => Some(Ty::I16),
            "i32" => Some(Ty::I32),
            "i64" => Some(Ty::I64),
            "i128" => Some(Ty::I128),
            "u8" => Some(Ty::U8),
            "u16" => Some(Ty::U16),
            "u32" => Some(Ty::U32),
            "u64" => Some(Ty::U64),
            "u128" => Some(Ty::U128),
            "f32" => Some(Ty::F32),
            "f64" => Some(Ty::F64),
            "bool" => Some(Ty::Bool),
            "str" | "String" => Some(Ty::Str),
            _ => None,
        }
    }
}

impl fmt::Display for Ty {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Ty::LitInt => write!(f, "integer literal"),
            Ty::I8 => write!(f, "i8"),
            Ty::I16 => write!(f, "i16"),
            Ty::I32 => write!(f, "i32"),
            Ty::I64 => write!(f, "i64"),
            Ty::I128 => write!(f, "i128"),
            Ty::U8 => write!(f, "u8"),
            Ty::U16 => write!(f, "u16"),
            Ty::U32 => write!(f, "u32"),
            Ty::U64 => write!(f, "u64"),
            Ty::U128 => write!(f, "u128"),
            Ty::F32 => write!(f, "f32"),
            Ty::F64 => write!(f, "f64"),
            Ty::Bool => write!(f, "bool"),
            Ty::Str => write!(f, "str"),
            Ty::Struct(s) | Ty::Enum(s) => write!(f, "{s}"),
            Ty::Applied(base, args) => {
                write!(f, "{base}<")?;
                for (i, arg) in args.iter().enumerate() {
                    if i > 0 {
                        write!(f, ", ")?;
                    }
                    write!(f, "{arg}")?;
                }
                write!(f, ">")
            }
            Ty::Reference(inner) => write!(f, "&{inner}"),
            Ty::Tuple(elems) => {
                write!(f, "(")?;
                for (i, elem) in elems.iter().enumerate() {
                    if i > 0 {
                        write!(f, ", ")?;
                    }
                    write!(f, "{elem}")?;
                }
                write!(f, ")")
            }
            Ty::Unit => write!(f, "()"),
            Ty::Never => write!(f, "!"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn from_primitive_name_correct_variants() {
        assert_eq!(Ty::from_primitive_name("i8"), Some(Ty::I8));
        assert_eq!(Ty::from_primitive_name("i16"), Some(Ty::I16));
        assert_eq!(Ty::from_primitive_name("i32"), Some(Ty::I32));
        assert_eq!(Ty::from_primitive_name("i64"), Some(Ty::I64));
        assert_eq!(Ty::from_primitive_name("i128"), Some(Ty::I128));
        assert_eq!(Ty::from_primitive_name("u8"), Some(Ty::U8));
        assert_eq!(Ty::from_primitive_name("u16"), Some(Ty::U16));
        assert_eq!(Ty::from_primitive_name("u32"), Some(Ty::U32));
        assert_eq!(Ty::from_primitive_name("u64"), Some(Ty::U64));
        assert_eq!(Ty::from_primitive_name("u128"), Some(Ty::U128));
        assert_eq!(Ty::from_primitive_name("f32"), Some(Ty::F32));
        assert_eq!(Ty::from_primitive_name("f64"), Some(Ty::F64));
        assert_eq!(Ty::from_primitive_name("bool"), Some(Ty::Bool));
        assert_eq!(Ty::from_primitive_name("str"), Some(Ty::Str));
        assert_eq!(Ty::from_primitive_name("Foo"), None);
        assert_eq!(Ty::from_primitive_name(""), None);
        assert_eq!(Ty::from_primitive_name("i33"), None);
    }

    #[test]
    fn is_numeric_integer_float_unsigned() {
        assert!(Ty::I32.is_numeric());
        assert!(Ty::U64.is_numeric());
        assert!(Ty::F32.is_numeric());
        assert!(!Ty::Bool.is_numeric());
        assert!(!Ty::Str.is_numeric());
        assert!(!Ty::Unit.is_numeric());

        assert!(Ty::I8.is_integer());
        assert!(Ty::U128.is_integer());
        assert!(!Ty::LitInt.is_integer());
        assert!(!Ty::F64.is_integer());
        assert!(!Ty::Bool.is_integer());

        assert!(Ty::F32.is_float());
        assert!(Ty::F64.is_float());
        assert!(!Ty::I32.is_float());

        assert!(Ty::U8.is_unsigned());
        assert!(Ty::U32.is_unsigned());
        assert!(!Ty::I32.is_unsigned());
        assert!(!Ty::F64.is_unsigned());
    }

    #[test]
    fn display_primitives() {
        assert_eq!(Ty::I32.to_string(), "i32");
        assert_eq!(Ty::LitInt.to_string(), "integer literal");
        assert_eq!(Ty::U64.to_string(), "u64");
        assert_eq!(Ty::F32.to_string(), "f32");
        assert_eq!(Ty::Bool.to_string(), "bool");
        assert_eq!(Ty::Str.to_string(), "str");
        assert_eq!(Ty::Unit.to_string(), "()");
        assert_eq!(Ty::Never.to_string(), "!");
    }

    #[test]
    fn display_struct_enum() {
        assert_eq!(Ty::Struct("Foo".to_string()).to_string(), "Foo");
        assert_eq!(Ty::Enum("Bar".to_string()).to_string(), "Bar");
    }

    #[test]
    fn display_applied() {
        let ty = Ty::Applied(Box::new(Ty::Struct("Vec".to_string())), vec![Ty::I32]);
        assert_eq!(ty.to_string(), "Vec<i32>");

        let ty2 = Ty::Applied(
            Box::new(Ty::Struct("Map".to_string())),
            vec![Ty::Str, Ty::U64],
        );
        assert_eq!(ty2.to_string(), "Map<str, u64>");
    }

    #[test]
    fn display_reference() {
        let ty = Ty::Reference(Box::new(Ty::I32));
        assert_eq!(ty.to_string(), "&i32");
    }

    #[test]
    fn display_tuple() {
        let ty = Ty::Tuple(vec![]);
        assert_eq!(ty.to_string(), "()");

        let ty2 = Ty::Tuple(vec![Ty::I32, Ty::Bool]);
        assert_eq!(ty2.to_string(), "(i32, bool)");
    }
}
