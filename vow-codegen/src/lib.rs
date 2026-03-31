pub mod cranelift_backend;
pub mod linker;

use vow_ir::Module;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BuildMode {
    Debug,
    Release,
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

    #[test]
    fn codegen_error_display_all_variants() {
        let cases = [
            (CodegenError::IsaBuild("e".into()), "ISA build error: e"),
            (
                CodegenError::FunctionDeclare("e".into()),
                "function declare error: e",
            ),
            (
                CodegenError::FunctionDefine("e".into()),
                "function define error: e",
            ),
            (CodegenError::Emit("e".into()), "emit error: e"),
            (
                CodegenError::UnsupportedOpcode("e".into()),
                "unsupported opcode: e",
            ),
            (CodegenError::Link("e".into()), "linker error: e"),
            (CodegenError::Io("e".into()), "I/O error: e"),
        ];
        for (err, expected) in cases {
            assert_eq!(err.to_string(), expected);
        }
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
