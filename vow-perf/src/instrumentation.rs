//! Operation-count instrumentation for a dedicated performance artifact.
//!
//! This module deliberately exposes no in-place mutation API. Instrumentation
//! allocates fresh function-unique IDs and applies [`InsertionSet`] to cloned
//! blocks, so existing IR references and canonical `.vmod` serialization stay
//! stable.

use std::fmt;
use vow_ir::{InsertionSet, Inst, InstData, InstId, Module, Opcode, RegionId, Ty};

const COUNTER_SYMBOL: &str = "__vow_perf_count";
const VEC_SORT_COUNTER_SYMBOL: &str = "__vow_perf_count_vec_sort";

/// A cloned IR module containing operation-counter calls.
///
/// The source [`Module`] passed to [`instrument_module`] is never mutated. A
/// caller must explicitly compile this module to create the performance-test
/// artifact, separately from the production compilation.
#[derive(Debug, Clone, PartialEq)]
pub struct InstrumentedModule {
    module: Module,
}

impl InstrumentedModule {
    /// Borrow the cloned, instrumented IR module for dedicated code generation.
    pub const fn as_module(&self) -> &Module {
        &self.module
    }

    /// Consume the wrapper and return the cloned, instrumented IR module.
    pub fn into_module(self) -> Module {
        self.module
    }
}

/// Instrumentation could not allocate function-unique instruction IDs.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum InstrumentationError {
    InstructionIdSpaceExhausted { function: String },
}

impl fmt::Display for InstrumentationError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InstructionIdSpaceExhausted { function } => write!(
                formatter,
                "operation-count instrumentation exhausted instruction IDs in function `{function}`"
            ),
        }
    }
}

impl std::error::Error for InstrumentationError {}

/// Clone `source` and insert an operation-counter call before each executable
/// IR instruction in the clone.
///
/// Contract and complexity descriptors are non-executable metadata and are not
/// counted. Calls to a catalogued size-dependent runtime helper use that
/// helper's cost adapter, which receives the original operands; every other
/// executable instruction uses the one-operation counter.
///
/// The catalogue currently holds only `__vow_vec_sort`. An uncatalogued
/// size-dependent helper still counts as one operation, so a performance
/// verdict built on these counts must fail closed as unverified when it reaches
/// one. Completing the catalogue is tracked by #486.
pub fn instrument_module(source: &Module) -> Result<InstrumentedModule, InstrumentationError> {
    let mut module = source.clone();

    for function in &mut module.functions {
        let insertion_count = function
            .blocks
            .iter()
            .flat_map(|block| &block.insts)
            .filter(|inst| is_counted(inst.opcode))
            .count() as u64;
        let first_fresh_id = function
            .blocks
            .iter()
            .flat_map(|block| &block.insts)
            .map(|inst| u64::from(inst.id.0) + 1)
            .max()
            .unwrap_or(0);
        let end_id = first_fresh_id.checked_add(insertion_count).ok_or_else(|| {
            InstrumentationError::InstructionIdSpaceExhausted {
                function: function.name.clone(),
            }
        })?;
        if end_id > u64::from(u32::MAX) + 1 {
            return Err(InstrumentationError::InstructionIdSpaceExhausted {
                function: function.name.clone(),
            });
        }

        let mut next_id = first_fresh_id;
        for block in &mut function.blocks {
            let mut insertions = InsertionSet::new();
            for (index, inst) in block.insts.iter().enumerate() {
                if !is_counted(inst.opcode) {
                    continue;
                }
                let (counter_symbol, counter_args) = counter_for(inst);
                insertions.insert_before(
                    index,
                    Inst {
                        id: InstId(next_id as u32),
                        opcode: Opcode::Call,
                        ty: Ty::Unit,
                        args: counter_args,
                        data: InstData::CallExtern(counter_symbol.to_string()),
                        origin: inst.origin,
                        region: RegionId::Root,
                    },
                );
                next_id += 1;
            }
            insertions.execute(block);
        }
    }

    Ok(InstrumentedModule { module })
}

fn counter_for(inst: &Inst) -> (&'static str, Vec<InstId>) {
    match &inst.data {
        // The adapter ABI is exactly `(vec ptr)`. Forwarding a different operand
        // list would emit a call Cranelift cannot verify, so a helper whose
        // shape drifts from the catalogue falls back to the plain counter and
        // keeps the artifact compilable.
        InstData::CallExtern(symbol) if symbol == "__vow_vec_sort" && inst.args.len() == 1 => {
            (VEC_SORT_COUNTER_SYMBOL, inst.args.clone())
        }
        // Charges one operation even for an uncatalogued size-dependent helper
        // such as `__vow_map_contains`; see `instrument_module` and #486.
        _ => (COUNTER_SYMBOL, vec![]),
    }
}

fn is_counted(opcode: Opcode) -> bool {
    !matches!(
        opcode,
        Opcode::ComplexityDescriptor
            | Opcode::VowRequires
            | Opcode::VowEnsures
            | Opcode::VowInvariant
    )
}
