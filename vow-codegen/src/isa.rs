use cranelift_codegen::isa::{self, TargetIsa};
use cranelift_codegen::settings::{self, Configurable};
use std::sync::Arc;

use crate::{BuildMode, CodegenError};

/// The ISA for `mode`. With `baseline`, the host architecture without CPU
/// feature detection, so the result does not depend on the CPU the process
/// runs on; used only for reproducible benchmarks.
pub(crate) fn make_isa(
    mode: BuildMode,
    baseline: bool,
) -> Result<Arc<dyn TargetIsa>, CodegenError> {
    let mut flag_builder = settings::builder();
    flag_builder
        .set("use_colocated_libcalls", "false")
        .map_err(|e| CodegenError::IsaBuild(e.to_string()))?;
    flag_builder
        .set("is_pic", "true")
        .map_err(|e| CodegenError::IsaBuild(e.to_string()))?;
    // I128 function parameters and returns are gated behind this flag; without
    // it Cranelift panics as soon as an i128/u128 crosses an ABI boundary.
    flag_builder
        .set("enable_llvm_abi_extensions", "true")
        .map_err(|e| CodegenError::IsaBuild(e.to_string()))?;
    if mode == BuildMode::Release || mode == BuildMode::Profile {
        flag_builder
            .set("opt_level", "speed")
            .map_err(|e| CodegenError::IsaBuild(e.to_string()))?;
    }
    let flags = settings::Flags::new(flag_builder);
    let isa_builder = if baseline {
        isa::lookup(target_lexicon::Triple::host())
            .map_err(|e| CodegenError::IsaBuild(e.to_string()))?
    } else {
        cranelift_native::builder().map_err(|e| CodegenError::IsaBuild(e.to_string()))?
    };
    isa_builder
        .finish(flags)
        .map_err(|e| CodegenError::IsaBuild(e.to_string()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn baseline_isa_targets_the_host_architecture_in_every_mode() {
        for mode in [BuildMode::Debug, BuildMode::Release, BuildMode::Profile] {
            let native = make_isa(mode, false).expect("native isa");
            let baseline = make_isa(mode, true).expect("baseline isa");
            assert_eq!(baseline.triple(), native.triple());
            assert_eq!(baseline.flags().opt_level(), native.flags().opt_level());
        }
    }
}
