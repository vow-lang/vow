use crate::types::{BasicBlock, Inst, InstData, InstId};

struct Insertion {
    index: usize,
    order: usize,
    inst: Inst,
}

pub struct InsertionSet {
    insertions: Vec<Insertion>,
    next_order: usize,
}

impl InsertionSet {
    pub fn new() -> Self {
        InsertionSet {
            insertions: Vec::new(),
            next_order: 0,
        }
    }

    pub fn insert_before(&mut self, index: usize, inst: Inst) {
        let order = self.next_order;
        self.next_order += 1;
        self.insertions.push(Insertion { index, order, inst });
    }

    pub fn execute(&mut self, block: &mut BasicBlock) {
        if self.insertions.is_empty() {
            return;
        }

        self.insertions.sort_by_key(|i| (i.index, i.order));

        let old_len = block.insts.len();
        let new_len = old_len + self.insertions.len();

        let mut remap = vec![0u32; old_len];
        let mut new_insts: Vec<Inst> = Vec::with_capacity(new_len);

        let mut ins_iter = self.insertions.iter().peekable();
        let mut new_pos: usize = 0;

        for (old_pos, old_inst) in block.insts.iter().enumerate() {
            while ins_iter.peek().is_some_and(|ins| ins.index == old_pos) {
                new_insts.push(ins_iter.next().unwrap().inst.clone());
                new_pos += 1;
            }
            remap[old_pos] = new_pos as u32;
            new_insts.push(old_inst.clone());
            new_pos += 1;
        }
        for ins in ins_iter {
            new_insts.push(ins.inst.clone());
        }

        for (pos, inst) in new_insts.iter_mut().enumerate() {
            inst.id = InstId(pos as u32);
            for arg in inst.args.iter_mut() {
                if (arg.0 as usize) < old_len {
                    *arg = InstId(remap[arg.0 as usize]);
                }
            }
            if let InstData::PhiTarget(id) = &mut inst.data
                && (id.0 as usize) < old_len
            {
                *id = InstId(remap[id.0 as usize]);
            }
        }

        block.insts = new_insts;
        self.insertions.clear();
        self.next_order = 0;
    }
}

impl Default for InsertionSet {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::{BlockId, InstData, Opcode, RegionId, Ty};
    use vow_syntax::span::Span;

    fn dummy_span() -> Span {
        Span::new(0, 0)
    }

    fn make_inst(id: u32, opcode: Opcode, data: InstData) -> Inst {
        Inst {
            id: InstId(id),
            opcode,
            ty: Ty::I32,
            args: vec![],
            data,
            origin: dummy_span(),
            region: RegionId::Root,
        }
    }

    fn make_inst_with_args(id: u32, opcode: Opcode, args: Vec<InstId>) -> Inst {
        Inst {
            id: InstId(id),
            opcode,
            ty: Ty::I32,
            args,
            data: InstData::None,
            origin: dummy_span(),
            region: RegionId::Root,
        }
    }

    fn block_with_insts(insts: Vec<Inst>) -> BasicBlock {
        BasicBlock {
            id: BlockId(0),
            insts,
        }
    }

    #[test]
    fn insert_before_beginning() {
        let orig = make_inst(0, Opcode::ConstI32, InstData::ConstI32(42));
        let mut block = block_with_insts(vec![orig]);

        let new_inst = make_inst(99, Opcode::ConstI32, InstData::ConstI32(1));
        let mut set = InsertionSet::new();
        set.insert_before(0, new_inst);
        set.execute(&mut block);

        assert_eq!(block.insts.len(), 2);
        assert_eq!(block.insts[0].id, InstId(0));
        assert_eq!(block.insts[0].data, InstData::ConstI32(1));
        assert_eq!(block.insts[1].id, InstId(1));
        assert_eq!(block.insts[1].data, InstData::ConstI32(42));
    }

    #[test]
    fn insert_before_middle_reindexes_args() {
        let i0 = make_inst(0, Opcode::ConstI32, InstData::ConstI32(10));
        let i1 = make_inst(1, Opcode::ConstI32, InstData::ConstI32(20));
        let i2 = make_inst_with_args(2, Opcode::WrappingAdd, vec![InstId(0), InstId(1)]);
        let mut block = block_with_insts(vec![i0, i1, i2]);

        let new_inst = make_inst(99, Opcode::ConstI32, InstData::ConstI32(5));
        let mut set = InsertionSet::new();
        set.insert_before(1, new_inst);
        set.execute(&mut block);

        assert_eq!(block.insts.len(), 4);
        assert_eq!(block.insts[0].id, InstId(0));
        assert_eq!(block.insts[1].id, InstId(1));
        assert_eq!(block.insts[1].data, InstData::ConstI32(5));
        assert_eq!(block.insts[2].id, InstId(2));
        assert_eq!(block.insts[3].id, InstId(3));
        assert_eq!(block.insts[3].args, vec![InstId(0), InstId(2)]);
    }

    #[test]
    fn multiple_insertions() {
        let i0 = make_inst(0, Opcode::ConstI32, InstData::ConstI32(1));
        let i1 = make_inst(1, Opcode::ConstI32, InstData::ConstI32(2));
        let mut block = block_with_insts(vec![i0, i1]);

        let a = make_inst(99, Opcode::ConstI32, InstData::ConstI32(10));
        let b = make_inst(100, Opcode::ConstI32, InstData::ConstI32(20));
        let c = make_inst(101, Opcode::ConstI32, InstData::ConstI32(30));

        let mut set = InsertionSet::new();
        set.insert_before(0, a);
        set.insert_before(1, b);
        set.insert_before(0, c);
        set.execute(&mut block);

        assert_eq!(block.insts.len(), 5);
        let values: Vec<i32> = block
            .insts
            .iter()
            .map(|inst| match inst.data {
                InstData::ConstI32(v) => v,
                _ => panic!("expected ConstI32"),
            })
            .collect();
        assert_eq!(values, vec![10, 30, 1, 20, 2]);
        for (expected_id, inst) in block.insts.iter().enumerate() {
            assert_eq!(inst.id, InstId(expected_id as u32));
        }
    }
}
