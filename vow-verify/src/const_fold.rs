use std::collections::HashMap;

use vow_ir::{Function, Inst, InstData, IntegerSignedness, IntegerType, IntegerWidth, Opcode, Ty};

use crate::c_emitter::{checked_integer_type, ir_ty_to_integer_type};

/// Wrap `bits` into the value range of `int_ty` (sign-extended when signed,
/// zero-extended when unsigned), keeping a 64-bit pattern.
fn normalize_const_bits(bits: u64, int_ty: IntegerType) -> u64 {
    let width = u32::from(int_ty.width.bits());
    if width >= 64 {
        return bits;
    }
    let mask = (1u64 << width) - 1;
    let low = bits & mask;
    if int_ty.signedness == IntegerSignedness::Signed && (low >> (width - 1)) & 1 == 1 {
        low | !mask
    } else {
        low
    }
}

/// The integer type a value of IR type `ty` folds as. A pointer folds as a
/// 64-bit unsigned address, so a null pointer constant is recognised whether the
/// IR types it as an integer or as a pointer.
fn narrow_int_type(ty: Ty) -> Option<IntegerType> {
    match ty {
        Ty::Ptr | Ty::LinearPtr => Some(IntegerType::U64),
        _ => ir_ty_to_integer_type(ty).filter(|t| t.width != IntegerWidth::W128),
    }
}

fn const_leaf_bits(inst: &Inst) -> Option<u64> {
    let int_ty = narrow_int_type(inst.ty)?;
    let raw = match inst.data {
        InstData::ConstI32(v) => i64::from(v) as u64,
        InstData::ConstI64(v) => v as u64,
        InstData::ConstU64(v) => v,
        InstData::ConstU8(v) => u64::from(v),
        _ => return None,
    };
    Some(normalize_const_bits(raw, int_ty))
}

/// Exact `a op b` for a checked operator, `None` when it overflows `int_ty`
/// (the operator aborts there, so no value flows on).
fn checked_const_bits(opcode: Opcode, a: u64, b: u64, int_ty: IntegerType) -> Option<u64> {
    let exact = match int_ty.signedness {
        IntegerSignedness::Signed => {
            let (x, y) = (a as i64, b as i64);
            match opcode {
                Opcode::CheckedAdd => x.checked_add(y),
                Opcode::CheckedSub => x.checked_sub(y),
                _ => x.checked_mul(y),
            }
            .map(|r| r as u64)
        }
        IntegerSignedness::Unsigned => match opcode {
            Opcode::CheckedAdd => a.checked_add(b),
            Opcode::CheckedSub => a.checked_sub(b),
            _ => a.checked_mul(b),
        },
    }?;
    (normalize_const_bits(exact, int_ty) == exact).then_some(exact)
}

fn fold_const_inst(inst: &Inst, known: &HashMap<u32, u64>) -> Option<u64> {
    if let Some(bits) = const_leaf_bits(inst) {
        return Some(bits);
    }
    let arg = |i: usize| inst.args.get(i).and_then(|a| known.get(&a.0)).copied();
    match inst.opcode {
        Opcode::WrappingAdd | Opcode::WrappingSub | Opcode::WrappingMul => {
            let int_ty = checked_integer_type(inst)?;
            let (a, b) = (arg(0)?, arg(1)?);
            let raw = match inst.opcode {
                Opcode::WrappingAdd => a.wrapping_add(b),
                Opcode::WrappingSub => a.wrapping_sub(b),
                _ => a.wrapping_mul(b),
            };
            Some(normalize_const_bits(raw, int_ty))
        }
        Opcode::CheckedAdd | Opcode::CheckedSub | Opcode::CheckedMul => {
            let int_ty = checked_integer_type(inst)?;
            checked_const_bits(inst.opcode, arg(0)?, arg(1)?, int_ty)
        }
        Opcode::IntCast => {
            let to = match inst.data {
                InstData::IntegerCast { to, .. } => Some(to),
                _ => narrow_int_type(inst.ty),
            }
            .filter(|t| t.width != IntegerWidth::W128)?;
            Some(normalize_const_bits(arg(0)?, to))
        }
        _ => None,
    }
}

/// Constant integer value (as a type-normalized 64-bit pattern) of every
/// instruction in `func` that provably always computes one: literals, wrapping
/// and checked `+ - *` over constants, integer casts of constants, and a `Phi`
/// whose every `Upsilon` carries the same constant. Facts only ever grow from
/// known operands, so the fixpoint is sound and cyclic phis stay unknown.
pub(crate) fn fold_const_bits(func: &Function) -> HashMap<u32, u64> {
    let mut phi_sources: HashMap<u32, Vec<u32>> = HashMap::new();
    for block in &func.blocks {
        for inst in &block.insts {
            if inst.opcode == Opcode::Upsilon
                && let (InstData::PhiTarget(phi), Some(src)) = (&inst.data, inst.args.first())
            {
                phi_sources.entry(phi.0).or_default().push(src.0);
            }
        }
    }
    let mut known: HashMap<u32, u64> = HashMap::new();
    loop {
        let mut changed = false;
        for block in &func.blocks {
            for inst in &block.insts {
                if known.contains_key(&inst.id.0) {
                    continue;
                }
                let folded = if inst.opcode == Opcode::Phi {
                    phi_sources.get(&inst.id.0).and_then(|srcs| {
                        let first = known.get(srcs.first()?).copied()?;
                        srcs.iter()
                            .all(|s| known.get(s) == Some(&first))
                            .then_some(first)
                    })
                } else {
                    fold_const_inst(inst, &known)
                };
                if let Some(bits) = folded {
                    known.insert(inst.id.0, bits);
                    changed = true;
                }
            }
        }
        if !changed {
            return known;
        }
    }
}
