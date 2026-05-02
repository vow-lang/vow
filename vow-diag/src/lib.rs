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

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Diagnostic {
    pub severity: Severity,
    pub code: ErrorCode,
    pub message: String,
    pub primary: SourceLocation,
    pub secondary: Vec<SourceLocation>,
    pub blame: Blame,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub hints: Vec<String>,
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
    // Method/feature errors
    UnknownMethod,
    UnsupportedFeature,
    // Lowering warnings
    LoweringWarning,
    // Contract errors
    MissingContract,
    ContractTypeMismatch,
    // Verification tool errors
    EsbmcNotFound,
    // IO errors
    IoError,
    // Region inference (arena-per-scope, Phase 3)
    RegionConflict,
    RegionLinear,
    // Emitted as a Warning when a vowed function's body cannot be modeled
    // by the verifier (e.g. uses RegionAlloc/FieldSet/Linear*/Load/Store).
    // The build still succeeds; the contract is documentary, not statically
    // checked. Runtime checks still apply in --mode debug.
    VerificationSkipped,
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

pub struct CollectingEmitter<'a> {
    inner: &'a mut dyn DiagnosticEmitter,
    collected: Vec<Diagnostic>,
}

impl<'a> CollectingEmitter<'a> {
    pub fn new(inner: &'a mut dyn DiagnosticEmitter) -> Self {
        Self {
            inner,
            collected: Vec::new(),
        }
    }

    pub fn into_diagnostics(self) -> Vec<Diagnostic> {
        self.collected
    }
}

impl DiagnosticEmitter for CollectingEmitter<'_> {
    fn emit(&mut self, diagnostic: &Diagnostic) {
        self.inner.emit(diagnostic);
        self.collected.push(diagnostic.clone());
    }

    fn finish(&mut self) {
        self.inner.finish();
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
        for hint in &diagnostic.hints {
            writeln!(self.output, "  hint: {hint}").ok();
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
            hints: vec![],
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

    #[test]
    fn json_emitter_skips_empty_hints() {
        let buf = Arc::new(Mutex::new(Vec::<u8>::new()));
        let mut emitter = JsonEmitter::new(Box::new(SharedBuf(Arc::clone(&buf))));
        emitter.emit(&make_diagnostic());
        emitter.finish();
        let output = String::from_utf8(buf.lock().unwrap().clone()).unwrap();
        assert!(
            !output.contains("hints"),
            "empty hints should be omitted from JSON"
        );
    }

    #[test]
    fn json_emitter_includes_nonempty_hints() {
        let buf = Arc::new(Mutex::new(Vec::<u8>::new()));
        let mut emitter = JsonEmitter::new(Box::new(SharedBuf(Arc::clone(&buf))));
        let mut diag = make_diagnostic();
        diag.hints = vec!["did you mean `counter`?".to_string()];
        emitter.emit(&diag);
        emitter.finish();
        let output = String::from_utf8(buf.lock().unwrap().clone()).unwrap();
        let parsed: serde_json::Value = serde_json::from_str(&output).unwrap();
        let arr = parsed.as_array().unwrap();
        let hints = arr[0]["hints"].as_array().unwrap();
        assert_eq!(hints.len(), 1);
        assert_eq!(hints[0], "did you mean `counter`?");
    }

    #[test]
    fn region_conflict_debug_format_is_pascalcase() {
        // Spec §13.1 mandates the external JSON `error_code` is "RegionConflict".
        // The DiagnosticJson layer in `vow/src/main.rs` derives this string via
        // `format!("{:?}", code)`, so the Debug format MUST stay PascalCase.
        assert_eq!(format!("{:?}", ErrorCode::RegionConflict), "RegionConflict");
    }

    #[test]
    fn region_linear_debug_format_is_pascalcase() {
        // Same contract as `region_conflict_debug_format_is_pascalcase`: the JSON
        // `error_code` for the post-region linear-obligation check is "RegionLinear".
        assert_eq!(format!("{:?}", ErrorCode::RegionLinear), "RegionLinear");
    }

    #[test]
    fn human_emitter_prints_hints() {
        let buf = Arc::new(Mutex::new(Vec::<u8>::new()));
        let mut emitter = HumanEmitter::new(Box::new(SharedBuf(Arc::clone(&buf))));
        let mut diag = make_diagnostic();
        diag.hints = vec!["check the value of y".to_string()];
        emitter.emit(&diag);
        let output = String::from_utf8(buf.lock().unwrap().clone()).unwrap();
        assert!(output.contains("hint: check the value of y"));
    }
}
