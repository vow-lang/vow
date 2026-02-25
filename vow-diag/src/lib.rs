use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Severity {
    Error,
    Warning,
    Note,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Blame {
    Caller,
    Callee,
    None,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SourceLocation {
    pub file: String,
    pub byte_offset: u32,
    pub byte_len: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Diagnostic {
    pub severity: Severity,
    pub code: ErrorCode,
    pub message: String,
    pub primary: SourceLocation,
    pub secondary: Vec<SourceLocation>,
    pub blame: Blame,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[non_exhaustive]
pub enum ErrorCode {
    // Lexer errors
    UnterminatedString,
    InvalidCharacter,
    // Parser errors
    UnexpectedToken,
    MissingDelimiter,
    // Type errors
    TypeMismatch,
    EffectViolation,
    LinearTypeViolation,
    NonExhaustiveMatch,
    // Vow errors
    VowRequiresViolated,
    VowEnsuresViolated,
    VowInvariantViolated,
}

pub trait DiagnosticEmitter {
    fn emit(&mut self, diagnostic: &Diagnostic);
    fn finish(&mut self);
}

pub struct JsonEmitter {
    diagnostics: Vec<Diagnostic>,
    output: Box<dyn std::io::Write>,
}

impl JsonEmitter {
    pub fn new(output: Box<dyn std::io::Write>) -> Self {
        Self {
            diagnostics: Vec::new(),
            output,
        }
    }
}

impl DiagnosticEmitter for JsonEmitter {
    fn emit(&mut self, diagnostic: &Diagnostic) {
        self.diagnostics.push(diagnostic.clone());
    }

    fn finish(&mut self) {
        let json = serde_json::to_string_pretty(&self.diagnostics)
            .expect("diagnostics must be serializable");
        writeln!(self.output, "{}", json).ok();
    }
}

pub struct HumanEmitter {
    output: Box<dyn std::io::Write>,
}

impl HumanEmitter {
    pub fn new(output: Box<dyn std::io::Write>) -> Self {
        Self { output }
    }
}

impl DiagnosticEmitter for HumanEmitter {
    fn emit(&mut self, diagnostic: &Diagnostic) {
        let severity = match diagnostic.severity {
            Severity::Error => "error",
            Severity::Warning => "warning",
            Severity::Note => "note",
        };
        writeln!(
            self.output,
            "{}[{:?}]: {} ({}:{})",
            severity,
            diagnostic.code,
            diagnostic.message,
            diagnostic.primary.file,
            diagnostic.primary.byte_offset,
        )
        .ok();
        if diagnostic.blame != Blame::None {
            writeln!(self.output, "  blame: {:?}", diagnostic.blame).ok();
        }
    }

    fn finish(&mut self) {}
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{Arc, Mutex};

    struct SharedBuf(Arc<Mutex<Vec<u8>>>);

    impl std::io::Write for SharedBuf {
        fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
            self.0.lock().unwrap().write(buf)
        }
        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }

    fn make_diagnostic() -> Diagnostic {
        Diagnostic {
            severity: Severity::Error,
            code: ErrorCode::VowRequiresViolated,
            message: "precondition violated: y != 0".to_string(),
            primary: SourceLocation {
                file: "test.vow".to_string(),
                byte_offset: 42,
                byte_len: 6,
            },
            secondary: vec![],
            blame: Blame::Caller,
        }
    }

    #[test]
    fn json_emitter_produces_valid_json() {
        let buf = Arc::new(Mutex::new(Vec::<u8>::new()));
        let mut emitter = JsonEmitter::new(Box::new(SharedBuf(Arc::clone(&buf))));
        emitter.emit(&make_diagnostic());
        emitter.finish();
        let output = String::from_utf8(buf.lock().unwrap().clone()).unwrap();
        let parsed: serde_json::Value = serde_json::from_str(&output).unwrap();
        assert!(parsed.is_array());
        let arr = parsed.as_array().unwrap();
        assert_eq!(arr.len(), 1);
        assert_eq!(arr[0]["blame"], "Caller");
        assert_eq!(arr[0]["severity"], "Error");
    }

    #[test]
    fn human_emitter_produces_output() {
        let buf = Arc::new(Mutex::new(Vec::<u8>::new()));
        let mut emitter = HumanEmitter::new(Box::new(SharedBuf(Arc::clone(&buf))));
        emitter.emit(&make_diagnostic());
        emitter.finish();
        let output = String::from_utf8(buf.lock().unwrap().clone()).unwrap();
        assert!(output.contains("error"));
        assert!(output.contains("precondition violated"));
    }
}
