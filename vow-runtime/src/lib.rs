#![allow(clippy::missing_safety_doc)]

use std::ffi::{c_char, CStr};
use std::io::Write as _;

const TAG_I32: u8 = 0;
const TAG_I64: u8 = 1;
const TAG_F32: u8 = 2;
const TAG_F64: u8 = 3;
const TAG_BOOL: u8 = 4;

#[repr(C)]
pub struct VowBinding {
    pub name: *const c_char,
    pub tag: u8,
    _pad: [u8; 7],
    pub payload: u64,
}

fn fmt_payload(tag: u8, payload: u64) -> String {
    match tag {
        TAG_I32 => format!("{}", payload as i32),
        TAG_I64 => format!("{}", payload as i64),
        TAG_F32 => format!("{}", f32::from_bits(payload as u32)),
        TAG_F64 => format!("{}", f64::from_bits(payload)),
        TAG_BOOL => if payload != 0 { "true" } else { "false" }.to_string(),
        _ => format!("0x{payload:x}"),
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn __vow_violation(
    vow_id: u32,
    blame: u8,
    desc_ptr: *const i8,
    bindings_ptr: *const VowBinding,
    binding_count: u32,
) {
    let blame_str = if blame == 0 { "Caller" } else { "Callee" };
    let desc = if desc_ptr.is_null() {
        std::borrow::Cow::Borrowed("")
    } else {
        unsafe { CStr::from_ptr(desc_ptr) }.to_string_lossy()
    };

    let (values_json, values_human) = if binding_count > 0 {
        let mut json_pairs = String::new();
        let mut human_pairs = String::new();
        for i in 0..binding_count as usize {
            let b = unsafe { &*bindings_ptr.add(i) };
            let name = unsafe { CStr::from_ptr(b.name) }.to_string_lossy();
            let val = fmt_payload(b.tag, b.payload);
            if i > 0 {
                json_pairs.push(',');
                human_pairs.push_str(", ");
            }
            json_pairs.push_str(&format!(r#""{name}":{val}"#));
            human_pairs.push_str(&format!("{name}={val}"));
        }
        (
            format!(r#","values":{{{json_pairs}}}"#),
            format!(", {human_pairs}"),
        )
    } else {
        (String::new(), String::new())
    };

    let json = format!(
        r#"{{"error":"VowViolation","vow_id":{vow_id},"blame":"{blame_str}","description":"{desc}"{values_json}}}"#
    );
    let human = format!("vow violation: {desc}, blame={blame_str}{values_human}");
    let _ = writeln!(std::io::stderr(), "{json}");
    let _ = writeln!(std::io::stderr(), "{human}");
    std::process::exit(1);
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn __vow_print_str(ptr: *const i8) {
    let s = unsafe { CStr::from_ptr(ptr) }.to_string_lossy();
    print!("{s}");
    let _ = std::io::stdout().flush();
}

#[unsafe(no_mangle)]
pub extern "C" fn __vow_print_i64(v: i64) {
    print!("{v}");
    let _ = std::io::stdout().flush();
}

#[unsafe(no_mangle)]
pub extern "C" fn __vow_arithmetic_overflow() {
    let json = r#"{"error":"ArithmeticOverflow"}"#;
    let _ = writeln!(std::io::stderr(), "{json}");
    let _ = writeln!(std::io::stderr(), "arithmetic overflow");
    std::process::exit(1);
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn __vow_unwrap_panic() {
    let json = r#"{"error":"UnwrapOnNone"}"#;
    let _ = writeln!(std::io::stderr(), "{json}");
    let _ = writeln!(std::io::stderr(), "unwrap on None");
    std::process::exit(1);
}

#[unsafe(no_mangle)]
pub extern "C" fn __vow_arena_alloc(size: usize, align: usize) -> *mut u8 {
    if size == 0 {
        return align as *mut u8;
    }
    let layout = unsafe { std::alloc::Layout::from_size_align_unchecked(size, align) };
    let ptr = unsafe { std::alloc::alloc_zeroed(layout) };
    if ptr.is_null() {
        std::process::abort();
    }
    ptr
}

#[unsafe(no_mangle)]
pub extern "C" fn __vow_arena_free(_ptr: *mut u8) {
    // MVP: no-op (memory leak); proper arena deallocation is future work
}

#[repr(C)]
pub struct VowVec {
    pub ptr: *mut u8,
    pub len: usize,
    pub cap: usize,
}

const VEC_INITIAL_CAP: usize = 8;

#[unsafe(no_mangle)]
pub extern "C" fn __vow_vec_new(elem_size: usize, align: usize) -> *mut u8 {
    let header_layout = unsafe { std::alloc::Layout::from_size_align_unchecked(24, 8) };
    let header_ptr = unsafe { std::alloc::alloc_zeroed(header_layout) } as *mut VowVec;
    if header_ptr.is_null() {
        std::process::abort();
    }
    let buf_size = VEC_INITIAL_CAP * elem_size;
    let buf_ptr = if buf_size > 0 {
        let buf_layout = unsafe { std::alloc::Layout::from_size_align_unchecked(buf_size, align) };
        let p = unsafe { std::alloc::alloc_zeroed(buf_layout) };
        if p.is_null() {
            std::process::abort();
        }
        p
    } else {
        align as *mut u8
    };
    unsafe {
        (*header_ptr).ptr = buf_ptr;
        (*header_ptr).len = 0;
        (*header_ptr).cap = VEC_INITIAL_CAP;
    }
    header_ptr as *mut u8
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn __vow_vec_push(
    vec: *mut u8,
    elem: *const u8,
    elem_size: usize,
    elem_align: usize,
) {
    let v = unsafe { &mut *(vec as *mut VowVec) };
    if v.len == v.cap {
        let old_size = v.cap * elem_size;
        let new_cap = if v.cap == 0 {
            VEC_INITIAL_CAP
        } else {
            v.cap * 2
        };
        let new_size = new_cap * elem_size;
        let old_layout =
            unsafe { std::alloc::Layout::from_size_align_unchecked(old_size, elem_align) };
        let new_ptr = if old_size == 0 {
            let new_layout =
                unsafe { std::alloc::Layout::from_size_align_unchecked(new_size, elem_align) };
            unsafe { std::alloc::alloc_zeroed(new_layout) }
        } else {
            unsafe { std::alloc::realloc(v.ptr, old_layout, new_size) }
        };
        if new_ptr.is_null() {
            std::process::abort();
        }
        v.ptr = new_ptr;
        v.cap = new_cap;
    }
    let dest = unsafe { v.ptr.add(v.len * elem_size) };
    unsafe { std::ptr::copy_nonoverlapping(elem, dest, elem_size) };
    v.len += 1;
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn __vow_vec_len(vec: *const u8) -> usize {
    let v = unsafe { &*(vec as *const VowVec) };
    v.len
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn __vow_vec_push_val(vec: *mut u8, value: i64) {
    let bytes = value.to_ne_bytes();
    unsafe { __vow_vec_push(vec, bytes.as_ptr(), 8, 8) };
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn __vow_vec_get_val(vec: *const u8, index: usize) -> i64 {
    let ptr = unsafe { __vow_vec_get_ptr(vec, index, 8) };
    unsafe { *(ptr as *const i64) }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn __vow_vec_get_ptr(
    vec: *const u8,
    index: usize,
    elem_size: usize,
) -> *const u8 {
    let v = unsafe { &*(vec as *const VowVec) };
    if index >= v.len {
        let json = r#"{"error":"IndexOutOfBounds"}"#;
        let _ = writeln!(std::io::stderr(), "{json}");
        let _ = writeln!(std::io::stderr(), "index out of bounds");
        std::process::exit(1);
    }
    unsafe { v.ptr.add(index * elem_size) as *const u8 }
}
