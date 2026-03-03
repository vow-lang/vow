pub mod c_emitter;
pub mod esbmc;

pub use esbmc::{Counterexample, VerificationResult, parse_esbmc_output, verify_function};
