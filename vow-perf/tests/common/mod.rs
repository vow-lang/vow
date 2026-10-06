//! Shared fixtures for vow-perf integration tests.

use vow_ir::{Inst, InstData, InstId, Opcode, RegionId, Ty};
use vow_syntax::span::Span;

pub fn instruction(id: u32, opcode: Opcode, ty: Ty, args: Vec<InstId>, data: InstData) -> Inst {
    Inst {
        id: InstId(id),
        opcode,
        ty,
        args,
        data,
        origin: Span::new(0, 0),
        region: RegionId::Root,
    }
}
