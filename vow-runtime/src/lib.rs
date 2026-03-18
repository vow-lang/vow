#![allow(clippy::missing_safety_doc)]

use std::cell::RefCell;
use std::collections::HashMap;
use std::ffi::{c_char, CStr};
use std::io::Write as _;
use std::sync::atomic::{AtomicI64, Ordering};
use std::sync::Mutex;

thread_local! {
    static LAST_STDOUT: RefCell<Vec<u8>> = const { RefCell::new(Vec::new()) };
    static LAST_STDERR: RefCell<Vec<u8>> = const { RefCell::new(Vec::new()) };
}

enum ProcessState {
    Running(std::process::Child),
    Completed { stdout: Vec<u8>, stderr: Vec<u8> },
}

static PROCESS_MAP: Mutex<Option<HashMap<i64, ProcessState>>> = Mutex::new(None);
static NEXT_PROCESS_HANDLE: AtomicI64 = AtomicI64::new(1);

fn process_map_init(
    map: &mut Option<HashMap<i64, ProcessState>>,
) -> &mut HashMap<i64, ProcessState> {
    map.get_or_insert_with(HashMap::new)
}

const TAG_I32: u8 = 0;
const TAG_I64: u8 = 1;
const TAG_F32: u8 = 2;
const TAG_F64: u8 = 3;
const TAG_BOOL: u8 = 4;
const TAG_U64: u8 = 5;

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
        TAG_U64 => format!("{payload}"),
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
    file_ptr: *const i8,
    offset: u32,
) {
    let blame_str = if blame == 0 { "Caller" } else { "Callee" };
    let desc = if desc_ptr.is_null() {
        std::borrow::Cow::Borrowed("")
    } else {
        unsafe { CStr::from_ptr(desc_ptr) }.to_string_lossy()
    };
    let file = if file_ptr.is_null() {
        std::borrow::Cow::Borrowed("")
    } else {
        unsafe { CStr::from_ptr(file_ptr) }.to_string_lossy()
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
        r#"{{"error":"VowViolation","vow_id":{vow_id},"blame":"{blame_str}","description":"{desc}","file":"{file}","offset":{offset}{values_json}}}"#
    );
    let human = format!(
        "vow violation: {desc}, blame={blame_str}, file={file}, offset={offset}{values_human}"
    );
    let _ = writeln!(std::io::stderr(), "{json}");
    let _ = writeln!(std::io::stderr(), "{human}");
    std::process::exit(1);
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn __vow_print_str(s: *const u8) {
    let v = unsafe { &*(s as *const VowVec) };
    let bytes = unsafe { std::slice::from_raw_parts(v.ptr, v.len) };
    let _ = std::io::stdout().write_all(bytes);
    let _ = std::io::stdout().flush();
}

#[unsafe(no_mangle)]
pub extern "C" fn __vow_print_i64(v: i64) {
    print!("{v}");
    let _ = std::io::stdout().flush();
}

#[unsafe(no_mangle)]
pub extern "C" fn __vow_print_u64(v: u64) {
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

// ---------------------------------------------------------------------------
// Trace instrumentation
// ---------------------------------------------------------------------------

#[unsafe(no_mangle)]
pub unsafe extern "C" fn __vow_trace_enter(fn_name_ptr: *const i8) {
    if fn_name_ptr.is_null() {
        return;
    }
    let name = unsafe { CStr::from_ptr(fn_name_ptr) }.to_string_lossy();
    let _ = writeln!(std::io::stderr(), r#"{{"event":"enter","fn":"{name}"}}"#);
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn __vow_trace_exit(fn_name_ptr: *const i8) {
    if fn_name_ptr.is_null() {
        return;
    }
    let name = unsafe { CStr::from_ptr(fn_name_ptr) }.to_string_lossy();
    let _ = writeln!(std::io::stderr(), r#"{{"event":"exit","fn":"{name}"}}"#);
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn __vow_trace_vow(fn_name_ptr: *const i8, vow_id: i64, passed: i64) {
    if fn_name_ptr.is_null() {
        return;
    }
    let name = unsafe { CStr::from_ptr(fn_name_ptr) }.to_string_lossy();
    let p = if passed != 0 { "true" } else { "false" };
    let _ = writeln!(
        std::io::stderr(),
        r#"{{"event":"vow","fn":"{name}","vow_id":{vow_id},"passed":{p}}}"#
    );
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
    // No-op: struct deallocation deferred to future work.
    // Typed free functions (__vow_string_free, __vow_vec_free_val, __vow_map_free) handle
    // collection types directly without needing arena headers.
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
pub extern "C" fn __vow_vec_new_val() -> *mut u8 {
    __vow_vec_new(8, 8)
}

unsafe fn __vow_vec_reserve(vec: *mut u8, additional: usize, elem_size: usize, elem_align: usize) {
    let v = unsafe { &mut *(vec as *mut VowVec) };
    let required = v.len + additional;
    if required <= v.cap {
        return;
    }
    let mut new_cap = if v.cap == 0 { VEC_INITIAL_CAP } else { v.cap };
    while new_cap < required {
        new_cap *= 2;
    }
    let old_size = v.cap * elem_size;
    let new_size = new_cap * elem_size;
    let new_ptr = if old_size == 0 {
        let layout = unsafe { std::alloc::Layout::from_size_align_unchecked(new_size, elem_align) };
        unsafe { std::alloc::alloc_zeroed(layout) }
    } else {
        let old_layout =
            unsafe { std::alloc::Layout::from_size_align_unchecked(old_size, elem_align) };
        unsafe { std::alloc::realloc(v.ptr, old_layout, new_size) }
    };
    if new_ptr.is_null() {
        std::process::abort();
    }
    v.ptr = new_ptr;
    v.cap = new_cap;
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn __vow_vec_push(
    vec: *mut u8,
    elem: *const u8,
    elem_size: usize,
    elem_align: usize,
) {
    unsafe { __vow_vec_reserve(vec, 1, elem_size, elem_align) };
    let v = unsafe { &mut *(vec as *mut VowVec) };
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
pub unsafe extern "C" fn __vow_vec_pop(vec: *mut u8) {
    let v = unsafe { &mut *(vec as *mut VowVec) };
    if v.len > 0 {
        v.len -= 1;
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn __vow_vec_set_val(vec: *mut u8, index: usize, value: i64) {
    let v = unsafe { &*(vec as *const VowVec) };
    if index >= v.len {
        let json = r#"{"error":"IndexOutOfBounds"}"#;
        let _ = writeln!(std::io::stderr(), "{json}");
        let _ = writeln!(std::io::stderr(), "index out of bounds");
        std::process::exit(1);
    }
    let elem_ptr = unsafe { v.ptr.add(index * 8) as *mut i64 };
    unsafe { *elem_ptr = value };
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

// ---------------------------------------------------------------------------
// String (VowVec<u8>) runtime
// ---------------------------------------------------------------------------

#[unsafe(no_mangle)]
pub unsafe extern "C" fn __vow_string_new(ptr: *const i8, len: usize) -> *mut u8 {
    let vec_ptr = __vow_vec_new(1, 1);
    if len > 0 && !ptr.is_null() {
        unsafe { __vow_vec_reserve(vec_ptr, len, 1, 1) };
        let v = unsafe { &mut *(vec_ptr as *mut VowVec) };
        unsafe { std::ptr::copy_nonoverlapping(ptr as *const u8, v.ptr, len) };
        v.len = len;
    }
    vec_ptr
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn __vow_string_from_cstr(ptr: *const i8) -> *mut u8 {
    if ptr.is_null() {
        return __vow_vec_new(1, 1);
    }
    let s = unsafe { CStr::from_ptr(ptr) };
    let bytes = s.to_bytes();
    unsafe { __vow_string_new(ptr, bytes.len()) }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn __vow_string_len(s: *const u8) -> usize {
    unsafe { __vow_vec_len(s) }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn __vow_string_eq(a: *const u8, b: *const u8) -> i64 {
    let va = unsafe { &*(a as *const VowVec) };
    let vb = unsafe { &*(b as *const VowVec) };
    if va.len != vb.len {
        return 0;
    }
    let sa = unsafe { std::slice::from_raw_parts(va.ptr, va.len) };
    let sb = unsafe { std::slice::from_raw_parts(vb.ptr, vb.len) };
    if sa == sb {
        1
    } else {
        0
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn __vow_string_contains(haystack: *const u8, needle: *const u8) -> i64 {
    let vh = unsafe { &*(haystack as *const VowVec) };
    let vn = unsafe { &*(needle as *const VowVec) };
    let sh = unsafe { std::slice::from_raw_parts(vh.ptr, vh.len) };
    let sn = unsafe { std::slice::from_raw_parts(vn.ptr, vn.len) };
    if sn.is_empty() {
        return 1;
    }
    if vn.len > vh.len {
        return 0;
    }
    for i in 0..=(vh.len - vn.len) {
        if sh[i..i + vn.len] == *sn {
            return 1;
        }
    }
    0
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn __vow_string_push_str(dest: *mut u8, src: *const u8) {
    let vs = unsafe { &*(src as *const VowVec) };
    if vs.len == 0 {
        return;
    }
    unsafe { __vow_vec_reserve(dest, vs.len, 1, 1) };
    let vd = unsafe { &mut *(dest as *mut VowVec) };
    unsafe { std::ptr::copy_nonoverlapping(vs.ptr, vd.ptr.add(vd.len), vs.len) };
    vd.len += vs.len;
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn __vow_string_from_i64(v: i64) -> *mut u8 {
    let s = v.to_string();
    unsafe { __vow_string_new(s.as_ptr() as *const i8, s.len()) }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn __vow_string_print(s: *const u8) {
    let v = unsafe { &*(s as *const VowVec) };
    let bytes = unsafe { std::slice::from_raw_parts(v.ptr, v.len) };
    let _ = std::io::stdout().write_all(bytes);
    let _ = std::io::stdout().flush();
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn __vow_string_byte_at(s: *const u8, idx: i64) -> i64 {
    let v = unsafe { &*(s as *const VowVec) };
    if idx < 0 || idx as usize >= v.len {
        return -1;
    }
    let bytes = unsafe { std::slice::from_raw_parts(v.ptr, v.len) };
    bytes[idx as usize] as i64
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn __vow_string_push_byte(s: *mut u8, byte: i64) {
    let b = byte as u8;
    unsafe { __vow_vec_push(s, &b as *const u8, 1, 1) };
}

// ---------------------------------------------------------------------------
// String utility builtins
// ---------------------------------------------------------------------------

#[unsafe(no_mangle)]
pub unsafe extern "C" fn __vow_string_substr(s: *const u8, start: i64, len: i64) -> *mut u8 {
    if s.is_null() {
        return __vow_vec_new(1, 1);
    }
    let v = unsafe { &*(s as *const VowVec) };
    let slen = v.len as i64;
    let clamped_start = start.clamp(0, slen) as usize;
    let clamped_len = len.clamp(0, slen - clamped_start as i64) as usize;
    let bytes = unsafe { std::slice::from_raw_parts(v.ptr, v.len) };
    unsafe { __vow_string_new(bytes[clamped_start..].as_ptr() as *const i8, clamped_len) }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn __vow_string_split(haystack: *const u8, separator: *const u8) -> *mut u8 {
    let result_vec = __vow_vec_new_val();
    if haystack.is_null() || separator.is_null() {
        return result_vec;
    }
    let vh = unsafe { &*(haystack as *const VowVec) };
    let vs = unsafe { &*(separator as *const VowVec) };
    let h = unsafe { std::slice::from_raw_parts(vh.ptr, vh.len) };
    let s = unsafe { std::slice::from_raw_parts(vs.ptr, vs.len) };

    if s.is_empty() {
        let str_vec = unsafe { __vow_string_new(h.as_ptr() as *const i8, h.len()) } as i64;
        unsafe { __vow_vec_push_val(result_vec, str_vec) };
        return result_vec;
    }

    let mut start = 0;
    while start <= h.len() {
        if let Some(pos) = h[start..].windows(s.len()).position(|w| w == s) {
            let piece = unsafe { __vow_string_new(h[start..].as_ptr() as *const i8, pos) } as i64;
            unsafe { __vow_vec_push_val(result_vec, piece) };
            start += pos + s.len();
        } else {
            let piece =
                unsafe { __vow_string_new(h[start..].as_ptr() as *const i8, h.len() - start) }
                    as i64;
            unsafe { __vow_vec_push_val(result_vec, piece) };
            break;
        }
    }
    result_vec
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn __vow_string_starts_with(s: *const u8, prefix: *const u8) -> i64 {
    if s.is_null() || prefix.is_null() {
        return 0;
    }
    let vs = unsafe { &*(s as *const VowVec) };
    let vp = unsafe { &*(prefix as *const VowVec) };
    let ss = unsafe { std::slice::from_raw_parts(vs.ptr, vs.len) };
    let sp = unsafe { std::slice::from_raw_parts(vp.ptr, vp.len) };
    if ss.starts_with(sp) {
        1
    } else {
        0
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn __vow_string_ends_with(s: *const u8, suffix: *const u8) -> i64 {
    if s.is_null() || suffix.is_null() {
        return 0;
    }
    let vs = unsafe { &*(s as *const VowVec) };
    let vp = unsafe { &*(suffix as *const VowVec) };
    let ss = unsafe { std::slice::from_raw_parts(vs.ptr, vs.len) };
    let sp = unsafe { std::slice::from_raw_parts(vp.ptr, vp.len) };
    if ss.ends_with(sp) {
        1
    } else {
        0
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn __vow_string_trim(s: *const u8) -> *mut u8 {
    if s.is_null() {
        return __vow_vec_new(1, 1);
    }
    let v = unsafe { &*(s as *const VowVec) };
    let bytes = unsafe { std::slice::from_raw_parts(v.ptr, v.len) };
    let trimmed = match std::str::from_utf8(bytes) {
        Ok(s) => s.trim(),
        Err(_) => return __vow_vec_new(1, 1),
    };
    unsafe { __vow_string_new(trimmed.as_ptr() as *const i8, trimmed.len()) }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn __vow_string_to_upper(s: *const u8) -> *mut u8 {
    if s.is_null() {
        return __vow_vec_new(1, 1);
    }
    let v = unsafe { &*(s as *const VowVec) };
    let bytes = unsafe { std::slice::from_raw_parts(v.ptr, v.len) };
    let upper = match std::str::from_utf8(bytes) {
        Ok(s) => s.to_uppercase(),
        Err(_) => return __vow_vec_new(1, 1),
    };
    unsafe { __vow_string_new(upper.as_ptr() as *const i8, upper.len()) }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn __vow_string_to_lower(s: *const u8) -> *mut u8 {
    if s.is_null() {
        return __vow_vec_new(1, 1);
    }
    let v = unsafe { &*(s as *const VowVec) };
    let bytes = unsafe { std::slice::from_raw_parts(v.ptr, v.len) };
    let lower = match std::str::from_utf8(bytes) {
        Ok(s) => s.to_lowercase(),
        Err(_) => return __vow_vec_new(1, 1),
    };
    unsafe { __vow_string_new(lower.as_ptr() as *const i8, lower.len()) }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn __vow_string_replace(
    s: *const u8,
    from: *const u8,
    to: *const u8,
) -> *mut u8 {
    if s.is_null() || from.is_null() || to.is_null() {
        return __vow_vec_new(1, 1);
    }
    let vs = unsafe { &*(s as *const VowVec) };
    let vf = unsafe { &*(from as *const VowVec) };
    let vt = unsafe { &*(to as *const VowVec) };
    let ss = unsafe { std::slice::from_raw_parts(vs.ptr, vs.len) };
    let sf = unsafe { std::slice::from_raw_parts(vf.ptr, vf.len) };
    let st = unsafe { std::slice::from_raw_parts(vt.ptr, vt.len) };
    let (ss_str, sf_str, st_str) = match (
        std::str::from_utf8(ss),
        std::str::from_utf8(sf),
        std::str::from_utf8(st),
    ) {
        (Ok(a), Ok(b), Ok(c)) => (a, b, c),
        _ => return __vow_vec_new(1, 1),
    };
    let result = ss_str.replace(sf_str, st_str);
    unsafe { __vow_string_new(result.as_ptr() as *const i8, result.len()) }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn __vow_string_join(vec_ptr: *const u8, sep: *const u8) -> *mut u8 {
    if vec_ptr.is_null() || sep.is_null() {
        return __vow_vec_new(1, 1);
    }
    let v = unsafe { &*(vec_ptr as *const VowVec) };
    let ptrs = unsafe { std::slice::from_raw_parts(v.ptr as *const i64, v.len) };

    let result = __vow_vec_new(1, 1);
    for (i, &str_ptr) in ptrs.iter().enumerate() {
        if i > 0 {
            unsafe { __vow_string_push_str(result, sep) };
        }
        unsafe { __vow_string_push_str(result, str_ptr as *const u8) };
    }
    result
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn __vow_parse_i64(s: *const u8) -> i64 {
    if s.is_null() {
        return 0;
    }
    let v = unsafe { &*(s as *const VowVec) };
    let bytes = unsafe { std::slice::from_raw_parts(v.ptr, v.len) };
    match std::str::from_utf8(bytes) {
        Ok(s) => s.trim().parse::<i64>().unwrap_or(0),
        Err(_) => 0,
    }
}

// ---------------------------------------------------------------------------
// Utility builtins
// ---------------------------------------------------------------------------

#[unsafe(no_mangle)]
pub unsafe extern "C" fn __vow_vec_sort(vec: *const u8) -> *mut u8 {
    let result = __vow_vec_new_val();
    if vec.is_null() {
        return result;
    }
    let v = unsafe { &*(vec as *const VowVec) };
    let src = unsafe { std::slice::from_raw_parts(v.ptr as *const i64, v.len) };
    let mut sorted: Vec<i64> = src.to_vec();
    sorted.sort_unstable();
    for &val in &sorted {
        unsafe { __vow_vec_push_val(result, val) };
    }
    result
}

#[unsafe(no_mangle)]
pub extern "C" fn __vow_time_unix() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn __vow_hex_encode(vec: *const u8) -> *mut u8 {
    if vec.is_null() {
        return __vow_vec_new(1, 1);
    }
    let v = unsafe { &*(vec as *const VowVec) };
    let vals = unsafe { std::slice::from_raw_parts(v.ptr as *const i64, v.len) };
    let mut hex = String::new();
    for &val in vals {
        hex.push_str(&format!("{:02x}", (val & 0xff) as u8));
    }
    unsafe { __vow_string_new(hex.as_ptr() as *const i8, hex.len()) }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn __vow_hex_decode(s: *const u8) -> *mut u8 {
    let result = __vow_vec_new_val();
    if s.is_null() {
        return result;
    }
    let v = unsafe { &*(s as *const VowVec) };
    let bytes = unsafe { std::slice::from_raw_parts(v.ptr, v.len) };
    let hex_str = match std::str::from_utf8(bytes) {
        Ok(s) => s,
        Err(_) => return result,
    };
    if hex_str.len() % 2 != 0 {
        return result;
    }
    let mut i = 0;
    while i < hex_str.len() {
        match u8::from_str_radix(&hex_str[i..i + 2], 16) {
            Ok(byte) => unsafe { __vow_vec_push_val(result, byte as i64) },
            Err(_) => return __vow_vec_new_val(),
        }
        i += 2;
    }
    result
}

// ---------------------------------------------------------------------------
// File I/O runtime
// ---------------------------------------------------------------------------

#[unsafe(no_mangle)]
pub unsafe extern "C" fn __vow_fs_read(path_ptr: *const u8) -> *mut u8 {
    if path_ptr.is_null() {
        return __vow_vec_new(1, 1);
    }
    let v = unsafe { &*(path_ptr as *const VowVec) };
    let bytes = unsafe { std::slice::from_raw_parts(v.ptr, v.len) };
    let path = match std::str::from_utf8(bytes) {
        Ok(s) => s,
        Err(_) => return __vow_vec_new(1, 1),
    };
    match std::fs::read(path) {
        Ok(bytes) => unsafe { __vow_string_new(bytes.as_ptr() as *const i8, bytes.len()) },
        Err(_) => __vow_vec_new(1, 1),
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn __vow_fs_write(path_ptr: *const u8, data_ptr: *const u8) -> i32 {
    if path_ptr.is_null() || data_ptr.is_null() {
        return -1;
    }
    let vp = unsafe { &*(path_ptr as *const VowVec) };
    let path_bytes = unsafe { std::slice::from_raw_parts(vp.ptr, vp.len) };
    let path = match std::str::from_utf8(path_bytes) {
        Ok(s) => s,
        Err(_) => return -1,
    };
    let vd = unsafe { &*(data_ptr as *const VowVec) };
    let bytes = unsafe { std::slice::from_raw_parts(vd.ptr, vd.len) };
    match std::fs::write(path, bytes) {
        Ok(_) => 0,
        Err(_) => -1,
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn __vow_fs_exists(path_ptr: *const u8) -> i64 {
    if path_ptr.is_null() {
        return 0;
    }
    let v = unsafe { &*(path_ptr as *const VowVec) };
    let bytes = unsafe { std::slice::from_raw_parts(v.ptr, v.len) };
    let path = match std::str::from_utf8(bytes) {
        Ok(s) => s,
        Err(_) => return 0,
    };
    if std::path::Path::new(path).exists() {
        1
    } else {
        0
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn __vow_fs_mkdir(path_ptr: *const u8) -> i64 {
    if path_ptr.is_null() {
        return -1;
    }
    let v = unsafe { &*(path_ptr as *const VowVec) };
    let bytes = unsafe { std::slice::from_raw_parts(v.ptr, v.len) };
    let path = match std::str::from_utf8(bytes) {
        Ok(s) => s,
        Err(_) => return -1,
    };
    match std::fs::create_dir_all(path) {
        Ok(_) => 0,
        Err(_) => -1,
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn __vow_fs_listdir(path_ptr: *const u8) -> *mut u8 {
    let result_vec = __vow_vec_new_val();
    if path_ptr.is_null() {
        return result_vec;
    }
    let v = unsafe { &*(path_ptr as *const VowVec) };
    let bytes = unsafe { std::slice::from_raw_parts(v.ptr, v.len) };
    let path = match std::str::from_utf8(bytes) {
        Ok(s) => s,
        Err(_) => return result_vec,
    };
    let entries = match std::fs::read_dir(path) {
        Ok(e) => e,
        Err(_) => return result_vec,
    };
    for entry in entries.flatten() {
        let name = entry.file_name();
        let name_str = name.to_string_lossy();
        let str_vec =
            unsafe { __vow_string_new(name_str.as_ptr() as *const i8, name_str.len()) } as i64;
        unsafe { __vow_vec_push_val(result_vec, str_vec) };
    }
    result_vec
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn __vow_fs_remove(path_ptr: *const u8) -> i64 {
    if path_ptr.is_null() {
        return -1;
    }
    let v = unsafe { &*(path_ptr as *const VowVec) };
    let bytes = unsafe { std::slice::from_raw_parts(v.ptr, v.len) };
    let path = match std::str::from_utf8(bytes) {
        Ok(s) => s,
        Err(_) => return -1,
    };
    match std::fs::remove_file(path) {
        Ok(_) => 0,
        Err(_) => -1,
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn __vow_fs_remove_dir(path_ptr: *const u8) -> i64 {
    if path_ptr.is_null() {
        return -1;
    }
    let v = unsafe { &*(path_ptr as *const VowVec) };
    let bytes = unsafe { std::slice::from_raw_parts(v.ptr, v.len) };
    let path = match std::str::from_utf8(bytes) {
        Ok(s) => s,
        Err(_) => return -1,
    };
    match std::fs::remove_dir_all(path) {
        Ok(_) => 0,
        Err(_) => -1,
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn __vow_fs_is_dir(path_ptr: *const u8) -> i64 {
    if path_ptr.is_null() {
        return 0;
    }
    let v = unsafe { &*(path_ptr as *const VowVec) };
    let bytes = unsafe { std::slice::from_raw_parts(v.ptr, v.len) };
    let path = match std::str::from_utf8(bytes) {
        Ok(s) => s,
        Err(_) => return 0,
    };
    if std::path::Path::new(path).is_dir() {
        1
    } else {
        0
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn __vow_fs_rename(old_ptr: *const u8, new_ptr: *const u8) -> i64 {
    if old_ptr.is_null() || new_ptr.is_null() {
        return -1;
    }
    let vo = unsafe { &*(old_ptr as *const VowVec) };
    let old_bytes = unsafe { std::slice::from_raw_parts(vo.ptr, vo.len) };
    let old_path = match std::str::from_utf8(old_bytes) {
        Ok(s) => s,
        Err(_) => return -1,
    };
    let vn = unsafe { &*(new_ptr as *const VowVec) };
    let new_bytes = unsafe { std::slice::from_raw_parts(vn.ptr, vn.len) };
    let new_path = match std::str::from_utf8(new_bytes) {
        Ok(s) => s,
        Err(_) => return -1,
    };
    match std::fs::rename(old_path, new_path) {
        Ok(_) => 0,
        Err(_) => -1,
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn __vow_eprintln_str(s: *const u8) {
    if !s.is_null() {
        let v = unsafe { &*(s as *const VowVec) };
        let bytes = unsafe { std::slice::from_raw_parts(v.ptr, v.len) };
        let _ = std::io::stderr().write_all(bytes);
        let _ = writeln!(std::io::stderr());
    }
}

#[unsafe(no_mangle)]
pub extern "C" fn __vow_args() -> *mut u8 {
    let result_vec = __vow_vec_new(8, 8);
    for arg in std::env::args().skip(1) {
        let str_vec = unsafe { __vow_string_new(arg.as_ptr() as *const i8, arg.len()) } as i64;
        unsafe { __vow_vec_push_val(result_vec, str_vec) };
    }
    result_vec
}

#[unsafe(no_mangle)]
pub extern "C" fn __vow_process_exit(code: i64) {
    std::process::exit(code as i32);
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn __vow_process_run(cmd_ptr: i64, args_ptr: i64) -> i64 {
    let cmd_vec = unsafe { &*(cmd_ptr as *const VowVec) };
    let cmd_bytes = unsafe { std::slice::from_raw_parts(cmd_vec.ptr, cmd_vec.len) };
    let cmd_str = match std::str::from_utf8(cmd_bytes) {
        Ok(s) => s,
        Err(_) => return -1,
    };

    let args_vec = unsafe { &*(args_ptr as *const VowVec) };
    let arg_ptrs = unsafe { std::slice::from_raw_parts(args_vec.ptr as *const i64, args_vec.len) };
    let mut args = Vec::new();
    for &arg_ptr in arg_ptrs {
        let av = unsafe { &*(arg_ptr as *const VowVec) };
        let ab = unsafe { std::slice::from_raw_parts(av.ptr, av.len) };
        match std::str::from_utf8(ab) {
            Ok(s) => args.push(s.to_string()),
            Err(_) => return -1,
        }
    }

    match std::process::Command::new(cmd_str).args(&args).output() {
        Ok(output) => {
            LAST_STDOUT.with(|cell| *cell.borrow_mut() = output.stdout);
            LAST_STDERR.with(|cell| *cell.borrow_mut() = output.stderr);
            output.status.code().unwrap_or(-1) as i64
        }
        Err(_) => -1,
    }
}

#[unsafe(no_mangle)]
pub extern "C" fn __vow_process_get_stdout() -> *mut u8 {
    LAST_STDOUT.with(|cell| {
        let bytes = cell.borrow();
        unsafe { __vow_string_new(bytes.as_ptr() as *const i8, bytes.len()) }
    })
}

#[unsafe(no_mangle)]
pub extern "C" fn __vow_process_get_stderr() -> *mut u8 {
    LAST_STDERR.with(|cell| {
        let bytes = cell.borrow();
        unsafe { __vow_string_new(bytes.as_ptr() as *const i8, bytes.len()) }
    })
}

// ---------------------------------------------------------------------------
// Non-blocking subprocess management
// ---------------------------------------------------------------------------

#[unsafe(no_mangle)]
pub unsafe extern "C" fn __vow_process_start(cmd_ptr: i64, args_ptr: i64) -> i64 {
    let cmd_vec = unsafe { &*(cmd_ptr as *const VowVec) };
    let cmd_bytes = unsafe { std::slice::from_raw_parts(cmd_vec.ptr, cmd_vec.len) };
    let cmd_str = match std::str::from_utf8(cmd_bytes) {
        Ok(s) => s,
        Err(_) => return -1,
    };

    let args_vec = unsafe { &*(args_ptr as *const VowVec) };
    let arg_ptrs = unsafe { std::slice::from_raw_parts(args_vec.ptr as *const i64, args_vec.len) };
    let mut args = Vec::new();
    for &arg_ptr in arg_ptrs {
        let av = unsafe { &*(arg_ptr as *const VowVec) };
        let ab = unsafe { std::slice::from_raw_parts(av.ptr, av.len) };
        match std::str::from_utf8(ab) {
            Ok(s) => args.push(s.to_string()),
            Err(_) => return -1,
        }
    }

    use std::process::{Command, Stdio};
    match Command::new(cmd_str)
        .args(&args)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
    {
        Ok(child) => {
            let handle = NEXT_PROCESS_HANDLE.fetch_add(1, Ordering::Relaxed);
            let mut guard = PROCESS_MAP.lock().unwrap();
            let map = process_map_init(&mut guard);
            map.insert(handle, ProcessState::Running(child));
            handle
        }
        Err(_) => -1,
    }
}

#[unsafe(no_mangle)]
pub extern "C" fn __vow_process_wait(handle: i64) -> i64 {
    let mut guard = PROCESS_MAP.lock().unwrap();
    let map = process_map_init(&mut guard);
    let state = match map.remove(&handle) {
        Some(s) => s,
        None => return -1,
    };
    match state {
        ProcessState::Running(child) => match child.wait_with_output() {
            Ok(output) => {
                let exit_code = output.status.code().unwrap_or(-1) as i64;
                map.insert(
                    handle,
                    ProcessState::Completed {
                        stdout: output.stdout,
                        stderr: output.stderr,
                    },
                );
                exit_code
            }
            Err(_) => -1,
        },
        ProcessState::Completed { stdout, stderr } => {
            map.insert(handle, ProcessState::Completed { stdout, stderr });
            0
        }
    }
}

#[unsafe(no_mangle)]
pub extern "C" fn __vow_process_stdout_for(handle: i64) -> *mut u8 {
    let guard = PROCESS_MAP.lock().unwrap();
    if let Some(Some(ProcessState::Completed { stdout, .. })) =
        guard.as_ref().map(|m| m.get(&handle))
    {
        unsafe { __vow_string_new(stdout.as_ptr() as *const i8, stdout.len()) }
    } else {
        unsafe { __vow_string_new(std::ptr::null(), 0) }
    }
}

#[unsafe(no_mangle)]
pub extern "C" fn __vow_process_stderr_for(handle: i64) -> *mut u8 {
    let guard = PROCESS_MAP.lock().unwrap();
    if let Some(Some(ProcessState::Completed { stderr, .. })) =
        guard.as_ref().map(|m| m.get(&handle))
    {
        unsafe { __vow_string_new(stderr.as_ptr() as *const i8, stderr.len()) }
    } else {
        unsafe { __vow_string_new(std::ptr::null(), 0) }
    }
}

// ---------------------------------------------------------------------------
// HashMap runtime (open VowVec of (key:i64, val:i64) pairs — O(n) scan MVP)
// ---------------------------------------------------------------------------

#[repr(C)]
pub struct VowMap {
    pub ptr: *mut u8,
    pub len: usize,
    pub cap: usize,
}

const MAP_ENTRY_BYTES: usize = 16;
const MAP_INITIAL_CAP: usize = 8;

#[unsafe(no_mangle)]
pub extern "C" fn __vow_map_new() -> *mut u8 {
    let header_layout = unsafe { std::alloc::Layout::from_size_align_unchecked(24, 8) };
    let header_ptr = unsafe { std::alloc::alloc_zeroed(header_layout) } as *mut VowMap;
    if header_ptr.is_null() {
        std::process::abort();
    }
    let buf_size = MAP_INITIAL_CAP * MAP_ENTRY_BYTES;
    let buf_layout = unsafe { std::alloc::Layout::from_size_align_unchecked(buf_size, 8) };
    let buf_ptr = unsafe { std::alloc::alloc_zeroed(buf_layout) };
    if buf_ptr.is_null() {
        std::process::abort();
    }
    unsafe {
        (*header_ptr).ptr = buf_ptr;
        (*header_ptr).len = 0;
        (*header_ptr).cap = MAP_INITIAL_CAP;
    }
    header_ptr as *mut u8
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn __vow_map_insert(map: *mut u8, key: i64, val: i64) {
    let m = unsafe { &mut *(map as *mut VowMap) };
    let entries = unsafe { std::slice::from_raw_parts_mut(m.ptr as *mut i64, m.len * 2) };
    for i in 0..m.len {
        if entries[i * 2] == key {
            entries[i * 2 + 1] = val;
            return;
        }
    }
    if m.len == m.cap {
        let old_size = m.cap * MAP_ENTRY_BYTES;
        let new_cap = m.cap * 2;
        let new_size = new_cap * MAP_ENTRY_BYTES;
        let old_layout = unsafe { std::alloc::Layout::from_size_align_unchecked(old_size, 8) };
        let new_ptr = unsafe { std::alloc::realloc(m.ptr, old_layout, new_size) };
        if new_ptr.is_null() {
            std::process::abort();
        }
        m.ptr = new_ptr;
        m.cap = new_cap;
        unsafe {
            let extra = new_ptr.add(old_size);
            std::ptr::write_bytes(extra, 0, new_size - old_size);
        }
    }
    let entries = unsafe { std::slice::from_raw_parts_mut(m.ptr as *mut i64, (m.len + 1) * 2) };
    entries[m.len * 2] = key;
    entries[m.len * 2 + 1] = val;
    m.len += 1;
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn __vow_map_get(map: *const u8, key: i64) -> i64 {
    let m = unsafe { &*(map as *const VowMap) };
    let entries = unsafe { std::slice::from_raw_parts(m.ptr as *const i64, m.len * 2) };
    for i in 0..m.len {
        if entries[i * 2] == key {
            return entries[i * 2 + 1];
        }
    }
    0
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn __vow_map_contains(map: *const u8, key: i64) -> bool {
    let m = unsafe { &*(map as *const VowMap) };
    let entries = unsafe { std::slice::from_raw_parts(m.ptr as *const i64, m.len * 2) };
    for i in 0..m.len {
        if entries[i * 2] == key {
            return true;
        }
    }
    false
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn __vow_map_remove(map: *mut u8, key: i64) {
    let m = unsafe { &mut *(map as *mut VowMap) };
    let entries = unsafe { std::slice::from_raw_parts_mut(m.ptr as *mut i64, m.len * 2) };
    for i in 0..m.len {
        if entries[i * 2] == key {
            let last = m.len - 1;
            if i != last {
                entries[i * 2] = entries[last * 2];
                entries[i * 2 + 1] = entries[last * 2 + 1];
            }
            m.len -= 1;
            return;
        }
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn __vow_map_len(map: *const u8) -> usize {
    let m = unsafe { &*(map as *const VowMap) };
    m.len
}

// ---------------------------------------------------------------------------
// Typed deallocation
// ---------------------------------------------------------------------------

#[unsafe(no_mangle)]
pub unsafe extern "C" fn __vow_string_free(s: *mut u8) {
    if s.is_null() {
        return;
    }
    let v = unsafe { &*(s as *const VowVec) };
    if v.cap > 0 && !v.ptr.is_null() {
        let buf_layout = unsafe { std::alloc::Layout::from_size_align_unchecked(v.cap, 1) };
        unsafe { std::alloc::dealloc(v.ptr, buf_layout) };
    }
    let header_layout = unsafe { std::alloc::Layout::from_size_align_unchecked(24, 8) };
    unsafe { std::alloc::dealloc(s, header_layout) };
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn __vow_vec_free_val(v: *mut u8) {
    if v.is_null() {
        return;
    }
    let vec = unsafe { &*(v as *const VowVec) };
    if vec.cap > 0 && !vec.ptr.is_null() {
        let buf_layout = unsafe { std::alloc::Layout::from_size_align_unchecked(vec.cap * 8, 8) };
        unsafe { std::alloc::dealloc(vec.ptr, buf_layout) };
    }
    let header_layout = unsafe { std::alloc::Layout::from_size_align_unchecked(24, 8) };
    unsafe { std::alloc::dealloc(v, header_layout) };
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn __vow_map_free(m: *mut u8) {
    if m.is_null() {
        return;
    }
    let map = unsafe { &*(m as *const VowMap) };
    if map.cap > 0 && !map.ptr.is_null() {
        let buf_layout =
            unsafe { std::alloc::Layout::from_size_align_unchecked(map.cap * MAP_ENTRY_BYTES, 8) };
        unsafe { std::alloc::dealloc(map.ptr, buf_layout) };
    }
    let header_layout = unsafe { std::alloc::Layout::from_size_align_unchecked(24, 8) };
    unsafe { std::alloc::dealloc(m, header_layout) };
}
