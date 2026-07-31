//! Operation-count instrumentation for a dedicated performance artifact.

use std::fmt;
use vow_ir::{InsertionSet, Inst, InstData, InstId, Module, Opcode, RegionId, Ty};

const COUNTER_SYMBOL: &str = "__vow_perf_count";

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
                insertions.insert_before(
                    index,
                    Inst {
                        id: InstId(next_id as u32),
                        opcode: Opcode::Call,
                        ty: Ty::Unit,
                        args: vec![],
                        data: InstData::CallExtern(COUNTER_SYMBOL.to_string()),
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

fn is_counted(opcode: Opcode) -> bool {
    !matches!(
        opcode,
        Opcode::ComplexityDescriptor
            | Opcode::VowRequires
            | Opcode::VowEnsures
            | Opcode::VowInvariant
    )
}
