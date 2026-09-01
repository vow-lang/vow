pub mod cranelift_backend;
pub mod linker;
mod return_materialization;

use vow_diag::{Blame, Diagnostic, ErrorCode, Severity, SourceLocation};
use vow_ir::Module;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BuildMode {
    Debug,
    Release,
    Profile,
    Sanitize,
}

impl BuildMode {
    /// Returns true if runtime vow checks should be emitted (Debug or Sanitize).
    pub fn has_debug_checks(self) -> bool {
        matches!(self, BuildMode::Debug | BuildMode::Sanitize)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TraceMode {
    Off,
    Calls,
    Full,
}

pub struct CompiledObject {
    pub bytes: Vec<u8>,
}

impl CompiledObject {
    pub fn write_to_file(&self, path: &std::path::Path) -> std::io::Result<()> {
        std::fs::write(path, &self.bytes)
    }
}

#[derive(Debug)]
pub enum CodegenError {
    IsaBuild(String),
    FunctionDeclare(String),
    FunctionDefine(String),
    Emit(String),
    UnsupportedOpcode(String),
    Link(String),
    Io(String),
}

impl CodegenError {
    fn error_code(&self) -> ErrorCode {
        match self {
            Self::UnsupportedOpcode(_) => ErrorCode::CodegenUnsupported,
            Self::IsaBuild(_)
            | Self::FunctionDeclare(_)
            | Self::FunctionDefine(_)
            | Self::Emit(_) => ErrorCode::CodegenFailed,
            Self::Link(_) => ErrorCode::LinkFailed,
            Self::Io(_) => ErrorCode::IoError,
        }
    }

    /// The structured diagnostic for this backend failure. Backend errors carry
    /// no instruction origin, so the span is the whole file.
    pub fn to_diagnostic(&self, file: &str) -> Diagnostic {
        Diagnostic {
            severity: Severity::Error,
            code: self.error_code(),
            message: self.to_string(),
            primary: SourceLocation {
                file: file.to_string(),
                byte_offset: 0,
                byte_len: 0,
            },
            secondary: vec![],
            blame: Blame::None,
            hints: vec![],
        }
    }

    /// The compatibility `message` field of a `CompileFailed` build result.
    /// Agents branch on `diagnostics[].error_code`, not on this free text.
    pub fn failure_message(&self) -> String {
        format!("{self:?}")
    }
}

impl std::fmt::Display for CodegenError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            CodegenError::IsaBuild(s) => write!(f, "ISA build error: {s}"),
            CodegenError::FunctionDeclare(s) => write!(f, "function declare error: {s}"),
            CodegenError::FunctionDefine(s) => write!(f, "function define error: {s}"),
            CodegenError::Emit(s) => write!(f, "emit error: {s}"),
            CodegenError::UnsupportedOpcode(s) => write!(f, "unsupported opcode: {s}"),
            CodegenError::Link(s) => write!(f, "linker error: {s}"),
            CodegenError::Io(s) => write!(f, "I/O error: {s}"),
        }
    }
}

pub trait Backend {
    fn compile_module(
        &self,
        module: &Module,
        mode: BuildMode,
        trace: TraceMode,
    ) -> Result<CompiledObject, CodegenError>;
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Every `CodegenError` variant with the `Display` text and `ErrorCode` it
    /// must produce. Adding a variant means adding exactly one row here.
    fn all_variants() -> [(CodegenError, &'static str, ErrorCode); 7] {
        [
            (
                CodegenError::IsaBuild("e".into()),
                "ISA build error: e",
                ErrorCode::CodegenFailed,
            ),
            (
                CodegenError::FunctionDeclare("e".into()),
                "function declare error: e",
                ErrorCode::CodegenFailed,
            ),
            (
                CodegenError::FunctionDefine("e".into()),
                "function define error: e",
                ErrorCode::CodegenFailed,
            ),
            (
                CodegenError::Emit("e".into()),
                "emit error: e",
                ErrorCode::CodegenFailed,
            ),
            (
                CodegenError::UnsupportedOpcode("e".into()),
                "unsupported opcode: e",
                ErrorCode::CodegenUnsupported,
            ),
            (
                CodegenError::Link("e".into()),
                "linker error: e",
                ErrorCode::LinkFailed,
            ),
            (
                CodegenError::Io("e".into()),
                "I/O error: e",
                ErrorCode::IoError,
            ),
        ]
    }

    #[test]
    fn codegen_error_maps_every_variant_to_an_error_code() {
        for (error, _, expected) in all_variants() {
            assert_eq!(error.error_code(), expected);
        }
    }

    #[test]
    fn codegen_error_display_all_variants() {
        for (error, expected, _) in all_variants() {
            assert_eq!(error.to_string(), expected);
        }
    }

    #[test]
    fn to_diagnostic_is_a_whole_file_error_carrying_the_display_message() {
        for (error, display, code) in all_variants() {
            let diagnostic = error.to_diagnostic("wide.vow");
            assert_eq!(diagnostic.severity, Severity::Error);
            assert_eq!(diagnostic.code, code);
            assert_eq!(diagnostic.message, display);
            assert_eq!(
                diagnostic.primary,
                SourceLocation {
                    file: "wide.vow".to_string(),
                    byte_offset: 0,
                    byte_len: 0,
                }
            );
            assert!(diagnostic.secondary.is_empty());
            assert_eq!(diagnostic.blame, Blame::None);
            assert!(diagnostic.hints.is_empty());
        }
    }

    #[test]
    fn failure_message_preserves_the_debug_rendering() {
        let error = CodegenError::UnsupportedOpcode("wide aggregate".to_string());
        assert_eq!(error.failure_message(), format!("{error:?}"));
    }

    #[test]
    fn compiled_object_write_to_file_roundtrip() {
        let dir = tempfile::TempDir::new().unwrap();
        let path = dir.path().join("out.bin");
        let obj = CompiledObject {
            bytes: vec![1, 2, 3, 255],
        };
        obj.write_to_file(&path).unwrap();
        assert_eq!(std::fs::read(&path).unwrap(), vec![1, 2, 3, 255]);
    }
}
