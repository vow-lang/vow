//! Operation-count instrumentation for a dedicated performance artifact.
//!
//! This module deliberately exposes no in-place mutation API. Instrumentation
//! allocates fresh function-unique IDs and applies [`InsertionSet`] to cloned
//! blocks, so existing IR references and canonical `.vmod` serialization stay
//! stable.

use std::fmt;
use vow_ir::{InsertionSet, Inst, InstData, InstId, Module, Opcode, RegionId, Ty};

const COUNTER_SYMBOL: &str = "__vow_perf_count";

/// A size-dependent runtime helper and the cost adapter that charges its work.
///
/// `operands` is the helper's IR operand count before codegen's arena routing,
/// which may prepend a hidden arena argument and so make the helper's ABI arity
/// larger. The adapter is handed the helper's own operands unchanged, so its
/// parameter list must mirror the helper's. Adding a
/// row here also requires the adapter's `extern "C"` definition in `vow-runtime`
/// and a matching arm in both `make_extern_sig` implementations (`vow-codegen`
/// and `vow-clif-shim`); a row on its own will not link. See #486.
struct CostAdapter {
    helper: &'static str,
    adapter: &'static str,
    operands: usize,
}

const COST_ADAPTERS: &[CostAdapter] = &[CostAdapter {
    helper: "__vow_vec_sort",
    adapter: "__vow_perf_count_vec_sort",
    operands: 1,
}];

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

/// Instrumentation could not produce a faithful operation count.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum InstrumentationError {
    InstructionIdSpaceExhausted {
        function: String,
    },
    /// A catalogued size-dependent helper carried an operand list its cost
    /// adapter cannot accept, so its hidden work is unmeasurable.
    CatalogedHelperArity {
        function: String,
        symbol: String,
        expected: usize,
        found: usize,
    },
}

impl fmt::Display for InstrumentationError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InstructionIdSpaceExhausted { function } => write!(
                formatter,
                "operation-count instrumentation exhausted instruction IDs in function `{function}`"
            ),
            Self::CatalogedHelperArity {
                function,
                symbol,
                expected,
                found,
            } => write!(
                formatter,
                "operation-count instrumentation found `{symbol}` with {found} operands in \
                 function `{function}`, but its cost adapter takes exactly {expected}"
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
///
/// A catalogued helper whose operand list no longer matches its cost adapter
/// is an error rather than a degraded count: charging the one-operation counter
/// would report a sort as constant work and let a wrong complexity class pass.
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
                let (counter_symbol, counter_args) = counter_for(inst, &function.name)?;
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

fn counter_for(
    inst: &Inst,
    function: &str,
) -> Result<(&'static str, Vec<InstId>), InstrumentationError> {
    // Charges one operation for anything uncatalogued, including a
    // size-dependent helper such as `__vow_map_contains`; see
    // `instrument_module` and #486.
    let InstData::CallExtern(symbol) = &inst.data else {
        return Ok((COUNTER_SYMBOL, vec![]));
    };
    let Some(entry) = COST_ADAPTERS
        .iter()
        .find(|entry| entry.helper == symbol.as_str())
    else {
        return Ok((COUNTER_SYMBOL, vec![]));
    };

    // A drifted operand list cannot be forwarded: Cranelift would reject the
    // call against the adapter's signature.
    if inst.args.len() != entry.operands {
        return Err(InstrumentationError::CatalogedHelperArity {
            function: function.to_string(),
            symbol: symbol.clone(),
            expected: entry.operands,
            found: inst.args.len(),
        });
    }
    Ok((entry.adapter, inst.args.clone()))
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
