use crate::types::{AbstractRegionId, Opcode};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AbstractHeap {
    Memory,
    SsaState,
    VowState,
    Io,
    Region(AbstractRegionId),
}

#[derive(Debug, Clone, Default)]
pub struct HeapSet(Vec<AbstractHeap>);

impl HeapSet {
    pub fn empty() -> Self {
        HeapSet(Vec::new())
    }

    pub fn contains(&self, heap: &AbstractHeap) -> bool {
        self.0.contains(heap)
    }

    pub fn insert(&mut self, heap: AbstractHeap) {
        if !self.contains(&heap) {
            self.0.push(heap);
        }
    }

    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }
}

#[derive(Debug, Clone)]
pub struct Effects {
    pub reads: HeapSet,
    pub writes: HeapSet,
    pub traps: bool,
    pub control: bool,
}

impl Effects {
    pub fn pure() -> Self {
        Effects {
            reads: HeapSet::empty(),
            writes: HeapSet::empty(),
            traps: false,
            control: false,
        }
    }

    pub fn is_pure(&self) -> bool {
        self.reads.is_empty() && self.writes.is_empty() && !self.traps && !self.control
    }
}

pub fn inst_effects(opcode: &Opcode) -> Effects {
    match opcode {
        Opcode::Phi => {
            let mut e = Effects::pure();
            e.reads.insert(AbstractHeap::SsaState);
            e
        }
        Opcode::Upsilon => {
            let mut e = Effects::pure();
            e.writes.insert(AbstractHeap::SsaState);
            e
        }
        Opcode::Load => {
            let mut e = Effects::pure();
            e.reads.insert(AbstractHeap::Memory);
            e
        }
        Opcode::Store => {
            let mut e = Effects::pure();
            e.writes.insert(AbstractHeap::Memory);
            e
        }
        Opcode::VowRequires | Opcode::VowEnsures | Opcode::VowInvariant => {
            let mut e = Effects::pure();
            e.reads.insert(AbstractHeap::VowState);
            e.traps = true;
            e
        }
        Opcode::Call => {
            let mut e = Effects::pure();
            e.reads.insert(AbstractHeap::Memory);
            e.reads.insert(AbstractHeap::Io);
            e.writes.insert(AbstractHeap::Memory);
            e.writes.insert(AbstractHeap::Io);
            e
        }
        Opcode::DebugCall => {
            let mut e = Effects::pure();
            e.writes.insert(AbstractHeap::Io);
            e
        }
        Opcode::Branch | Opcode::Jump | Opcode::Return | Opcode::Unreachable => {
            let mut e = Effects::pure();
            e.control = true;
            e
        }
        Opcode::RegionAlloc => {
            let mut e = Effects::pure();
            e.reads.insert(AbstractHeap::Memory);
            e.writes.insert(AbstractHeap::Memory);
            e
        }
        Opcode::RegionOpen | Opcode::RegionClose => {
            let mut e = Effects::pure();
            e.reads.insert(AbstractHeap::Memory);
            e.writes.insert(AbstractHeap::Memory);
            e
        }
        Opcode::LinearConsume | Opcode::LinearBorrow => {
            let mut e = Effects::pure();
            e.reads.insert(AbstractHeap::Memory);
            e
        }
        Opcode::FieldGet => {
            let mut e = Effects::pure();
            e.reads.insert(AbstractHeap::Memory);
            e
        }
        Opcode::FieldSet => {
            let mut e = Effects::pure();
            e.reads.insert(AbstractHeap::Memory);
            e.writes.insert(AbstractHeap::Memory);
            e
        }
        _ => Effects::pure(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pure_ops_have_no_effects() {
        for opcode in [Opcode::ConstI32, Opcode::WrappingAddI32, Opcode::EqI32] {
            let e = inst_effects(&opcode);
            assert!(e.is_pure(), "{opcode:?} should be pure");
        }
    }

    #[test]
    fn load_reads_memory() {
        let e = inst_effects(&Opcode::Load);
        assert!(e.reads.contains(&AbstractHeap::Memory));
        assert!(e.writes.is_empty());
        assert!(!e.traps);
        assert!(!e.control);
    }

    #[test]
    fn store_writes_memory() {
        let e = inst_effects(&Opcode::Store);
        assert!(e.writes.contains(&AbstractHeap::Memory));
        assert!(e.reads.is_empty());
        assert!(!e.traps);
        assert!(!e.control);
    }

    #[test]
    fn vow_ops_trap() {
        for opcode in [
            Opcode::VowRequires,
            Opcode::VowEnsures,
            Opcode::VowInvariant,
        ] {
            let e = inst_effects(&opcode);
            assert!(e.traps, "{opcode:?} should trap");
            assert!(e.reads.contains(&AbstractHeap::VowState));
        }
    }

    #[test]
    fn control_ops() {
        for opcode in [
            Opcode::Branch,
            Opcode::Jump,
            Opcode::Return,
            Opcode::Unreachable,
        ] {
            let e = inst_effects(&opcode);
            assert!(e.control, "{opcode:?} should be control");
            assert!(!e.traps);
        }
    }

    #[test]
    fn upsilon_writes_ssa_state() {
        let e = inst_effects(&Opcode::Upsilon);
        assert!(e.writes.contains(&AbstractHeap::SsaState));
        assert!(e.reads.is_empty());
    }

    #[test]
    fn phi_reads_ssa_state() {
        let e = inst_effects(&Opcode::Phi);
        assert!(e.reads.contains(&AbstractHeap::SsaState));
        assert!(e.writes.is_empty());
    }

    #[test]
    fn call_reads_and_writes_memory_and_io() {
        let e = inst_effects(&Opcode::Call);
        assert!(e.reads.contains(&AbstractHeap::Memory));
        assert!(e.reads.contains(&AbstractHeap::Io));
        assert!(e.writes.contains(&AbstractHeap::Memory));
        assert!(e.writes.contains(&AbstractHeap::Io));
    }
}
