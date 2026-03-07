use crate::types::{BasicBlock, BlockId, FuncId, Function, InstData, InstId, Module, Opcode, Ty};
use std::collections::HashSet;

#[derive(Debug)]
pub enum ValidationError {
    UndefinedInstRef { user: InstId, referenced: InstId },
    TypeMismatch { inst: InstId, expected: Ty, got: Ty },
    BlockNotTerminated(BlockId),
    MultipleTerminators(BlockId),
    PhiWithoutUpsilon(InstId),
    UpsilonTargetNotPhi(InstId),
    LinearConsumedTwice(InstId),
    LinearNotConsumed(InstId),
    EmptyFunction(FuncId),
}

pub struct ValidationResult {
    pub errors: Vec<ValidationError>,
}

impl ValidationResult {
    pub fn is_ok(&self) -> bool {
        self.errors.is_empty()
    }
}

pub fn validate(module: &Module) -> ValidationResult {
    let mut errors = Vec::new();
    for func in &module.functions {
        errors.extend(validate_function(func).errors);
    }
    ValidationResult { errors }
}

pub fn validate_function(func: &Function) -> ValidationResult {
    let mut errors = Vec::new();

    if func.blocks.is_empty() {
        errors.push(ValidationError::EmptyFunction(func.id));
        return ValidationResult { errors };
    }

    for block in &func.blocks {
        validate_block(block, &mut errors);
    }

    let phi_ids: HashSet<InstId> = func
        .blocks
        .iter()
        .flat_map(|b| b.insts.iter())
        .filter(|i| i.opcode == Opcode::Phi)
        .map(|i| i.id)
        .collect();

    for block in &func.blocks {
        for inst in &block.insts {
            if inst.opcode == Opcode::Upsilon
                && let InstData::PhiTarget(target) = &inst.data
                && !phi_ids.contains(target)
            {
                errors.push(ValidationError::UpsilonTargetNotPhi(inst.id));
            }
        }
    }

    let upsilon_targets: HashSet<InstId> = func
        .blocks
        .iter()
        .flat_map(|b| b.insts.iter())
        .filter(|i| i.opcode == Opcode::Upsilon)
        .filter_map(|i| {
            if let InstData::PhiTarget(target) = &i.data {
                Some(*target)
            } else {
                None
            }
        })
        .collect();

    for &phi_id in &phi_ids {
        if !upsilon_targets.contains(&phi_id) {
            errors.push(ValidationError::PhiWithoutUpsilon(phi_id));
        }
    }

    check_linear_types(func, &mut errors);

    ValidationResult { errors }
}

fn check_linear_types(func: &Function, errors: &mut Vec<ValidationError>) {
    use std::collections::HashMap;
    let mut consume_count: HashMap<InstId, usize> = func
        .blocks
        .iter()
        .flat_map(|b| b.insts.iter())
        .filter(|i| i.ty == Ty::LinearPtr)
        .map(|i| (i.id, 0usize))
        .collect();
    if consume_count.is_empty() {
        return;
    }
    for block in &func.blocks {
        for inst in &block.insts {
            if inst.opcode == Opcode::LinearConsume {
                for &arg in &inst.args {
                    if let Some(n) = consume_count.get_mut(&arg) {
                        *n += 1;
                    }
                }
            }
        }
    }
    for (id, count) in consume_count {
        match count {
            0 => errors.push(ValidationError::LinearNotConsumed(id)),
            1 => {}
            _ => errors.push(ValidationError::LinearConsumedTwice(id)),
        }
    }
}

fn validate_block(block: &BasicBlock, errors: &mut Vec<ValidationError>) {
    let inst_ids: HashSet<InstId> = block.insts.iter().map(|i| i.id).collect();
    let mut found_terminal = false;

    for inst in &block.insts {
        if found_terminal {
            errors.push(ValidationError::MultipleTerminators(block.id));
            break;
        }
        if inst.opcode.is_terminal() {
            found_terminal = true;
        }

        for &arg in &inst.args {
            if !inst_ids.contains(&arg) {
                errors.push(ValidationError::UndefinedInstRef {
                    user: inst.id,
                    referenced: arg,
                });
            }
        }

        if inst.opcode == Opcode::Branch
            && let Some(&cond_id) = inst.args.first()
            && let Some(cond_inst) = block.insts.iter().find(|i| i.id == cond_id)
            && cond_inst.ty != Ty::Bool
        {
            errors.push(ValidationError::TypeMismatch {
                inst: inst.id,
                expected: Ty::Bool,
                got: cond_inst.ty,
            });
        }
    }

    if !found_terminal {
        errors.push(ValidationError::BlockNotTerminated(block.id));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::{
        BasicBlock, BlockId, FuncId, Function, Inst, InstData, InstId, Module, Opcode, Ty,
    };
    use vow_syntax::span::Span;

    fn dummy_span() -> Span {
        Span::new(0, 0)
    }

    fn make_inst(id: u32, opcode: Opcode, ty: Ty, args: Vec<InstId>, data: InstData) -> Inst {
        Inst {
            id: InstId(id),
            opcode,
            ty,
            args,
            data,
            origin: dummy_span(),
        }
    }

    fn make_func(id: u32, name: &str, blocks: Vec<BasicBlock>) -> Function {
        Function {
            id: FuncId(id),
            name: name.to_string(),
            params: vec![],
            param_names: vec![],
            return_ty: Ty::Unit,
            effects: vec![],
            vows: vec![],
            blocks,
            local_names: std::collections::HashMap::new(),
        }
    }

    #[test]
    fn valid_simple_function() {
        let insts = vec![
            make_inst(0, Opcode::GetArg, Ty::I64, vec![], InstData::ArgIndex(0)),
            make_inst(1, Opcode::Return, Ty::Unit, vec![InstId(0)], InstData::None),
        ];
        let block = BasicBlock {
            id: BlockId(0),
            insts,
        };
        let func = make_func(0, "simple", vec![block]);
        let module = Module {
            name: "m".to_string(),
            functions: vec![func],
            strings: vec![],
            struct_layouts: vec![],
            enum_layouts: vec![],
        };
        assert!(validate(&module).is_ok());
    }

    #[test]
    fn empty_function_fails() {
        let func = make_func(0, "empty", vec![]);
        let module = Module {
            name: "m".to_string(),
            functions: vec![func],
            strings: vec![],
            struct_layouts: vec![],
            enum_layouts: vec![],
        };
        let result = validate(&module);
        assert!(!result.is_ok());
        assert!(
            result
                .errors
                .iter()
                .any(|e| matches!(e, ValidationError::EmptyFunction(_)))
        );
    }

    #[test]
    fn unterminated_block_fails() {
        let insts = vec![make_inst(
            0,
            Opcode::GetArg,
            Ty::I64,
            vec![],
            InstData::ArgIndex(0),
        )];
        let block = BasicBlock {
            id: BlockId(0),
            insts,
        };
        let func = make_func(0, "unterminated", vec![block]);
        let module = Module {
            name: "m".to_string(),
            functions: vec![func],
            strings: vec![],
            struct_layouts: vec![],
            enum_layouts: vec![],
        };
        let result = validate(&module);
        assert!(!result.is_ok());
        assert!(
            result
                .errors
                .iter()
                .any(|e| matches!(e, ValidationError::BlockNotTerminated(_)))
        );
    }

    #[test]
    fn upsilon_target_not_phi() {
        let insts = vec![
            make_inst(0, Opcode::GetArg, Ty::I64, vec![], InstData::ArgIndex(0)),
            make_inst(
                1,
                Opcode::Upsilon,
                Ty::Unit,
                vec![InstId(0)],
                InstData::PhiTarget(InstId(0)),
            ),
            make_inst(2, Opcode::Return, Ty::Unit, vec![], InstData::None),
        ];
        let block = BasicBlock {
            id: BlockId(0),
            insts,
        };
        let func = make_func(0, "upsilon_bad", vec![block]);
        let module = Module {
            name: "m".to_string(),
            functions: vec![func],
            strings: vec![],
            struct_layouts: vec![],
            enum_layouts: vec![],
        };
        let result = validate(&module);
        assert!(!result.is_ok());
        assert!(
            result
                .errors
                .iter()
                .any(|e| matches!(e, ValidationError::UpsilonTargetNotPhi(_)))
        );
    }

    #[test]
    fn phi_without_upsilon_fails() {
        let insts = vec![
            make_inst(0, Opcode::Phi, Ty::I64, vec![], InstData::None),
            make_inst(1, Opcode::Return, Ty::Unit, vec![], InstData::None),
        ];
        let block = BasicBlock {
            id: BlockId(0),
            insts,
        };
        let func = make_func(0, "phi_no_upsilon", vec![block]);
        let module = Module {
            name: "m".to_string(),
            functions: vec![func],
            strings: vec![],
            struct_layouts: vec![],
            enum_layouts: vec![],
        };
        let result = validate(&module);
        assert!(!result.is_ok());
        assert!(
            result
                .errors
                .iter()
                .any(|e| matches!(e, ValidationError::PhiWithoutUpsilon(_)))
        );
    }

    #[test]
    fn linear_not_consumed_fails() {
        let insts = vec![
            make_inst(
                0,
                Opcode::GetArg,
                Ty::LinearPtr,
                vec![],
                InstData::ArgIndex(0),
            ),
            make_inst(1, Opcode::Return, Ty::Unit, vec![], InstData::None),
        ];
        let block = BasicBlock {
            id: BlockId(0),
            insts,
        };
        let func = make_func(0, "linear_no_consume", vec![block]);
        let module = Module {
            name: "m".to_string(),
            functions: vec![func],
            strings: vec![],
            struct_layouts: vec![],
            enum_layouts: vec![],
        };
        let result = validate(&module);
        assert!(!result.is_ok());
        assert!(
            result
                .errors
                .iter()
                .any(|e| matches!(e, ValidationError::LinearNotConsumed(_)))
        );
    }

    #[test]
    fn linear_consumed_twice_fails() {
        let insts = vec![
            make_inst(
                0,
                Opcode::GetArg,
                Ty::LinearPtr,
                vec![],
                InstData::ArgIndex(0),
            ),
            make_inst(
                1,
                Opcode::LinearConsume,
                Ty::Unit,
                vec![InstId(0)],
                InstData::None,
            ),
            make_inst(
                2,
                Opcode::LinearConsume,
                Ty::Unit,
                vec![InstId(0)],
                InstData::None,
            ),
            make_inst(3, Opcode::Return, Ty::Unit, vec![], InstData::None),
        ];
        let block = BasicBlock {
            id: BlockId(0),
            insts,
        };
        let func = make_func(0, "linear_double_consume", vec![block]);
        let module = Module {
            name: "m".to_string(),
            functions: vec![func],
            strings: vec![],
            struct_layouts: vec![],
            enum_layouts: vec![],
        };
        let result = validate(&module);
        assert!(!result.is_ok());
        assert!(
            result
                .errors
                .iter()
                .any(|e| matches!(e, ValidationError::LinearConsumedTwice(_)))
        );
    }

    #[test]
    fn multiple_terminals_fail() {
        let insts = vec![
            make_inst(0, Opcode::Return, Ty::Unit, vec![], InstData::None),
            make_inst(1, Opcode::Return, Ty::Unit, vec![], InstData::None),
        ];
        let block = BasicBlock {
            id: BlockId(0),
            insts,
        };
        let func = make_func(0, "multi_term", vec![block]);
        let module = Module {
            name: "m".to_string(),
            functions: vec![func],
            strings: vec![],
            struct_layouts: vec![],
            enum_layouts: vec![],
        };
        let result = validate(&module);
        assert!(!result.is_ok());
        assert!(
            result
                .errors
                .iter()
                .any(|e| matches!(e, ValidationError::MultipleTerminators(_)))
        );
    }
}
