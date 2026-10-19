use cranelift_codegen::isa::{self, TargetIsa};
use cranelift_codegen::settings::{self, Configurable};
use std::sync::Arc;

use crate::{BuildMode, CodegenError};

/// Which CPU features the generated code may use.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum IsaProfile {
    /// Everything the host CPU supports.
    Native,
    /// The host architecture without feature detection, so the result does not
    /// depend on the CPU the process runs on. For reproducible benchmarks.
    Baseline,
}

/// The ISA using everything the host CPU supports.
pub(crate) fn make_isa(mode: BuildMode) -> Result<Arc<dyn TargetIsa>, CodegenError> {
    build_isa(mode, IsaProfile::Native)
}

/// The host-architecture ISA without feature detection (see `IsaProfile`).
pub(crate) fn make_baseline_isa(mode: BuildMode) -> Result<Arc<dyn TargetIsa>, CodegenError> {
    build_isa(mode, IsaProfile::Baseline)
}

fn build_isa(mode: BuildMode, profile: IsaProfile) -> Result<Arc<dyn TargetIsa>, CodegenError> {
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
    let isa_builder = match profile {
        IsaProfile::Baseline => isa::lookup(target_lexicon::Triple::host())
            .map_err(|e| CodegenError::IsaBuild(e.to_string()))?,
        IsaProfile::Native => {
            cranelift_native::builder().map_err(|e| CodegenError::IsaBuild(e.to_string()))?
        }
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
            let native = make_isa(mode).expect("native isa");
            let baseline = make_baseline_isa(mode).expect("baseline isa");
            assert_eq!(baseline.triple(), native.triple());
            assert_eq!(baseline.flags().opt_level(), native.flags().opt_level());
        }
    }
}
