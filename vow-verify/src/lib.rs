pub mod c_emitter;
pub mod esbmc;

pub use c_emitter::detect_constant_functions;
pub use esbmc::{
    Counterexample, VerificationResult, parse_esbmc_output, verify_function,
    verify_function_with_const_fns, verify_function_with_module,
    verify_function_with_module_and_const_fns,
};
