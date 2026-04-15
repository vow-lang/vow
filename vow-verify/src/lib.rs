pub mod c_emitter;
pub mod esbmc;
pub mod solver_strategy;

pub use c_emitter::detect_constant_functions;
pub use esbmc::{
    Counterexample, DEFAULT_UNWIND, VerificationResult, emit_verify_c_source, find_esbmc,
    parse_esbmc_output, run_esbmc, run_esbmc_with_unwind, verify_function,
    verify_function_with_const_fns, verify_function_with_module,
    verify_function_with_module_and_const_fns,
    verify_function_with_module_and_const_fns_configured,
    verify_function_with_module_and_const_fns_with_unwind,
};
pub use solver_strategy::{
    DEFAULT_AUTO_TIMEOUT_SECS, Encoding, Solver, SolverConfig, classify_function, run_with_fallback,
};
