pub mod c_emitter;
pub mod esbmc;

pub use c_emitter::detect_constant_functions;
pub use esbmc::{
    Counterexample, DEFAULT_MAX_K_STEP, VerificationResult, emit_verify_c_source, find_esbmc,
    parse_esbmc_output, run_esbmc_k_induction, run_esbmc_with_max_k_step, verify_function,
    verify_function_with_const_fns, verify_function_with_module,
    verify_function_with_module_and_const_fns,
    verify_function_with_module_and_const_fns_with_max_k_step,
};
