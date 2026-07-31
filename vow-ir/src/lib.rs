pub mod effects;
pub mod insertion_set;
pub mod lower;
pub mod printer;
pub mod region;
pub mod serialize;
pub mod types;
pub mod validator;

pub use effects::{AbstractHeap, Effects, HeapSet, inst_effects};
pub use insertion_set::InsertionSet;
pub use lower::{StringExprSet, lower_module};
pub use printer::{print_function, print_module};
pub use region::{infer_regions, insert_region_markers};
pub use serialize::{DecodeError, MODULE_MAGIC, MODULE_VERSION, decode_module, encode_module};
pub use types::{
    AbstractRegionId, BasicBlock, BlockId, EnumLayout, FieldLayout, FuncId, Function,
    HiddenRegionIdx, Inst, InstData, InstId, IntegerSignedness, IntegerType, IntegerWidth, Module,
    Opcode, RegionConstraint, RegionId, RegionSummary, RegionVar, StoreEffect, StructLayout, Ty,
    VariantLayout, VowEntry, VowId,
};
pub use validator::{ValidationError, ValidationResult, validate, validate_function};
