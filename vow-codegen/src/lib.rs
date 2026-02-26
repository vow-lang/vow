pub mod cranelift_backend;

use vow_ir::Module;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BuildMode {
    Debug,
    Release,
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
}

impl std::fmt::Display for CodegenError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            CodegenError::IsaBuild(s) => write!(f, "ISA build error: {s}"),
            CodegenError::FunctionDeclare(s) => write!(f, "function declare error: {s}"),
            CodegenError::FunctionDefine(s) => write!(f, "function define error: {s}"),
            CodegenError::Emit(s) => write!(f, "emit error: {s}"),
            CodegenError::UnsupportedOpcode(s) => write!(f, "unsupported opcode: {s}"),
        }
    }
}

pub trait Backend {
    fn compile_module(
        &self,
        module: &Module,
        mode: BuildMode,
    ) -> Result<CompiledObject, CodegenError>;
}
