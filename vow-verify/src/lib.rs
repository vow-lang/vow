pub mod c_emitter;
pub mod esbmc;

pub use esbmc::{VerificationResult, verify_function};
