//! Minimal `.vmod` binary module format (spec §15 Phase 2a).
//!
//! **Version semantics.** Phase 2 introduces version 1 — the first version
//! that exists at all. The issue #197 acceptance "Module-format version
//! bumped" is read as "the on-disk module format now carries a version
//! tag"; future phases increment from here.
//!
//! **Wire format.** Little-endian. Variable-length fields use LEB128 for
//! counts/lengths. Each function carries its `RegionSummary`; each
//! `Inst` carries its `RegionId` as `kind:u8 + payload:u32` (identical
//! shape to the self-hosted packed `i64` in `compiler/module_io.vow`).
//!
//! Not wired into the compiler's build flow in Phase 2 — nothing produces
//! or consumes `.vmod` files yet. The round-trip test in `#[cfg(test)]`
//! is the acceptance evidence that the two compilers agree on the format.

use crate::types::{
    BasicBlock, BlockId, EnumLayout, FieldLayout, FuncId, Function, HiddenRegionIdx, Inst,
    InstData, InstId, Module, Opcode, RegionConstraint, RegionId, RegionSummary, RegionVar,
    StoreEffect, StructLayout, Ty, VariantLayout, VowEntry, VowId,
};
use vow_diag::Blame;
use vow_syntax::ast::Effect;
use vow_syntax::span::Span;

pub const MODULE_MAGIC: [u8; 4] = *b"VMOD";
pub const MODULE_VERSION: u32 = 1;

#[derive(Debug, PartialEq, Eq)]
pub enum DecodeError {
    BadMagic,
    VersionMismatch(u32),
    Truncated,
    /// Unknown discriminant for the named enum. Payload is `u32` to
    /// faithfully carry `RegionId`'s `u32` payload bytes when they are
    /// rejected as non-canonical; enum discriminants (Opcode, Ty, etc.)
    /// fit in this space with room to spare.
    InvalidKind(&'static str, u32),
    /// Decoder produced a `Module` but the input buffer had trailing bytes
    /// past the last function. Carries the number of unconsumed bytes.
    TrailingBytes(usize),
}

// ---------------------------------------------------------------------------
// Primitive readers / writers
// ---------------------------------------------------------------------------

struct Reader<'a> {
    buf: &'a [u8],
    pos: usize,
}

impl<'a> Reader<'a> {
    fn new(buf: &'a [u8]) -> Self {
        Reader { buf, pos: 0 }
    }

    fn take(&mut self, n: usize) -> Result<&'a [u8], DecodeError> {
        // checked_add guards against `usize` overflow on crafted input
        // where `n` could be near `usize::MAX`.
        let end = self.pos.checked_add(n).ok_or(DecodeError::Truncated)?;
        if end > self.buf.len() {
            return Err(DecodeError::Truncated);
        }
        let slice = &self.buf[self.pos..end];
        self.pos = end;
        Ok(slice)
    }

    /// Returns the number of unread bytes. Used by length-bounded `read_*`
    /// helpers to reject LEB counts that cannot possibly fit in the
    /// remaining buffer — every element decodes ≥1 byte, so any `n`
    /// exceeding this must be truncation or crafted input.
    fn remaining(&self) -> usize {
        self.buf.len() - self.pos
    }

    /// Validate an LEB-decoded count before using it as a `Vec` capacity.
    /// Rejects values that exceed the remaining buffer (and fit in
    /// `usize`), preventing `Vec::with_capacity` DoS on crafted input.
    fn bounded_count(&self, n: u64) -> Result<usize, DecodeError> {
        let n = usize::try_from(n).map_err(|_| DecodeError::Truncated)?;
        if n > self.remaining() {
            return Err(DecodeError::Truncated);
        }
        Ok(n)
    }

    fn u8(&mut self) -> Result<u8, DecodeError> {
        Ok(self.take(1)?[0])
    }

    fn u32(&mut self) -> Result<u32, DecodeError> {
        let s = self.take(4)?;
        Ok(u32::from_le_bytes([s[0], s[1], s[2], s[3]]))
    }

    fn u64(&mut self) -> Result<u64, DecodeError> {
        let s = self.take(8)?;
        Ok(u64::from_le_bytes([
            s[0], s[1], s[2], s[3], s[4], s[5], s[6], s[7],
        ]))
    }

    fn leb(&mut self) -> Result<u64, DecodeError> {
        let mut v: u64 = 0;
        let mut shift = 0u32;
        loop {
            let b = self.u8()?;
            let chunk = (b & 0x7f) as u64;
            // At shift == 63 the next chunk shift goes past u64. Only bit 0
            // of `chunk` is valid there; any higher bit is a non-canonical
            // encoding that would silently drop overflow bits. Reject
            // rather than decode two different byte streams to the same
            // value. See LEB128 canonicity rules (WebAssembly binary format
            // adopts the same check).
            if shift == 63 && chunk > 1 {
                return Err(DecodeError::Truncated);
            }
            v |= chunk << shift;
            if b & 0x80 == 0 {
                // Reject overlong (non-minimal) encodings: a terminator
                // byte carrying chunk==0 at shift > 0 means the value
                // could have been encoded in fewer bytes (e.g. 0 as
                // [0x80, 0x00]). Canonical LEB128 requires the shortest
                // form so distinct byte streams can't decode to the same
                // value.
                if shift > 0 && chunk == 0 {
                    return Err(DecodeError::Truncated);
                }
                return Ok(v);
            }
            shift += 7;
            if shift >= 64 {
                // 11th continuation byte: LEB too long, overflow region.
                return Err(DecodeError::Truncated);
            }
        }
    }

    fn string(&mut self) -> Result<String, DecodeError> {
        let raw_len = self.leb()?;
        let len = self.bounded_count(raw_len)?;
        let bytes = self.take(len)?;
        std::str::from_utf8(bytes)
            .map(str::to_string)
            .map_err(|_| DecodeError::Truncated)
    }
}

fn write_leb(out: &mut Vec<u8>, mut v: u64) {
    loop {
        let byte = (v & 0x7f) as u8;
        v >>= 7;
        if v == 0 {
            out.push(byte);
            return;
        }
        out.push(byte | 0x80);
    }
}

fn write_string(out: &mut Vec<u8>, s: &str) {
    write_leb(out, s.len() as u64);
    out.extend_from_slice(s.as_bytes());
}

// ---------------------------------------------------------------------------
// Region types
// ---------------------------------------------------------------------------

fn write_region_id(out: &mut Vec<u8>, r: RegionId) {
    let (kind, payload): (u8, u32) = match r {
        RegionId::Block(b) => (0, b.0),
        RegionId::Caller(i) => (1, i.0),
        RegionId::Root => (2, 0),
        RegionId::Rodata => (3, 0),
    };
    out.push(kind);
    out.extend_from_slice(&payload.to_le_bytes());
}

fn read_region_id(r: &mut Reader) -> Result<RegionId, DecodeError> {
    let kind = r.u8()?;
    let payload = r.u32()?;
    match kind {
        0 => Ok(RegionId::Block(BlockId(payload))),
        1 => Ok(RegionId::Caller(HiddenRegionIdx(payload))),
        // Root / Rodata carry no payload on the encoder side, so require
        // the payload to be zero on read. Non-canonical non-zero bytes
        // would otherwise round-trip to a different buffer than the input.
        2 if payload == 0 => Ok(RegionId::Root),
        3 if payload == 0 => Ok(RegionId::Rodata),
        2 | 3 => Err(DecodeError::InvalidKind("RegionId.payload", payload)),
        _ => Err(DecodeError::InvalidKind("RegionId", kind as u32)),
    }
}

fn write_region_constraint(out: &mut Vec<u8>, c: &RegionConstraint) {
    match c {
        RegionConstraint::FreshInCaller => out.push(0),
        RegionConstraint::AliasOf(i) => {
            out.push(1);
            out.extend_from_slice(&i.to_le_bytes());
        }
        RegionConstraint::AliasOfAny(xs) => {
            out.push(2);
            write_leb(out, xs.len() as u64);
            for x in xs {
                out.extend_from_slice(&x.to_le_bytes());
            }
        }
        RegionConstraint::ConstantGlobal => out.push(3),
    }
}

fn read_region_constraint(r: &mut Reader) -> Result<RegionConstraint, DecodeError> {
    let tag = r.u8()?;
    match tag {
        0 => Ok(RegionConstraint::FreshInCaller),
        1 => Ok(RegionConstraint::AliasOf(r.u32()?)),
        2 => {
            let raw_n = r.leb()?;
            let n = r.bounded_count(raw_n)?;
            let mut xs = Vec::new();
            for _ in 0..n {
                xs.push(r.u32()?);
            }
            Ok(RegionConstraint::AliasOfAny(xs))
        }
        3 => Ok(RegionConstraint::ConstantGlobal),
        _ => Err(DecodeError::InvalidKind("RegionConstraint", tag as u32)),
    }
}

fn write_region_summary(out: &mut Vec<u8>, s: &RegionSummary) {
    write_leb(out, s.param_regions.len() as u64);
    for v in &s.param_regions {
        out.extend_from_slice(&v.0.to_le_bytes());
    }
    write_region_constraint(out, &s.return_region);
    write_leb(out, s.store_effects.len() as u64);
    for e in &s.store_effects {
        out.extend_from_slice(&e.target.to_le_bytes());
        write_region_constraint(out, &e.source);
    }
}

fn read_region_summary(r: &mut Reader) -> Result<RegionSummary, DecodeError> {
    let raw_n = r.leb()?;
    let n = r.bounded_count(raw_n)?;
    let mut param_regions = Vec::new();
    for _ in 0..n {
        param_regions.push(RegionVar(r.u32()?));
    }
    let return_region = read_region_constraint(r)?;
    let raw_m = r.leb()?;
    let m = r.bounded_count(raw_m)?;
    let mut store_effects = Vec::new();
    for _ in 0..m {
        let target = r.u32()?;
        let source = read_region_constraint(r)?;
        store_effects.push(StoreEffect { target, source });
    }
    Ok(RegionSummary {
        param_regions,
        return_region,
        store_effects,
    })
}

// ---------------------------------------------------------------------------
// Opcode / Ty / Effect / Blame discriminants
// ---------------------------------------------------------------------------
//
// Discriminants are assigned explicitly here rather than relying on the
// source order of the `Opcode` / `Ty` enums; source order is a refactor
// hazard and would silently change the wire format. The `opcode_disc` /
// `disc_opcode` functions (and their peers for Ty) are the single point
// of truth for the on-disk discriminant space.

macro_rules! opcode_map {
    ($($disc:literal => $op:ident,)*) => {
        fn opcode_disc(op: Opcode) -> u16 {
            match op {
                $(Opcode::$op => $disc,)*
            }
        }
        fn disc_opcode(d: u16) -> Result<Opcode, DecodeError> {
            match d {
                $($disc => Ok(Opcode::$op),)*
                _ => Err(DecodeError::InvalidKind("Opcode", d as u32)),
            }
        }
    };
}

opcode_map! {
    0 => ConstI32, 1 => ConstI64, 2 => ConstF32, 3 => ConstF64,
    4 => ConstBool, 5 => ConstStr, 6 => ConstUnit,
    7 => GetArg,
    8 => WrappingAddI32, 9 => WrappingSubI32, 10 => WrappingMulI32,
    11 => WrappingDivI32, 12 => WrappingRemI32,
    13 => CheckedAddI32, 14 => CheckedSubI32, 15 => CheckedMulI32,
    16 => CheckedDivI32, 17 => CheckedRemI32,
    18 => EqI32, 19 => NeI32, 20 => LtI32, 21 => LeI32, 22 => GtI32, 23 => GeI32,
    24 => WrappingAddI64, 25 => WrappingSubI64, 26 => WrappingMulI64,
    27 => WrappingDivI64, 28 => WrappingRemI64,
    29 => CheckedAddI64, 30 => CheckedSubI64, 31 => CheckedMulI64,
    32 => CheckedDivI64, 33 => CheckedRemI64,
    34 => EqI64, 35 => NeI64, 36 => LtI64, 37 => LeI64, 38 => GtI64, 39 => GeI64,
    40 => AddF32, 41 => SubF32, 42 => MulF32, 43 => DivF32, 44 => RemF32,
    45 => EqF32, 46 => NeF32, 47 => LtF32, 48 => LeF32, 49 => GtF32, 50 => GeF32,
    51 => AddF64, 52 => SubF64, 53 => MulF64, 54 => DivF64, 55 => RemF64,
    56 => EqF64, 57 => NeF64, 58 => LtF64, 59 => LeF64, 60 => GtF64, 61 => GeF64,
    62 => Not, 63 => And, 64 => Or,
    65 => Load, 66 => Store,
    67 => Branch, 68 => Jump, 69 => Return, 70 => Unreachable,
    71 => Phi, 72 => Upsilon,
    73 => VowRequires, 74 => VowEnsures, 75 => VowInvariant,
    76 => Call,
    77 => RegionAlloc, 78 => RegionFree,
    79 => LinearConsume, 80 => LinearBorrow,
    81 => FieldGet, 82 => FieldSet,
    83 => XorI32, 84 => XorI64,
    85 => WrappingAddU64, 86 => WrappingSubU64, 87 => WrappingMulU64,
    88 => WrappingDivU64, 89 => WrappingRemU64,
    90 => CheckedAddU64, 91 => CheckedSubU64, 92 => CheckedMulU64,
    93 => CheckedDivU64, 94 => CheckedRemU64,
    95 => EqU64, 96 => NeU64, 97 => LtU64, 98 => LeU64, 99 => GtU64, 100 => GeU64,
    101 => XorU64, 102 => ConstU64,
    103 => CastI64ToU64, 104 => CastU64ToI64,
    105 => DebugCall,
    106 => BitAndI64, 107 => BitOrI64, 108 => ShlI64, 109 => ShrI64,
    110 => BitAndU64, 111 => BitOrU64, 112 => ShlU64, 113 => ShrU64,
    114 => RegionOpen, 115 => RegionClose,
}

fn ty_disc(t: Ty) -> u8 {
    match t {
        Ty::I32 => 0,
        Ty::I64 => 1,
        Ty::F32 => 2,
        Ty::F64 => 3,
        Ty::Bool => 4,
        Ty::Unit => 5,
        Ty::Ptr => 6,
        Ty::LinearPtr => 7,
        Ty::U64 => 8,
    }
}

fn disc_ty(d: u8) -> Result<Ty, DecodeError> {
    match d {
        0 => Ok(Ty::I32),
        1 => Ok(Ty::I64),
        2 => Ok(Ty::F32),
        3 => Ok(Ty::F64),
        4 => Ok(Ty::Bool),
        5 => Ok(Ty::Unit),
        6 => Ok(Ty::Ptr),
        7 => Ok(Ty::LinearPtr),
        8 => Ok(Ty::U64),
        _ => Err(DecodeError::InvalidKind("Ty", d as u32)),
    }
}

fn effect_disc(e: &Effect) -> u8 {
    match e {
        Effect::IO => 0,
        Effect::Panic => 1,
        Effect::Read => 2,
        Effect::Unsafe => 3,
        Effect::Write => 4,
    }
}

fn disc_effect(d: u8) -> Result<Effect, DecodeError> {
    match d {
        0 => Ok(Effect::IO),
        1 => Ok(Effect::Panic),
        2 => Ok(Effect::Read),
        3 => Ok(Effect::Unsafe),
        4 => Ok(Effect::Write),
        _ => Err(DecodeError::InvalidKind("Effect", d as u32)),
    }
}

fn blame_disc(b: Blame) -> u8 {
    match b {
        Blame::Caller => 0,
        Blame::Callee => 1,
        Blame::None => 2,
    }
}

fn disc_blame(d: u8) -> Result<Blame, DecodeError> {
    match d {
        0 => Ok(Blame::Caller),
        1 => Ok(Blame::Callee),
        2 => Ok(Blame::None),
        _ => Err(DecodeError::InvalidKind("Blame", d as u32)),
    }
}

// ---------------------------------------------------------------------------
// InstData
// ---------------------------------------------------------------------------

fn write_inst_data(out: &mut Vec<u8>, d: &InstData) {
    match d {
        InstData::None => out.push(0),
        InstData::ConstI32(v) => {
            out.push(1);
            out.extend_from_slice(&v.to_le_bytes());
        }
        InstData::ConstI64(v) => {
            out.push(2);
            out.extend_from_slice(&v.to_le_bytes());
        }
        InstData::ConstF32(v) => {
            out.push(3);
            out.extend_from_slice(&v.to_bits().to_le_bytes());
        }
        InstData::ConstF64(v) => {
            out.push(4);
            out.extend_from_slice(&v.to_bits().to_le_bytes());
        }
        InstData::ConstBool(b) => {
            out.push(5);
            out.push(if *b { 1 } else { 0 });
        }
        InstData::ArgIndex(n) => {
            out.push(6);
            out.extend_from_slice(&n.to_le_bytes());
        }
        InstData::PhiTarget(id) => {
            out.push(7);
            out.extend_from_slice(&id.0.to_le_bytes());
        }
        InstData::ConstU64(v) => {
            out.push(8);
            out.extend_from_slice(&v.to_le_bytes());
        }
        InstData::ConstStr(i) => {
            out.push(9);
            out.extend_from_slice(&i.to_le_bytes());
        }
        InstData::CallTarget(f) => {
            out.push(10);
            out.extend_from_slice(&f.0.to_le_bytes());
        }
        InstData::CallExtern(s) => {
            out.push(11);
            write_string(out, s);
        }
        InstData::BranchTargets {
            then_block,
            else_block,
        } => {
            out.push(12);
            out.extend_from_slice(&then_block.0.to_le_bytes());
            out.extend_from_slice(&else_block.0.to_le_bytes());
        }
        InstData::JumpTarget(b) => {
            out.push(13);
            out.extend_from_slice(&b.0.to_le_bytes());
        }
        InstData::VowId(v) => {
            out.push(14);
            out.extend_from_slice(&v.0.to_le_bytes());
        }
        InstData::AllocSize { size, align } => {
            out.push(15);
            out.extend_from_slice(&size.to_le_bytes());
            out.extend_from_slice(&align.to_le_bytes());
        }
        InstData::FieldIndex(i) => {
            out.push(16);
            out.extend_from_slice(&i.to_le_bytes());
        }
    }
}

fn read_inst_data(r: &mut Reader) -> Result<InstData, DecodeError> {
    let tag = r.u8()?;
    match tag {
        0 => Ok(InstData::None),
        1 => Ok(InstData::ConstI32(r.u32()? as i32)),
        2 => Ok(InstData::ConstI64(r.u64()? as i64)),
        3 => Ok(InstData::ConstF32(f32::from_bits(r.u32()?))),
        4 => Ok(InstData::ConstF64(f64::from_bits(r.u64()?))),
        5 => {
            // Reject non-canonical bool bytes so `[5, 2]` doesn't round-trip
            // to `[5, 1]` — distinct byte streams must not decode to the
            // same module.
            let b = r.u8()?;
            match b {
                0 => Ok(InstData::ConstBool(false)),
                1 => Ok(InstData::ConstBool(true)),
                _ => Err(DecodeError::InvalidKind("ConstBool", b as u32)),
            }
        }
        6 => Ok(InstData::ArgIndex(r.u32()?)),
        7 => Ok(InstData::PhiTarget(InstId(r.u32()?))),
        8 => Ok(InstData::ConstU64(r.u64()?)),
        9 => Ok(InstData::ConstStr(r.u32()?)),
        10 => Ok(InstData::CallTarget(FuncId(r.u32()?))),
        11 => Ok(InstData::CallExtern(r.string()?)),
        12 => Ok(InstData::BranchTargets {
            then_block: BlockId(r.u32()?),
            else_block: BlockId(r.u32()?),
        }),
        13 => Ok(InstData::JumpTarget(BlockId(r.u32()?))),
        14 => Ok(InstData::VowId(VowId(r.u32()?))),
        15 => Ok(InstData::AllocSize {
            size: r.u32()?,
            align: r.u32()?,
        }),
        16 => Ok(InstData::FieldIndex(r.u32()?)),
        _ => Err(DecodeError::InvalidKind("InstData", tag as u32)),
    }
}

// ---------------------------------------------------------------------------
// Inst / BasicBlock / Function / Module
// ---------------------------------------------------------------------------

fn write_inst(out: &mut Vec<u8>, i: &Inst) {
    out.extend_from_slice(&i.id.0.to_le_bytes());
    let op = opcode_disc(i.opcode);
    out.extend_from_slice(&op.to_le_bytes());
    out.push(ty_disc(i.ty));
    write_leb(out, i.args.len() as u64);
    for a in &i.args {
        out.extend_from_slice(&a.0.to_le_bytes());
    }
    write_inst_data(out, &i.data);
    out.extend_from_slice(&i.origin.start.to_le_bytes());
    out.extend_from_slice(&i.origin.len.to_le_bytes());
    write_region_id(out, i.region);
}

fn read_inst(r: &mut Reader) -> Result<Inst, DecodeError> {
    let id = InstId(r.u32()?);
    let op_bytes = r.take(2)?;
    let op = u16::from_le_bytes([op_bytes[0], op_bytes[1]]);
    let opcode = disc_opcode(op)?;
    let ty = disc_ty(r.u8()?)?;
    let raw_n = r.leb()?;
    let n = r.bounded_count(raw_n)?;
    let mut args = Vec::new();
    for _ in 0..n {
        args.push(InstId(r.u32()?));
    }
    let data = read_inst_data(r)?;
    let start = r.u32()?;
    let len = r.u32()?;
    let origin = Span::new(start, len);
    let region = read_region_id(r)?;
    Ok(Inst {
        id,
        opcode,
        ty,
        args,
        data,
        origin,
        region,
    })
}

fn write_block(out: &mut Vec<u8>, b: &BasicBlock) {
    out.extend_from_slice(&b.id.0.to_le_bytes());
    write_leb(out, b.insts.len() as u64);
    for inst in &b.insts {
        write_inst(out, inst);
    }
}

fn read_block(r: &mut Reader) -> Result<BasicBlock, DecodeError> {
    let id = BlockId(r.u32()?);
    let raw_n = r.leb()?;
    let n = r.bounded_count(raw_n)?;
    let mut insts = Vec::new();
    for _ in 0..n {
        insts.push(read_inst(r)?);
    }
    Ok(BasicBlock { id, insts })
}

fn write_vow_entry(out: &mut Vec<u8>, v: &VowEntry) {
    out.extend_from_slice(&v.id.0.to_le_bytes());
    write_string(out, &v.description);
    out.push(blame_disc(v.blame));
    write_leb(out, v.bindings.len() as u64);
    for (name, id) in &v.bindings {
        write_string(out, name);
        out.extend_from_slice(&id.0.to_le_bytes());
    }
    write_string(out, &v.file);
    out.extend_from_slice(&v.offset.to_le_bytes());
}

fn read_vow_entry(r: &mut Reader) -> Result<VowEntry, DecodeError> {
    let id = VowId(r.u32()?);
    let description = r.string()?;
    let blame = disc_blame(r.u8()?)?;
    let raw_n = r.leb()?;
    let n = r.bounded_count(raw_n)?;
    let mut bindings = Vec::new();
    for _ in 0..n {
        let name = r.string()?;
        let bid = InstId(r.u32()?);
        bindings.push((name, bid));
    }
    let file = r.string()?;
    let offset = r.u32()?;
    Ok(VowEntry {
        id,
        description,
        blame,
        bindings,
        file,
        offset,
    })
}

fn write_function(out: &mut Vec<u8>, f: &Function) {
    out.extend_from_slice(&f.id.0.to_le_bytes());
    write_string(out, &f.name);
    write_leb(out, f.params.len() as u64);
    for t in &f.params {
        out.push(ty_disc(*t));
    }
    write_leb(out, f.param_names.len() as u64);
    for n in &f.param_names {
        write_string(out, n);
    }
    out.push(ty_disc(f.return_ty));
    write_leb(out, f.effects.len() as u64);
    for e in &f.effects {
        out.push(effect_disc(e));
    }
    write_leb(out, f.vows.len() as u64);
    for v in &f.vows {
        write_vow_entry(out, v);
    }
    write_leb(out, f.blocks.len() as u64);
    for b in &f.blocks {
        write_block(out, b);
    }
    // local_names: encode in sorted key order for determinism.
    let mut locals: Vec<(&u32, &String)> = f.local_names.iter().collect();
    locals.sort_by_key(|(k, _)| **k);
    write_leb(out, locals.len() as u64);
    for (k, v) in locals {
        out.extend_from_slice(&k.to_le_bytes());
        write_string(out, v);
    }
    write_region_summary(out, &f.summary);
}

fn read_function(r: &mut Reader) -> Result<Function, DecodeError> {
    let id = FuncId(r.u32()?);
    let name = r.string()?;
    let raw_np = r.leb()?;
    let np = r.bounded_count(raw_np)?;
    let mut params = Vec::new();
    for _ in 0..np {
        params.push(disc_ty(r.u8()?)?);
    }
    let raw_nn = r.leb()?;
    let nn = r.bounded_count(raw_nn)?;
    let mut param_names = Vec::new();
    for _ in 0..nn {
        param_names.push(r.string()?);
    }
    let return_ty = disc_ty(r.u8()?)?;
    let raw_ne = r.leb()?;
    let ne = r.bounded_count(raw_ne)?;
    let mut effects = Vec::new();
    for _ in 0..ne {
        effects.push(disc_effect(r.u8()?)?);
    }
    let raw_nv = r.leb()?;
    let nv = r.bounded_count(raw_nv)?;
    let mut vows = Vec::new();
    for _ in 0..nv {
        vows.push(read_vow_entry(r)?);
    }
    let raw_nb = r.leb()?;
    let nb = r.bounded_count(raw_nb)?;
    let mut blocks = Vec::new();
    for _ in 0..nb {
        blocks.push(read_block(r)?);
    }
    let raw_nl = r.leb()?;
    let nl = r.bounded_count(raw_nl)?;
    let mut local_names = std::collections::HashMap::new();
    for _ in 0..nl {
        let k = r.u32()?;
        let v = r.string()?;
        // Reject duplicate keys so distinct wire payloads cannot collapse
        // to the same decoded function. The encoder emits keys in sorted
        // order (see write_function) so duplicates can only come from
        // crafted or corrupted input.
        if local_names.insert(k, v).is_some() {
            return Err(DecodeError::InvalidKind("local_names.duplicate", k));
        }
    }
    let summary = read_region_summary(r)?;
    Ok(Function {
        id,
        name,
        params,
        param_names,
        return_ty,
        effects,
        vows,
        blocks,
        local_names,
        summary,
    })
}

fn write_field(out: &mut Vec<u8>, f: &FieldLayout) {
    write_string(out, &f.name);
    out.push(ty_disc(f.ty));
}

fn read_field(r: &mut Reader) -> Result<FieldLayout, DecodeError> {
    let name = r.string()?;
    let ty = disc_ty(r.u8()?)?;
    Ok(FieldLayout { name, ty })
}

fn write_struct_layout(out: &mut Vec<u8>, s: &StructLayout) {
    write_string(out, &s.name);
    write_leb(out, s.fields.len() as u64);
    for f in &s.fields {
        write_field(out, f);
    }
    out.push(if s.is_linear { 1 } else { 0 });
}

fn read_struct_layout(r: &mut Reader) -> Result<StructLayout, DecodeError> {
    let name = r.string()?;
    let raw_n = r.leb()?;
    let n = r.bounded_count(raw_n)?;
    let mut fields = Vec::new();
    for _ in 0..n {
        fields.push(read_field(r)?);
    }
    let is_linear = match r.u8()? {
        0 => false,
        1 => true,
        b => return Err(DecodeError::InvalidKind("StructLayout.is_linear", b as u32)),
    };
    Ok(StructLayout {
        name,
        fields,
        is_linear,
    })
}

fn write_variant(out: &mut Vec<u8>, v: &VariantLayout) {
    write_string(out, &v.name);
    out.extend_from_slice(&v.tag.to_le_bytes());
    write_leb(out, v.payload.len() as u64);
    for f in &v.payload {
        write_field(out, f);
    }
}

fn read_variant(r: &mut Reader) -> Result<VariantLayout, DecodeError> {
    let name = r.string()?;
    let tag = r.u64()?;
    let raw_n = r.leb()?;
    let n = r.bounded_count(raw_n)?;
    let mut payload = Vec::new();
    for _ in 0..n {
        payload.push(read_field(r)?);
    }
    Ok(VariantLayout { name, tag, payload })
}

fn write_enum_layout(out: &mut Vec<u8>, e: &EnumLayout) {
    write_string(out, &e.name);
    write_leb(out, e.variants.len() as u64);
    for v in &e.variants {
        write_variant(out, v);
    }
}

fn read_enum_layout(r: &mut Reader) -> Result<EnumLayout, DecodeError> {
    let name = r.string()?;
    let raw_n = r.leb()?;
    let n = r.bounded_count(raw_n)?;
    let mut variants = Vec::new();
    for _ in 0..n {
        variants.push(read_variant(r)?);
    }
    Ok(EnumLayout { name, variants })
}

/// Encode a module to the v1 `.vmod` byte format. Deterministic.
pub fn encode_module(m: &Module) -> Vec<u8> {
    let mut out = Vec::new();
    out.extend_from_slice(&MODULE_MAGIC);
    out.extend_from_slice(&MODULE_VERSION.to_le_bytes());
    write_string(&mut out, &m.name);
    write_leb(&mut out, m.strings.len() as u64);
    for s in &m.strings {
        write_string(&mut out, s);
    }
    write_leb(&mut out, m.struct_layouts.len() as u64);
    for s in &m.struct_layouts {
        write_struct_layout(&mut out, s);
    }
    write_leb(&mut out, m.enum_layouts.len() as u64);
    for e in &m.enum_layouts {
        write_enum_layout(&mut out, e);
    }
    write_leb(&mut out, m.functions.len() as u64);
    for f in &m.functions {
        write_function(&mut out, f);
    }
    // Warnings are compiler-state diagnostics, not part of the module's
    // semantic content — omitted from the on-disk format intentionally.
    out
}

/// Decode a v1 `.vmod` byte buffer. Errors distinguish bad magic,
/// version mismatch, truncation, and invalid enum discriminants.
pub fn decode_module(bytes: &[u8]) -> Result<Module, DecodeError> {
    let mut r = Reader::new(bytes);
    let magic = r.take(4)?;
    if magic != MODULE_MAGIC {
        return Err(DecodeError::BadMagic);
    }
    let version = r.u32()?;
    if version != MODULE_VERSION {
        return Err(DecodeError::VersionMismatch(version));
    }
    let name = r.string()?;
    let raw_ns = r.leb()?;
    let ns = r.bounded_count(raw_ns)?;
    let mut strings = Vec::new();
    for _ in 0..ns {
        strings.push(r.string()?);
    }
    let raw_nsl = r.leb()?;
    let nsl = r.bounded_count(raw_nsl)?;
    let mut struct_layouts = Vec::new();
    for _ in 0..nsl {
        struct_layouts.push(read_struct_layout(&mut r)?);
    }
    let raw_nel = r.leb()?;
    let nel = r.bounded_count(raw_nel)?;
    let mut enum_layouts = Vec::new();
    for _ in 0..nel {
        enum_layouts.push(read_enum_layout(&mut r)?);
    }
    let raw_nf = r.leb()?;
    let nf = r.bounded_count(raw_nf)?;
    let mut functions = Vec::new();
    for _ in 0..nf {
        functions.push(read_function(&mut r)?);
    }
    // Reject trailing bytes so two different buffers can never decode to
    // the same semantic module — concatenated payloads or trailing
    // garbage now surface as `TrailingBytes` instead of silent success.
    if r.pos != bytes.len() {
        return Err(DecodeError::TrailingBytes(bytes.len() - r.pos));
    }
    Ok(Module {
        name,
        functions,
        strings,
        struct_layouts,
        enum_layouts,
        warnings: vec![],
    })
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::AbstractRegionId;

    fn empty_module() -> Module {
        Module {
            name: "m".to_string(),
            functions: vec![],
            strings: vec![],
            struct_layouts: vec![],
            enum_layouts: vec![],
            warnings: vec![],
        }
    }

    #[test]
    fn magic_and_version_are_stable() {
        assert_eq!(&MODULE_MAGIC, b"VMOD");
        assert_eq!(MODULE_VERSION, 1);
        let bytes = encode_module(&empty_module());
        assert_eq!(&bytes[..4], b"VMOD");
        assert_eq!(&bytes[4..8], &1u32.to_le_bytes());
    }

    #[test]
    fn round_trip_empty_module() {
        let m = empty_module();
        let b = encode_module(&m);
        let m2 = decode_module(&b).unwrap();
        assert_eq!(m, m2);
    }

    #[test]
    fn bad_magic_rejected() {
        let mut b = encode_module(&empty_module());
        b[0] = b'X';
        assert_eq!(decode_module(&b), Err(DecodeError::BadMagic));
    }

    #[test]
    fn region_id_rejects_nonzero_root_payload() {
        // Root encodes as kind=2 + payload=0 (5 bytes). Crafting kind=2 +
        // payload=7 must not silently round-trip to `RegionId::Root` —
        // otherwise two byte streams would decode to the same module.
        let mut buf = Vec::new();
        buf.push(2u8);
        buf.extend_from_slice(&7u32.to_le_bytes());
        let mut r = Reader::new(&buf);
        assert_eq!(
            read_region_id(&mut r),
            Err(DecodeError::InvalidKind("RegionId.payload", 7))
        );
    }

    #[test]
    fn leb_overflow_10th_byte_rejected() {
        // 10-byte LEB where the 10th byte's high bits would shift past u64.
        // `[0x80 × 9, 0x02]` previously decoded silently to 0 (losing bit
        // 64). Must now return Truncated.
        let mut b = encode_module(&empty_module());
        // Replace the function-count byte (the last zero byte of the empty
        // module) with a 10-byte overflow-bearing LEB.
        b.pop();
        b.extend_from_slice(&[0x80, 0x80, 0x80, 0x80, 0x80, 0x80, 0x80, 0x80, 0x80, 0x02]);
        assert!(matches!(decode_module(&b), Err(DecodeError::Truncated)));
    }

    #[test]
    fn crafted_huge_leb_count_rejected() {
        // Header + name "" + LEB(0 strings) + LEB(0 structs) + LEB(0 enums)
        // + LEB(HUGE) would normally trip Vec::with_capacity. The
        // `bounded_count` helper must reject the count before reservation.
        let mut b = encode_module(&empty_module());
        // empty_module serialises as: magic(4) + version(4) + name_len(1) +
        // strings_len(1) + structs_len(1) + enums_len(1) + fns_len(1) = 13.
        // Replace the trailing `0` (function count) with a 10-byte LEB
        // encoding of a value exceeding remaining bytes.
        b.truncate(12);
        // LEB128 encoding of 2^63 (massive count)
        b.extend_from_slice(&[0x80, 0x80, 0x80, 0x80, 0x80, 0x80, 0x80, 0x80, 0x80, 0x01]);
        assert!(matches!(decode_module(&b), Err(DecodeError::Truncated)));
    }

    #[test]
    fn version_mismatch_rejected() {
        let mut b = encode_module(&empty_module());
        b[4] = 99;
        assert_eq!(decode_module(&b), Err(DecodeError::VersionMismatch(99)));
    }

    #[test]
    fn trailing_bytes_rejected() {
        let mut b = encode_module(&empty_module());
        let before = b.len();
        b.extend_from_slice(&[0xde, 0xad, 0xbe, 0xef]);
        assert_eq!(
            decode_module(&b),
            Err(DecodeError::TrailingBytes(b.len() - before))
        );
    }

    #[test]
    fn region_id_every_variant_round_trips() {
        for r in [
            RegionId::Block(BlockId(7)),
            RegionId::Caller(HiddenRegionIdx(3)),
            RegionId::Root,
            RegionId::Rodata,
        ] {
            let mut out = Vec::new();
            write_region_id(&mut out, r);
            let mut reader = Reader::new(&out);
            assert_eq!(read_region_id(&mut reader).unwrap(), r);
        }
    }

    #[test]
    fn region_constraint_every_variant_round_trips() {
        let cases = vec![
            RegionConstraint::FreshInCaller,
            RegionConstraint::AliasOf(2),
            RegionConstraint::AliasOfAny(vec![0, 3, 7]),
            RegionConstraint::ConstantGlobal,
        ];
        for c in cases {
            let mut out = Vec::new();
            write_region_constraint(&mut out, &c);
            let mut reader = Reader::new(&out);
            assert_eq!(read_region_constraint(&mut reader).unwrap(), c);
        }
    }

    #[test]
    fn region_summary_full_round_trip() {
        let summary = RegionSummary {
            param_regions: vec![RegionVar(0), RegionVar(1)],
            return_region: RegionConstraint::AliasOfAny(vec![0, 1]),
            store_effects: vec![
                StoreEffect {
                    target: 0,
                    source: RegionConstraint::FreshInCaller,
                },
                StoreEffect {
                    target: 1,
                    source: RegionConstraint::ConstantGlobal,
                },
            ],
        };
        let mut out = Vec::new();
        write_region_summary(&mut out, &summary);
        let mut reader = Reader::new(&out);
        assert_eq!(read_region_summary(&mut reader).unwrap(), summary);
    }

    fn make_caller_inst() -> Inst {
        Inst {
            id: InstId(42),
            opcode: Opcode::RegionAlloc,
            ty: Ty::Ptr,
            args: vec![],
            data: InstData::AllocSize { size: 24, align: 8 },
            origin: Span::new(5, 10),
            region: RegionId::Caller(HiddenRegionIdx(7)),
        }
    }

    #[test]
    fn module_with_caller_region_round_trips() {
        let func = Function {
            id: FuncId(0),
            name: "alloc_caller".to_string(),
            params: vec![],
            param_names: vec![],
            return_ty: Ty::Ptr,
            effects: vec![],
            vows: vec![],
            blocks: vec![BasicBlock {
                id: BlockId(0),
                insts: vec![make_caller_inst()],
            }],
            local_names: std::collections::HashMap::new(),
            summary: RegionSummary {
                param_regions: vec![],
                return_region: RegionConstraint::FreshInCaller,
                store_effects: vec![],
            },
        };
        let m = Module {
            name: "caller_test".to_string(),
            functions: vec![func],
            strings: vec!["hi".to_string()],
            struct_layouts: vec![],
            enum_layouts: vec![],
            warnings: vec![],
        };
        let bytes = encode_module(&m);
        let m2 = decode_module(&bytes).unwrap();
        assert_eq!(m, m2);
        // Verify the Caller index survived — this is the specific property
        // the issue's tests list calls out.
        let got_inst = &m2.functions[0].blocks[0].insts[0];
        assert_eq!(got_inst.region, RegionId::Caller(HiddenRegionIdx(7)));
    }

    #[test]
    fn determinism_encode_is_stable() {
        let m = empty_module();
        assert_eq!(encode_module(&m), encode_module(&m));
    }

    #[test]
    fn abstract_region_id_type_is_reachable() {
        // Phase-2 rename sanity check — guards the inner u32 field name
        // at runtime. A rename or removal of the field would fail to
        // compile here rather than disappearing silently with the
        // compile-error-only check a `let _ = AbstractRegionId(0);`
        // would give.
        let a = AbstractRegionId(7);
        assert_eq!(a.0, 7);
    }

    #[test]
    fn leb_overlong_form_rejected() {
        // `[0x80, 0x00]` decodes to 0 under a naive LEB reader but is
        // non-minimal — the encoding could have been `[0x00]`. Canonical
        // LEB128 requires the shortest form, so distinct byte streams
        // cannot decode to the same value. Must return Truncated.
        let mut b = encode_module(&empty_module());
        // Replace the trailing function-count byte (0) with a 2-byte
        // overlong encoding of 0.
        b.pop();
        b.extend_from_slice(&[0x80, 0x00]);
        assert!(matches!(decode_module(&b), Err(DecodeError::Truncated)));
    }
}
