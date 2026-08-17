use std::path::Path;

use crate::CodegenError;
pub use vow_linker::{find_runtime_lib, find_shim_lib};

/// Link one or more object files together with the vow runtime into an
/// executable. Uses the system C compiler as the linker driver.
/// If `shim_lib` is provided, it is also included in the link.
pub fn link(
    objects: &[&Path],
    runtime_lib: &Path,
    shim_lib: Option<&Path>,
    output: &Path,
) -> Result<(), CodegenError> {
    let inputs = objects
        .iter()
        .copied()
        .chain(std::iter::once(runtime_lib))
        .chain(shim_lib);
    vow_linker::link_executable(inputs, output)
        .map_err(|error| CodegenError::Link(error.to_string()))
}
