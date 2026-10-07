#![allow(clippy::missing_safety_doc)]

mod profile;
mod violation;

use profile::render_profile_report;
use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::ffi::{CStr, c_char};
use std::io::Write as _;
use std::ptr::NonNull;
use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, AtomicI64, AtomicU64, AtomicUsize, Ordering};
use violation::{ValueBinding, render_violation};

thread_local! {
    static LAST_STDOUT: RefCell<Vec<u8>> = const { RefCell::new(Vec::new()) };
    static LAST_STDERR: RefCell<Vec<u8>> = const { RefCell::new(Vec::new()) };
}

enum ProcessState {
    Running(std::process::Child),
    Completed { stdout: Vec<u8>, stderr: Vec<u8> },
}

struct FileReadState {
    reader: std::io::BufReader<std::fs::File>,
    line_buf: Vec<u8>,
    status: i64,
}

static PROCESS_MAP: Mutex<Option<HashMap<i64, ProcessState>>> = Mutex::new(None);
static NEXT_PROCESS_HANDLE: AtomicI64 = AtomicI64::new(1);
// Persistent stdout/stderr drain threads for handles being polled via
// __vow_process_poll_wait, so a chatty child (ESBMC) cannot deadlock on a full
// pipe between polls. Keyed by handle; joined when the child completes or is
// killed (issue #784).
type PollReaders = (
    std::thread::JoinHandle<Vec<u8>>,
    std::thread::JoinHandle<Vec<u8>>,
);
static POLL_READERS: Mutex<Option<HashMap<i64, PollReaders>>> = Mutex::new(None);
static FILE_READ_MAP: Mutex<Option<HashMap<i64, FileReadState>>> = Mutex::new(None);
static NEXT_FILE_READ_HANDLE: AtomicI64 = AtomicI64::new(1);

fn process_map_init(
    map: &mut Option<HashMap<i64, ProcessState>>,
) -> &mut HashMap<i64, ProcessState> {
    map.get_or_insert_with(HashMap::new)
}

fn file_read_map_init(
    map: &mut Option<HashMap<i64, FileReadState>>,
) -> &mut HashMap<i64, FileReadState> {
    map.get_or_insert_with(HashMap::new)
}

/// Reserved process exit status for any runtime abort — a contract
/// violation, arithmetic overflow, invalid division or remainder,
/// unwrap-on-None, index-out-of-bounds, region-literal mutation,
/// runtime-invariant violation, sanitizer trap, stack overflow, or
/// out-of-memory. A runtime abort is an environment/soundness failure, never
/// an application result, so it must terminate with a status that cannot be
/// confused with an application's own `return N` from `main` (issue #877).
/// 134 = 128 + SIGABRT, the conventional "aborted" status; it is what the
/// stack-overflow handler and `__vow_malloc` failure already use, so this
/// unifies every runtime abort on one reserved code. The structured JSON
/// envelope written to stderr still identifies which abort occurred.
const VOW_RUNTIME_ABORT_EXIT: i32 = 134;

#[repr(C)]
pub struct VowBinding {
    pub name: *const c_char,
    pub tag: u8,
    _pad: [u8; 7],
    pub payload: u64,
    pub payload_hi: u64,
}

// The two Cranelift writers hand-roll this layout rather than sharing it:
// `vow-codegen/src/cranelift_backend.rs` and `vow-clif-shim/src/lib.rs` (neither
// depends on this crate). Changing a field here means changing their
// `BINDING_*` constants in the same commit.
const _: () = assert!(core::mem::size_of::<VowBinding>() == 32);
const _: () = assert!(core::mem::align_of::<VowBinding>() == 8);
const _: () = assert!(core::mem::offset_of!(VowBinding, payload) == 16);
const _: () = assert!(core::mem::offset_of!(VowBinding, payload_hi) == 24);

#[unsafe(no_mangle)]
pub unsafe extern "C" fn __vow_violation(
    vow_id: u32,
    blame: u8,
    desc_ptr: *const c_char,
    bindings_ptr: *const VowBinding,
    binding_count: u32,
    file_ptr: *const c_char,
    offset: u32,
) {
    let decode = |ptr: *const c_char| -> String {
        if ptr.is_null() {
            String::new()
        } else {
            unsafe { CStr::from_ptr(ptr) }
                .to_string_lossy()
                .into_owned()
        }
    };
    let desc = decode(desc_ptr);
    let file = decode(file_ptr);
    // Decode the C ABI records into owned data first so the pure renderer can
    // borrow plain `&str`s, keeping it free of `unsafe` and raw pointers.
    let decoded: Vec<(String, u8, u128)> = (0..binding_count as usize)
        .map(|i| {
            let b = unsafe { &*bindings_ptr.add(i) };
            (
                decode(b.name),
                b.tag,
                u128::from(b.payload) | (u128::from(b.payload_hi) << 64),
            )
        })
        .collect();
    let bindings: Vec<ValueBinding<'_>> = decoded
        .iter()
        .map(|(name, tag, payload)| ValueBinding {
            name: name.as_str(),
            tag: *tag,
            payload: *payload,
        })
        .collect();

    let rendered = render_violation(vow_id, blame, &desc, &file, offset, &bindings);
    let _ = writeln!(std::io::stderr(), "{}", rendered.json);
    let _ = writeln!(std::io::stderr(), "{}", rendered.human);
    std::process::exit(VOW_RUNTIME_ABORT_EXIT);
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
pub unsafe extern "C" fn __vow_debug_str(s: *const u8) {
    if !s.is_null() {
        sanitize_on_read(s as usize, 0);
        let v = unsafe { &*(s as *const VowVec) };
        let bytes = unsafe { std::slice::from_raw_parts(v.ptr, v.len) };
        let _ = std::io::stderr().write_all(bytes);
        let _ = std::io::stderr().flush();
    }
}

#[unsafe(no_mangle)]
pub extern "C" fn __vow_debug_i64(v: i64) {
    let _ = write!(std::io::stderr(), "{v}");
    let _ = std::io::stderr().flush();
}

#[unsafe(no_mangle)]
pub extern "C" fn __vow_debug_u64(v: u64) {
    let _ = write!(std::io::stderr(), "{v}");
    let _ = std::io::stderr().flush();
}

#[unsafe(no_mangle)]
pub extern "C" fn __vow_arithmetic_overflow() {
    let json = r#"{"error":"ArithmeticOverflow"}"#;
    let _ = writeln!(std::io::stderr(), "{json}");
    let _ = writeln!(std::io::stderr(), "arithmetic overflow");
    std::process::exit(VOW_RUNTIME_ABORT_EXIT);
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn __vow_unwrap_panic() {
    let json = r#"{"error":"UnwrapOnNone"}"#;
    let _ = writeln!(std::io::stderr(), "{json}");
    let _ = writeln!(std::io::stderr(), "unwrap on None");
    std::process::exit(VOW_RUNTIME_ABORT_EXIT);
}

// Arena / rodata runtime-error emitters. Both print a JSON envelope to stderr
// then exit with VOW_RUNTIME_ABORT_EXIT. Not routed through vow-diag (see
// docs/design/arena_memory.md §13.3).
//
// Both helpers are **non-allocating**. They take &'static str operation names
// and emit to stderr via direct byte writes. oom_trap is called on allocation
// failure; a heap allocation here would itself fail under memory pressure and
// mask the structured OOM envelope.
fn oom_trap(operation: &'static str) -> ! {
    use std::io::Write;
    let stderr = std::io::stderr();
    let mut lock = stderr.lock();
    let _ = lock.write_all(b"{\"error\":\"OutOfMemory\",\"operation\":\"");
    let _ = lock.write_all(operation.as_bytes());
    let _ = lock.write_all(b"\"}\n");
    std::process::exit(VOW_RUNTIME_ABORT_EXIT);
}

fn region_literal_mutation_trap(operation: &'static str) -> ! {
    use std::io::Write;
    // VOW_CAP_RODATA marks read-only descriptors, including literals and
    // stdin_read_line scratch storage. Keep the hint useful for both origins.
    let hint: &[u8] = if operation.starts_with("String::") {
        b"hint: use String::from(literal) for literals; use pin_to_root(value) for read-only scratch strings\n"
    } else if operation.starts_with("Vec::") {
        b"hint: use Vec::from(literal) for literals; use pin_to_root(value) for read-only vectors\n"
    } else if operation.starts_with("HashMap::") {
        b"hint: construct a mutable HashMap and copy entries before mutating\n"
    } else {
        b"hint: obtain a mutable copy before mutation\n"
    };
    let stderr = std::io::stderr();
    let mut lock = stderr.lock();
    let _ = lock.write_all(b"{\"error\":\"RegionLiteralMutation\",\"operation\":\"");
    let _ = lock.write_all(operation.as_bytes());
    let _ = lock.write_all(b"\",\"origin\":\"rodata\"}\n");
    let _ = lock.write_all(hint);
    std::process::exit(VOW_RUNTIME_ABORT_EXIT);
}

fn runtime_invariant_trap(operation: &'static str, reason: &'static str) -> ! {
    use std::io::Write;
    let stderr = std::io::stderr();
    let mut lock = stderr.lock();
    let _ = lock.write_all(b"{\"error\":\"RuntimeInvariantViolation\",\"operation\":\"");
    let _ = lock.write_all(operation.as_bytes());
    let _ = lock.write_all(b"\",\"reason\":\"");
    let _ = lock.write_all(reason.as_bytes());
    let _ = lock.write_all(b"\"}\n");
    std::process::exit(VOW_RUNTIME_ABORT_EXIT);
}

fn null_arena_trap(operation: &'static str) -> ! {
    runtime_invariant_trap(operation, "null arena");
}

// ---------------------------------------------------------------------------
// Trace instrumentation
// ---------------------------------------------------------------------------

#[unsafe(no_mangle)]
pub unsafe extern "C" fn __vow_trace_enter(fn_name_ptr: *const c_char) {
    if fn_name_ptr.is_null() {
        return;
    }
    let name = unsafe { CStr::from_ptr(fn_name_ptr) }.to_string_lossy();
    let _ = writeln!(std::io::stderr(), r#"{{"event":"enter","fn":"{name}"}}"#);
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn __vow_trace_exit(fn_name_ptr: *const c_char) {
    if fn_name_ptr.is_null() {
        return;
    }
    let name = unsafe { CStr::from_ptr(fn_name_ptr) }.to_string_lossy();
    let _ = writeln!(std::io::stderr(), r#"{{"event":"exit","fn":"{name}"}}"#);
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn __vow_trace_vow(fn_name_ptr: *const c_char, vow_id: i64, passed: i64) {
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

// ---------------------------------------------------------------------------
// Performance operation counting
// ---------------------------------------------------------------------------

static PERF_OPERATION_COUNT: AtomicU64 = AtomicU64::new(0);

fn perf_operation_count_add(amount: u64) {
    let _ = PERF_OPERATION_COUNT.try_update(Ordering::Relaxed, Ordering::Relaxed, |count| {
        Some(count.saturating_add(amount))
    });
}

/// Count one executed operation in a `vow-perf` instrumented artifact.
///
/// Saturation preserves the strongest reportable lower bound instead of
/// wrapping a long-running measurement back to a deceptively small value.
#[unsafe(no_mangle)]
pub extern "C" fn __vow_perf_count() {
    perf_operation_count_add(1);
}

/// Attribute the caller-side operation and size-dependent work performed by
/// `__vow_vec_sort` in an instrumented artifact.
///
/// The synthetic cost is `1 + n * (2 + ceil(log2(n)))`: one call, one input
/// copy and output push per item, and an `n log n` sorting term. Saturating
/// arithmetic keeps an oversized measurement from wrapping to a false low.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn __vow_perf_count_vec_sort(vec: *const u8) {
    let len = if vec.is_null() {
        0
    } else {
        u64::try_from(unsafe { __vow_vec_len(vec) }).unwrap_or(u64::MAX)
    };
    let logarithm = if len <= 1 {
        0
    } else {
        u64::from((len - 1).ilog2() + 1)
    };
    let cost = len
        .saturating_mul(logarithm.saturating_add(2))
        .saturating_add(1);
    perf_operation_count_add(cost);
}

// HashMap uses a linear scan. Charge the full length even when the key is
// found early, since complexity declarations cover the worst case. An insert
// can additionally copy the whole buffer when it grows.
unsafe fn perf_map_cost(map: *const u8, scan_multiplier: u64) {
    let len = u64::try_from(unsafe { __vow_map_len(map) }).unwrap_or(u64::MAX);
    perf_operation_count_add(len.saturating_mul(scan_multiplier).saturating_add(1));
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn __vow_perf_count_map_contains(map: *const u8, _key: i64) {
    unsafe { perf_map_cost(map, 1) };
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn __vow_perf_count_map_get(map: *const u8, _key: i64) {
    unsafe { perf_map_cost(map, 1) };
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn __vow_perf_count_map_insert(map: *const u8, _key: i64, _value: i64) {
    unsafe { perf_map_cost(map, 2) };
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn __vow_perf_count_map_remove(map: *const u8, _key: i64) {
    unsafe { perf_map_cost(map, 1) };
}

/// Equality can compare every byte only when the lengths match.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn __vow_perf_count_string_eq(a: *const u8, b: *const u8) {
    let a_len = unsafe { __vow_string_len(a) };
    let b_len = unsafe { __vow_string_len(b) };
    let bytes = if a_len == b_len {
        u64::try_from(a_len).unwrap_or(u64::MAX)
    } else {
        0
    };
    perf_operation_count_add(bytes.saturating_add(1));
}

#[unsafe(no_mangle)]
pub extern "C" fn __vow_perf_counter_reset() {
    PERF_OPERATION_COUNT.store(0, Ordering::Relaxed);
}

#[unsafe(no_mangle)]
pub extern "C" fn __vow_perf_counter_read() -> u64 {
    PERF_OPERATION_COUNT.load(Ordering::Relaxed)
}

// ---------------------------------------------------------------------------
// Profile instrumentation
// ---------------------------------------------------------------------------

static PROFILE_COUNTERS: Mutex<Option<HashMap<&'static str, u64>>> = Mutex::new(None);

fn profile_counters_init<'a>(
    map: &'a mut Option<HashMap<&'static str, u64>>,
) -> &'a mut HashMap<&'static str, u64> {
    map.get_or_insert_with(HashMap::new)
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn __vow_profile_enter(fn_name_ptr: *const c_char) {
    if fn_name_ptr.is_null() {
        return;
    }
    // SAFETY: fn_name_ptr is a static C-string literal embedded in the binary.
    // It lives for the duration of the program, so we can treat it as 'static.
    let name: &'static str = unsafe { CStr::from_ptr(fn_name_ptr) }
        .to_str()
        .unwrap_or("?");
    let mut guard = PROFILE_COUNTERS.lock().unwrap();
    let counters = profile_counters_init(&mut guard);
    *counters.entry(name).or_insert(0) += 1;
}

#[unsafe(no_mangle)]
pub extern "C" fn __vow_profile_init() {
    use std::sync::Once;
    static INIT: Once = Once::new();
    INIT.call_once(|| {
        extern "C" fn report() {
            let guard = PROFILE_COUNTERS.lock().unwrap_or_else(|e| e.into_inner());
            if let Some(counters) = guard.as_ref() {
                let entries: Vec<(&str, u64)> = counters
                    .iter()
                    .map(|(name, count)| (*name, *count))
                    .collect();
                if let Some(report) = render_profile_report(&entries) {
                    let _ = write!(std::io::stderr(), "{report}");
                }
            }
        }
        unsafe {
            libc::atexit(report);
        }
    });
}

// ---------------------------------------------------------------------------
// Stack overflow detection
// ---------------------------------------------------------------------------

static STACK_DEPTH: AtomicI64 = AtomicI64::new(0);
static STACK_FN_NAME: AtomicU64 = AtomicU64::new(0);
// Stack boundary saved at init time (on the main stack, before any signal).
static STACK_BOTTOM: AtomicU64 = AtomicU64::new(0);
static STACK_TOP: AtomicU64 = AtomicU64::new(0);

// STACK_FN_NAME tracks the most recently entered function, not the full call
// chain. After a callee returns, the name may be stale (pointing to the callee
// rather than the caller). This is intentional: the diagnostic reports the
// "last known function" at overflow time, which is the deepest frame — exactly
// the function whose entry pushed the stack past the limit.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn __vow_stack_enter(fn_name_ptr: *const c_char) {
    STACK_DEPTH.fetch_add(1, Ordering::Relaxed);
    STACK_FN_NAME.store(fn_name_ptr as u64, Ordering::Relaxed);
}

#[unsafe(no_mangle)]
pub extern "C" fn __vow_stack_exit() {
    STACK_DEPTH.fetch_sub(1, Ordering::Relaxed);
}

#[unsafe(no_mangle)]
pub extern "C" fn __vow_init_stack_guard() {
    use std::sync::Once;
    static INIT: Once = Once::new();
    INIT.call_once(|| {
        // Record the main stack boundaries while we're on the main stack.
        // Use address of a local variable as a portable SP approximation.
        let local = 0u8;
        let sp_approx = &local as *const u8 as usize;
        let mut rl: libc::rlimit = unsafe { std::mem::zeroed() };
        if unsafe { libc::getrlimit(libc::RLIMIT_STACK, &mut rl) } == 0
            && rl.rlim_cur != libc::RLIM_INFINITY
        {
            let stack_size = rl.rlim_cur as usize;
            // Stack grows downward. The guard page sits just below
            // `initial_SP - stack_size`. sp_approx is already `delta` bytes
            // below initial_SP (startup frames), so computed STACK_BOTTOM =
            // sp_approx - stack_size is `delta` below the actual bottom.
            // The guard page fault address lands inside [BOTTOM, TOP] as long
            // as delta >= PAGE_SIZE (~4KB), which holds on any realistic binary.
            STACK_BOTTOM.store(
                sp_approx.saturating_sub(stack_size) as u64,
                Ordering::Relaxed,
            );
            STACK_TOP.store(sp_approx.saturating_add(4096) as u64, Ordering::Relaxed);
        }

        unsafe {
            // Allocate an alternate signal stack so the SIGSEGV handler can run
            // even when the main stack is exhausted.
            let alt_stack_size = libc::SIGSTKSZ * 2;
            // Allocated once at process startup; owned for process lifetime (freed by OS on exit).
            let stack_mem = libc::mmap(
                std::ptr::null_mut(),
                alt_stack_size,
                libc::PROT_READ | libc::PROT_WRITE,
                libc::MAP_PRIVATE | libc::MAP_ANONYMOUS,
                -1,
                0,
            );
            if stack_mem == libc::MAP_FAILED {
                return;
            }
            let ss = libc::stack_t {
                ss_sp: stack_mem,
                ss_flags: 0,
                ss_size: alt_stack_size,
            };
            if libc::sigaltstack(&ss, std::ptr::null_mut()) != 0 {
                return;
            }

            // Install SIGSEGV handler on the alternate stack.
            let mut sa: libc::sigaction = std::mem::zeroed();
            sa.sa_flags = libc::SA_ONSTACK | libc::SA_SIGINFO;
            libc::sigemptyset(&mut sa.sa_mask);
            sa.sa_sigaction = stack_overflow_handler as *const () as usize;
            libc::sigaction(libc::SIGSEGV, &sa, std::ptr::null_mut());
        }
    });
}

unsafe extern "C" fn stack_overflow_handler(
    _sig: libc::c_int,
    info: *mut libc::siginfo_t,
    _ctx: *mut libc::c_void,
) {
    // Distinguish stack overflow from other SIGSEGVs by checking whether the
    // fault address falls within the main stack region (saved at init time).
    // If not a stack overflow, restore default handler and re-raise so the OS
    // produces a core dump.
    let bottom = STACK_BOTTOM.load(Ordering::Relaxed);
    let top = STACK_TOP.load(Ordering::Relaxed);
    if info.is_null() {
        // SA_SIGINFO guarantees non-null on Linux, but handle defensively.
        unsafe {
            libc::signal(libc::SIGSEGV, libc::SIG_DFL);
            libc::raise(libc::SIGSEGV);
        }
        return;
    }
    let fault_addr = unsafe { (*info).si_addr() } as u64;
    let is_stack_overflow = if bottom != 0 && top != 0 {
        fault_addr >= bottom && fault_addr <= top
    } else {
        // Bounds unknown (e.g. RLIM_INFINITY or getrlimit failed).
        false
    };
    if !is_stack_overflow {
        unsafe {
            libc::signal(libc::SIGSEGV, libc::SIG_DFL);
            libc::raise(libc::SIGSEGV);
        }
        return;
    }
    // Accepted heuristic limitation: [bottom, top] covers the entire main
    // stack region, not just the guard page. A use-after-return dereference of
    // a dead stack address could land in this window and be reported as
    // StackOverflow. Do not "tighten" the range to exclude live-stack addresses
    // without a precise guard-page boundary — doing so would silently break
    // real-overflow detection.

    // Read depth and function name (best-effort in signal context)
    let depth = STACK_DEPTH.load(Ordering::Relaxed);
    let fn_ptr = STACK_FN_NAME.load(Ordering::Relaxed) as *const c_char;

    let mut buf = [0u8; 512];
    let mut pos = 0;

    macro_rules! write_bytes {
        ($bytes:expr) => {
            let src = $bytes;
            let n = src.len().min(buf.len().saturating_sub(pos));
            buf[pos..pos + n].copy_from_slice(&src[..n]);
            pos += n;
        };
    }

    write_bytes!(b"{\"error\":\"StackOverflow\"");

    if depth > 0 {
        write_bytes!(b",\"depth\":");
        let mut num_buf = [0u8; 20];
        let num_str = format_i64_to_buf(depth, &mut num_buf);
        write_bytes!(num_str);
    }

    if !fn_ptr.is_null() {
        write_bytes!(b",\"function\":\"");
        // SAFETY: fn_ptr points into .rodata (codegen-emitted function name
        // global), so CStr::from_ptr is safe even in signal context.
        let name = unsafe { CStr::from_ptr(fn_ptr) };
        let name_bytes = name.to_bytes();
        let n = name_bytes
            .len()
            .min(buf.len().saturating_sub(pos).saturating_sub(3));
        buf[pos..pos + n].copy_from_slice(&name_bytes[..n]);
        pos += n;
        write_bytes!(b"\"");
    }

    write_bytes!(b"}\n");

    unsafe {
        libc::write(2, buf.as_ptr() as *const libc::c_void, pos);
    }

    // Also write human-readable line
    let mut hbuf = [0u8; 512];
    let mut hpos = 0;

    macro_rules! hwrite {
        ($bytes:expr) => {
            let src = $bytes;
            let n = src.len().min(hbuf.len().saturating_sub(hpos));
            hbuf[hpos..hpos + n].copy_from_slice(&src[..n]);
            hpos += n;
        };
    }

    hwrite!(b"stack overflow");

    if depth > 0 {
        hwrite!(b" at depth ");
        let mut num_buf = [0u8; 20];
        let num_str = format_i64_to_buf(depth, &mut num_buf);
        hwrite!(num_str);
    }

    if !fn_ptr.is_null() {
        hwrite!(b" in ");
        // SAFETY: fn_ptr points into .rodata (see JSON branch above).
        let name = unsafe { CStr::from_ptr(fn_ptr) };
        let name_bytes = name.to_bytes();
        let n = name_bytes
            .len()
            .min(hbuf.len().saturating_sub(hpos).saturating_sub(1));
        hbuf[hpos..hpos + n].copy_from_slice(&name_bytes[..n]);
        hpos += n;
    }

    hwrite!(b"\n");

    unsafe {
        libc::write(2, hbuf.as_ptr() as *const libc::c_void, hpos);
    }

    unsafe {
        libc::_exit(VOW_RUNTIME_ABORT_EXIT);
    }
}

fn format_i64_to_buf(mut val: i64, buf: &mut [u8; 20]) -> &[u8] {
    if val == 0 {
        buf[0] = b'0';
        return &buf[..1];
    }
    let negative = val < 0;
    if negative {
        val = val.checked_neg().unwrap_or(i64::MAX);
    }
    let mut pos = 20;
    while val > 0 {
        pos -= 1;
        buf[pos] = b'0' + (val % 10) as u8;
        val /= 10;
    }
    if negative {
        pos -= 1;
        buf[pos] = b'-';
    }
    &buf[pos..]
}

#[unsafe(no_mangle)]
pub extern "C" fn __vow_malloc(size: usize, align: usize) -> *mut u8 {
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
pub unsafe extern "C" fn __vow_free(ptr: *mut u8, size: usize, align: usize) {
    if size == 0 || ptr.is_null() {
        return;
    }
    let layout =
        std::alloc::Layout::from_size_align(size, align).expect("__vow_free: invalid layout");
    unsafe { std::alloc::dealloc(ptr, layout) };
}

// ---------------------------------------------------------------------------
// Arena primitive (docs/design/arena_memory.md §3)
// ---------------------------------------------------------------------------

// Sentinel capacity used by rodata-backed container descriptors. Any mutation
// entry point must trap with RegionLiteralMutation before any growth logic.
// See docs/design/arena_memory.md §6.1, §7.3.
pub const VOW_CAP_RODATA: usize = usize::MAX;

// Runtime-owned mutable Vec/String descriptors set the high bit of the public
// capacity word. The remaining bits store the actual capacity. Valid foreign
// descriptors leave this bit clear, so checked projection routing can reject
// them before consulting the private owner prefix. `VOW_CAP_RODATA` remains a
// distinct all-bits-set sentinel.
const VOW_CAP_RUNTIME_OWNED: usize = 1usize << (usize::BITS - 1);
const VOW_CAP_VALUE_MASK: usize = VOW_CAP_RUNTIME_OWNED - 1;

#[repr(C)]
pub struct VowArena {
    pub first_chunk: *mut u8,
    pub current_chunk: *mut u8,
    pub cursor: usize,
    pub chunk_end: usize,
    pub last_alloc_start: *mut u8,
    pub last_alloc_size: usize,
    pub retained_bytes: usize,
}

const _: () = assert!(core::mem::size_of::<VowArena>() == 56);

const CHUNK_PAYLOAD: usize = 4096;
// Chunk header layout: [next: 8 bytes][total | oversized-flag: 8 bytes].
// The `total` word at offset 8 records the chunk's libc::malloc size and
// also carries a high bit (CHUNK_OVERSIZED_FLAG) that records whether the
// chunk was allocated via __vow_arena_alloc's oversized path. The
// allocation-path flag — not the size — is what
// `arena_try_free_oversized_chunk` uses to classify chunks (issue #391):
// path-oversized chunks can have totals below, equal to, or above
// `normal_chunk_total()` (e.g. a 3000-byte single-resident string backing
// has total 3016 < 4112), so a size-only predicate would miss real
// oversized chunks in the 2049–4096 byte range.
const CHUNK_LINK_BYTES: usize = 16;
const CHUNK_TOTAL_OFFSET: usize = 8;
// Bit 62 (not 63) is safe because the __vow_arena_alloc overflow guard
// keeps `bytes + align <= isize::MAX`; the largest possible `total` is
// `16 + bytes + (align - 1)` which still fits below 2^62 in practice
// (any real malloc result is far below 2^48).
const CHUNK_OVERSIZED_FLAG: usize = 1usize << 62;
// Make the 64-bit requirement explicit. On a 32-bit target `1usize << 62`
// would be a compile-time shift overflow; the assert turns a cryptic
// constant-eval error into a clear diagnostic. The runtime is already
// implicitly 64-bit elsewhere (e.g. `size_of::<VowArena>() == 56`).
const _: () = assert!(
    usize::BITS == 64,
    "vow-runtime requires a 64-bit target (CHUNK_OVERSIZED_FLAG uses bit 62)"
);
const OVERSIZED_THRESHOLD: usize = 2048;

const fn normal_chunk_total() -> usize {
    CHUNK_LINK_BYTES + CHUNK_PAYLOAD
}

const fn oversized_chunk_total(bytes: usize, align: usize) -> usize {
    CHUNK_LINK_BYTES + bytes + (align - 1)
}

static MEMORY_CURRENT_BYTES: AtomicUsize = AtomicUsize::new(0);
static MEMORY_PEAK_BYTES: AtomicUsize = AtomicUsize::new(0);
static MEMORY_ROOT_ARENA_BYTES: AtomicUsize = AtomicUsize::new(0);
static MEMORY_ALLOC_COUNT: AtomicU64 = AtomicU64::new(0);

fn atomic_usize_add_saturating(counter: &AtomicUsize, delta: usize) -> usize {
    let mut current = counter.load(Ordering::Relaxed);
    loop {
        let next = current.saturating_add(delta);
        match counter.compare_exchange_weak(current, next, Ordering::Relaxed, Ordering::Relaxed) {
            Ok(_) => return next,
            Err(observed) => current = observed,
        }
    }
}

fn atomic_usize_sub_saturating(counter: &AtomicUsize, delta: usize) -> usize {
    let mut current = counter.load(Ordering::Relaxed);
    loop {
        let next = current.saturating_sub(delta);
        match counter.compare_exchange_weak(current, next, Ordering::Relaxed, Ordering::Relaxed) {
            Ok(_) => return next,
            Err(observed) => current = observed,
        }
    }
}

fn memory_update_peak(current: usize) {
    let mut peak = MEMORY_PEAK_BYTES.load(Ordering::Relaxed);
    while current > peak {
        match MEMORY_PEAK_BYTES.compare_exchange_weak(
            peak,
            current,
            Ordering::Relaxed,
            Ordering::Relaxed,
        ) {
            Ok(_) => break,
            Err(observed) => peak = observed,
        }
    }
}

fn arena_is_root(a: *mut VowArena) -> bool {
    std::ptr::addr_eq(a as *const VowArena, &raw const __vow_root_arena)
}

fn memory_note_chunk_alloc(a: *mut VowArena, bytes: usize) {
    let current = atomic_usize_add_saturating(&MEMORY_CURRENT_BYTES, bytes);
    memory_update_peak(current);
    if arena_is_root(a) {
        atomic_usize_add_saturating(&MEMORY_ROOT_ARENA_BYTES, bytes);
    }
}

fn memory_note_arena_release(a: *mut VowArena, bytes: usize) {
    atomic_usize_sub_saturating(&MEMORY_CURRENT_BYTES, bytes);
    if arena_is_root(a) {
        atomic_usize_sub_saturating(&MEMORY_ROOT_ARENA_BYTES, bytes);
    }
}

fn memory_note_alloc_request() {
    let _ = MEMORY_ALLOC_COUNT.try_update(Ordering::Relaxed, Ordering::Relaxed, |count| {
        Some(count.saturating_add(1))
    });
}

// libc::malloc a chunk of `total` bytes, zero the next-chunk link word at
// offset 0, and store `total` (OR-ed with CHUNK_OVERSIZED_FLAG if the
// allocation took the oversized path) at offset 8. Returns the base pointer
// or null on OOM; caller decides trap site.
unsafe fn alloc_chunk(total: usize, oversized: bool) -> *mut u8 {
    let base = unsafe { libc::malloc(total) } as *mut u8;
    if !base.is_null() {
        unsafe { set_next_chunk(base, core::ptr::null_mut()) };
        let flag = if oversized { CHUNK_OVERSIZED_FLAG } else { 0 };
        unsafe { set_chunk_total_word(base, total | flag) };
    }
    base
}

// The gate every chunk-header access passes through. `NonNull::new` splits
// the base pointer into "absent" and "present" in the type: the `None` arm
// traps, and the accessors below receive a `NonNull<u8>` that cannot be null
// by construction, so each one's dereference carries its own proof instead
// of inheriting an undocumented precondition from whichever chain walk
// called it. A null base means the chain itself is corrupt, which is a
// runtime invariant violation rather than a recoverable condition.
#[inline]
unsafe fn chunk_header(base: *const u8, operation: &'static str) -> NonNull<u8> {
    match NonNull::new(base.cast_mut()) {
        Some(header) => header,
        None => runtime_invariant_trap(operation, "null chunk"),
    }
}

// Read the total-size word written by `alloc_chunk`, masking off the
// oversized-flag bit.
unsafe fn chunk_total(base: *const u8) -> usize {
    let total = unsafe { chunk_total_word(base, "arena_chunk_total") };
    total & !CHUNK_OVERSIZED_FLAG
}

// True iff the chunk was allocated via __vow_arena_alloc's oversized path,
// regardless of its `total` size. This is the predicate
// `arena_try_free_oversized_chunk` consults to decide whether a chunk is
// single-resident and safe to free.
unsafe fn chunk_is_oversized(base: *const u8) -> bool {
    let total = unsafe { chunk_total_word(base, "arena_chunk_is_oversized") };
    total & CHUNK_OVERSIZED_FLAG != 0
}

// The raw total-size word at offset 8, flag bit included.
#[inline]
unsafe fn chunk_total_word(base: *const u8, operation: &'static str) -> usize {
    unsafe {
        chunk_header(base, operation)
            .add(CHUNK_TOTAL_OFFSET)
            .cast::<usize>()
            .read()
    }
}

// Write the total-size word. Only `alloc_chunk` calls this, at the point the
// chunk is created.
#[inline]
unsafe fn set_chunk_total_word(base: *mut u8, word: usize) {
    unsafe {
        chunk_header(base, "arena_set_chunk_total")
            .add(CHUNK_TOTAL_OFFSET)
            .cast::<usize>()
            .write(word)
    };
}

// Intrusive chunk-link accessors. Every chunk's first word (offset 0, within
// the CHUNK_LINK_BYTES header) holds a `*mut u8` link to the next chunk in the
// arena's chain — null at the tail — written by `alloc_chunk` at allocation
// time. These two functions are the ONLY places that reinterpret that word as
// a `*mut *mut u8`; every chain walk, link, and unlink in the allocator and its
// tests goes through them. Centralizing the cast keeps the single `unsafe`
// reinterpretation documented in one place and gives static analysers one
// narrow site to reason about instead of nine (issue #894).
//
// Safety: `chunk` must be a base pointer returned by `alloc_chunk` — a live
// `libc::malloc(total)` block with `total >= CHUNK_LINK_BYTES` (16) — and not
// yet freed. The 8-byte link word at offset 0 is therefore fully in-bounds and
// was initialized by `alloc_chunk`. The non-null half of that precondition is
// discharged by `chunk_header` rather than left to the caller: every chain
// walk already tests the link before recursing, but that test is a whole
// function away from the access, so neither a reader nor a static analyser
// can discharge it locally and a future walk that forgets it would fail
// silently into a read of address 0. The C ESBMC mirror in
// `vow-runtime/verify/arena.c` performs the identical `*(void**)chunk` access
// under the same non-null invariant, so the model check exercises this exact
// reasoning for every chunk that reaches the access.
#[inline]
unsafe fn next_chunk(chunk: *mut u8) -> *mut u8 {
    unsafe {
        chunk_header(chunk, "arena_next_chunk")
            .cast::<*mut u8>()
            .read()
    }
}

#[inline]
unsafe fn set_next_chunk(chunk: *mut u8, next: *mut u8) {
    unsafe {
        chunk_header(chunk, "arena_set_next_chunk")
            .cast::<*mut u8>()
            .write(next)
    };
}

// Align a raw address up to `align` (power of two).
fn align_up(addr: usize, align: usize) -> usize {
    (addr + align - 1) & !(align - 1)
}

// First usable address within a chunk for allocations of the given alignment.
// Usable space begins at offset 16 (after the next-link word and the total-size
// word — see CHUNK_LINK_BYTES).
unsafe fn chunk_usable_start(base: *mut u8, align: usize) -> usize {
    align_up(base as usize + CHUNK_LINK_BYTES, align)
}

// The single place an arena handle received across the C ABI becomes a Rust
// reference.
//
// The five exported `__vow_arena_*` entry points below are part of the
// runtime's C ABI: generated code calls them directly, so a null handle is
// reachable from outside this crate and must fail closed rather than be
// undefined behaviour. Routing every reborrow through `NonNull::new` puts
// that split in the type — the `None` arm traps with the same
// `null_arena_trap` every `*_in_arena` entry point already uses, and each
// caller below receives a `&mut VowArena` whose validity rustc tracks,
// rather than a raw pointer carrying an unwritten non-null precondition
// established by some distant caller.
//
// The lifetime is unconstrained by design: `a` points at a `VowArena` owned
// by the caller's frame (or at `__vow_root_arena` in `.bss`), which outlives
// the call. That is the same obligation the previous `&mut *a` carried; it
// is stated here once instead of at seven separate reborrows.
#[inline]
unsafe fn arena_mut<'a>(a: *mut VowArena, operation: &'static str) -> &'a mut VowArena {
    match NonNull::new(a) {
        Some(mut arena) => unsafe { arena.as_mut() },
        None => null_arena_trap(operation),
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn __vow_arena_init_closed(a: *mut VowArena) {
    let arena = unsafe { arena_mut(a, "arena_init_closed") };
    arena.first_chunk = core::ptr::null_mut();
    arena.current_chunk = core::ptr::null_mut();
    arena.cursor = 0;
    arena.chunk_end = 0;
    arena.last_alloc_start = core::ptr::null_mut();
    arena.last_alloc_size = 0;
    arena.retained_bytes = 0;
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn __vow_arena_open(a: *mut VowArena) {
    let arena = unsafe { arena_mut(a, "arena_open") };
    if !arena.first_chunk.is_null() {
        return;
    }

    let total = normal_chunk_total();
    let base = unsafe { alloc_chunk(total, false) };
    if base.is_null() {
        oom_trap("arena_open");
    }
    let arena = unsafe { arena_mut(a, "arena_open") };
    arena.first_chunk = base;
    arena.current_chunk = base;
    arena.cursor = unsafe { chunk_usable_start(base, 8) };
    arena.chunk_end = base as usize + total;
    arena.last_alloc_start = core::ptr::null_mut();
    arena.last_alloc_size = 0;
    arena.retained_bytes = total;
    memory_note_chunk_alloc(a, total);
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn __vow_arena_close(a: *mut VowArena) {
    let arena = unsafe { arena_mut(a, "arena_close") };
    let retained_bytes = arena.retained_bytes;
    let mut chunk = arena.first_chunk;
    while !chunk.is_null() {
        let next = unsafe { next_chunk(chunk) };
        unsafe { libc::free(chunk as *mut libc::c_void) };
        chunk = next;
    }
    // Zero all fields. Spec §3.3 leaves the post-close state unspecified,
    // and this zeroing choice makes a double-close a safe no-op (the loop
    // above walks a null chain) rather than a dangling-pointer walk.
    arena.first_chunk = core::ptr::null_mut();
    arena.current_chunk = core::ptr::null_mut();
    arena.cursor = 0;
    arena.chunk_end = 0;
    arena.last_alloc_start = core::ptr::null_mut();
    arena.last_alloc_size = 0;
    arena.retained_bytes = 0;
    memory_note_arena_release(a, retained_bytes);
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn __vow_arena_alloc(
    a: *mut VowArena,
    bytes: usize,
    align: usize,
) -> *mut u8 {
    let arena = unsafe { arena_mut(a, "arena_alloc") };
    // Overflow guard: all downstream arithmetic in this function
    // (`align_up`, the fit-check, `oversized_chunk_total`) sums `bytes`
    // and `align`, so both individually AND combined must fit in the
    // allocator's size limit (`isize::MAX`, Rust convention). Without
    // the combined check, `bytes == align == isize::MAX` would still
    // wrap `CHUNK_LINK_BYTES + bytes + (align - 1)` on 64-bit `usize`.
    let size_limit = isize::MAX as usize;
    if bytes > size_limit || align > size_limit || bytes.saturating_add(align) > size_limit {
        oom_trap("arena_alloc");
    }
    let aligned_cursor = align_up(arena.cursor, align);
    if aligned_cursor + bytes <= arena.chunk_end {
        arena.cursor = aligned_cursor + bytes;
        arena.last_alloc_start = aligned_cursor as *mut u8;
        arena.last_alloc_size = bytes;
        memory_note_alloc_request();
        return aligned_cursor as *mut u8;
    }
    // Need a new chunk. Use the oversized path whenever (a) bytes exceed the
    // threshold or (b) worst-case alignment padding could push past a normal
    // chunk's payload. (b) is inert today (all callers use align <= 8) but
    // keeps the `cursor <= chunk_end` invariant under any alignment.
    let oversized = bytes > OVERSIZED_THRESHOLD || bytes + (align - 1) > CHUNK_PAYLOAD;
    let total = if oversized {
        oversized_chunk_total(bytes, align)
    } else {
        normal_chunk_total()
    };
    let new_base = unsafe { alloc_chunk(total, oversized) };
    if new_base.is_null() {
        oom_trap("arena_alloc");
    }
    // Link new chunk as the tail.
    unsafe { set_next_chunk(arena.current_chunk, new_base) };
    arena.current_chunk = new_base;
    let start = unsafe { chunk_usable_start(new_base, align) };
    let chunk_end = new_base as usize + total;
    // Seal oversized chunks: leave no room for a subsequent allocation to
    // land in the alignment-slack tail. `arena_try_free_oversized_chunk`
    // identifies a chunk as reclaimable via `chunk_is_oversized()` (the
    // path flag recorded in this chunk's header at allocation time) and
    // frees it when the original backing is abandoned. The path flag alone
    // cannot guarantee single residency — without sealing, a later fast-
    // path allocation could land in the alignment-slack tail (up to
    // `align - 1` bytes between `start + bytes` and `chunk_end`) and the
    // subsequent free would dangle it. Sealing the cursor to `chunk_end`
    // enforces the single-resident invariant by construction at the cost
    // of `total - (start + bytes)` bytes of waste, bounded by `(align - 1)`
    // for the alignment-driven path. Normal chunks continue to use the
    // bump cursor as before.
    arena.cursor = if oversized { chunk_end } else { start + bytes };
    arena.chunk_end = chunk_end;
    arena.last_alloc_start = start as *mut u8;
    arena.last_alloc_size = bytes;
    arena.retained_bytes = arena.retained_bytes.saturating_add(total);
    memory_note_chunk_alloc(a, total);
    memory_note_alloc_request();
    start as *mut u8
}

#[unsafe(no_mangle)]
pub extern "C" fn __vow_memory_root_arena_bytes() -> u64 {
    MEMORY_ROOT_ARENA_BYTES.load(Ordering::Relaxed) as u64
}

#[unsafe(no_mangle)]
pub extern "C" fn __vow_memory_peak_bytes() -> u64 {
    MEMORY_PEAK_BYTES.load(Ordering::Relaxed) as u64
}

#[unsafe(no_mangle)]
pub extern "C" fn __vow_memory_alloc_count_since_start() -> u64 {
    MEMORY_ALLOC_COUNT.load(Ordering::Relaxed)
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn __vow_arena_try_extend(
    a: *mut VowArena,
    ptr: *mut u8,
    old_size: usize,
    new_size: usize,
) -> i64 {
    let arena = unsafe { arena_mut(a, "arena_try_extend") };
    if ptr != arena.last_alloc_start || arena.last_alloc_size != old_size {
        return 0;
    }
    if new_size < old_size {
        return 0;
    }
    let delta = new_size - old_size;
    if arena.cursor.saturating_add(delta) > arena.chunk_end {
        return 0;
    }
    arena.cursor += delta;
    arena.last_alloc_size = new_size;
    1
}

// Walk the arena's chunk chain for the chunk that contains `ptr`. If that
// chunk was allocated via the oversized path (`chunk_is_oversized()`) and
// is not the current (tail) chunk, unlink it from the chain and libc::free
// it; decrement the arena's retained bytes and the global memory counters.
//
// Used by `arena_grow_backing` after a growth that allocated into a new
// chunk: the prior backing is now unreachable, and if it was the sole
// allocation in its (oversized) chunk we can return that memory to libc
// immediately rather than waiting for arena close. This fixes issue #391
// (long-lived Vec/String/HashMap grow-then-truncate accumulating committed
// pages). Normal-chunk backings cannot be freed early because the chunk
// is shared with other allocations.
//
// Cost: O(chunks-before-match) chunk-chain scan from `first_chunk`. The
// upper bound is the number of *retained* normal chunks plus any unfreed
// oversized chunks ahead of the match; prior oversized chunks released on
// earlier growths are no longer in the chain, so the typical grow-then-
// truncate loop that motivated this fix stays effectively O(1) per growth
// — the cost is dominated by normal-chunk count, not by growth count.
unsafe fn arena_try_free_oversized_chunk(a: *mut VowArena, ptr: *const u8) -> bool {
    if ptr.is_null() {
        return false;
    }
    let arena = unsafe { arena_mut(a, "arena_free_oversized_chunk") };
    // `Option<NonNull<_>>` rather than a null-as-absent raw pointer: the
    // "no predecessor yet" case is the head of the chain, and making that a
    // distinct variant means the relink below cannot mistake it for a
    // writable link word.
    let mut prev: Option<NonNull<u8>> = None;
    let mut chunk = arena.first_chunk;
    while !chunk.is_null() {
        let total = unsafe { chunk_total(chunk) };
        let chunk_base = chunk as usize;
        let payload_start = chunk_base + CHUNK_LINK_BYTES;
        let chunk_limit = chunk_base + total;
        if (ptr as usize) >= payload_start && (ptr as usize) < chunk_limit {
            // Normal chunks may carry other live allocations; refuse to free.
            // The path-oversized predicate (recorded at allocation time) is
            // the correct test — not `total > normal_chunk_total()`, which
            // would miss path-oversized chunks whose total is ≤ 4112 (e.g.
            // a 3000-byte single-resident string backing with total 3016).
            if !unsafe { chunk_is_oversized(chunk) } {
                return false;
            }
            // The grow path appends a new chunk before reaching here, so the
            // abandoned chunk is always non-tail. Refuse if that invariant
            // breaks rather than corrupt `cursor`/`chunk_end`.
            if chunk == arena.current_chunk {
                return false;
            }
            let next = unsafe { next_chunk(chunk) };
            match prev {
                None => arena.first_chunk = next,
                Some(prev) => unsafe { set_next_chunk(prev.as_ptr(), next) },
            }
            // Saturating by design: a violated `retained_bytes >= total`
            // invariant must not panic in production. The C ESBMC mirror in
            // `vow-runtime/verify/arena.c` uses plain unsigned subtraction
            // at the same site so any such underflow stays verifier-visible.
            arena.retained_bytes = arena.retained_bytes.saturating_sub(total);
            memory_note_arena_release(a, total);
            unsafe { libc::free(chunk as *mut libc::c_void) };
            return true;
        }
        prev = NonNull::new(chunk);
        chunk = unsafe { next_chunk(chunk) };
    }
    false
}

// Root region header lives in .bss. Initialized by __vow_runtime_start before
// main; never reclaimed (spec §6.2). Not yet wired to main (Phase 4).
#[unsafe(no_mangle)]
pub static mut __vow_root_arena: VowArena = VowArena {
    first_chunk: core::ptr::null_mut(),
    current_chunk: core::ptr::null_mut(),
    cursor: 0,
    chunk_end: 0,
    last_alloc_start: core::ptr::null_mut(),
    last_alloc_size: 0,
    retained_bytes: 0,
};

static ROOT_ARENA_INITIALIZED: AtomicBool = AtomicBool::new(false);
static ROOT_ARENA_LOCK: Mutex<()> = Mutex::new(());

unsafe fn ensure_root_arena() {
    unsafe { with_root_arena(|_| ()) }
}

unsafe fn ensure_root_arena_locked() {
    if !ROOT_ARENA_INITIALIZED.load(Ordering::SeqCst) {
        unsafe { __vow_arena_open(&raw mut __vow_root_arena) };
        ROOT_ARENA_INITIALIZED.store(true, Ordering::SeqCst);
    }
}

thread_local! {
    static ROOT_LOCK_HELD: Cell<bool> = const { Cell::new(false) };
}

struct RootLockHeld;

impl RootLockHeld {
    fn set() -> Self {
        ROOT_LOCK_HELD.with(|held| held.set(true));
        RootLockHeld
    }
}

impl Drop for RootLockHeld {
    fn drop(&mut self) {
        ROOT_LOCK_HELD.with(|held| held.set(false));
    }
}

/// Run `f` against the process-wide root arena with `ROOT_ARENA_LOCK` held for
/// the whole call, so a root wrapper that allocates several objects locks once.
unsafe fn with_root_arena<R>(f: impl FnOnce(*mut VowArena) -> R) -> R {
    let _guard = ROOT_ARENA_LOCK.lock().unwrap();
    let _held = RootLockHeld::set();
    unsafe { ensure_root_arena_locked() };
    f(&raw mut __vow_root_arena)
}

/// Grow a map backing buffer in the arena recorded in the map header, not the
/// arena the entry point was handed: a map reached through a struct field or a
/// `Vec` element has no provable region. A root-owned map takes the root lock
/// unless this thread already holds it; all `ptrs` grow under one acquisition.
unsafe fn arena_grow_map_buffers<const N: usize>(
    owner: *mut VowArena,
    ptrs: [*mut u8; N],
    old_size: usize,
    new_size: usize,
) -> [*mut u8; N] {
    if owner.is_null() {
        null_arena_trap("map growth");
    }
    let grow = |arena| ptrs.map(|p| unsafe { arena_grow_backing(arena, p, old_size, new_size, 8) });
    if !arena_is_root(owner) || ROOT_LOCK_HELD.with(Cell::get) {
        return grow(owner);
    }
    unsafe { with_root_arena(grow) }
}

/// Grow a backing buffer that lives in `arena`. Implements the spec §7.2
/// zero-copy fast path: try `__vow_arena_try_extend` first; if the backing
/// is the most recent allocation in the chunk and the new size still fits,
/// growth is O(1) with no copy and no orphaned backing. Otherwise fall back
/// to a fresh allocation + memcpy of the prefix.
///
/// When fallback runs and the old backing was the sole allocation in an
/// oversized chunk (>2048 bytes — see `__vow_arena_alloc`), the abandoned
/// chunk is returned to libc rather than retained until arena close. This
/// is the fix for issue #391: long-running programs that grow-then-truncate
/// a Vec/String/HashMap in a hot loop no longer accumulate committed pages
/// from prior reallocation peaks.
unsafe fn arena_grow_backing(
    arena: *mut VowArena,
    ptr: *mut u8,
    old_size: usize,
    new_size: usize,
    align: usize,
) -> *mut u8 {
    if old_size > 0 && unsafe { __vow_arena_try_extend(arena, ptr, old_size, new_size) != 0 } {
        unsafe { std::ptr::write_bytes(ptr.add(old_size), 0, new_size - old_size) };
        return ptr;
    }

    let new_ptr = unsafe { __vow_arena_alloc(arena, new_size, align) };
    if old_size > 0 {
        unsafe { std::ptr::copy_nonoverlapping(ptr, new_ptr, old_size) };
        // Old backing is unreachable from here on. Release its chunk if
        // it was the sole resident of an oversized chunk.
        //
        // Cheap fast-skip: if this predicate is false, the backing was
        // necessarily placed in a normal chunk by __vow_arena_alloc (the
        // oversized new-chunk path requires it to be true). Calling
        // arena_try_free_oversized_chunk when false would only walk the
        // chain to find chunk_is_oversized == false and bail, so we skip
        // the O(N) walk. When the predicate is true the backing *may* be
        // in an oversized chunk (it could also have been placed via the
        // fast path into a shared normal chunk if room was available);
        // the chain walker's `chunk_is_oversized` check is the
        // authoritative answer.
        if old_size > OVERSIZED_THRESHOLD || old_size + (align - 1) > CHUNK_PAYLOAD {
            unsafe { arena_try_free_oversized_chunk(arena, ptr) };
        }
    }
    unsafe { std::ptr::write_bytes(new_ptr.add(old_size), 0, new_size - old_size) };
    new_ptr
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn __vow_runtime_start() {
    unsafe { ensure_root_arena() };
}

#[repr(C)]
pub struct VowVec {
    pub ptr: *mut u8,
    pub len: usize,
    pub cap: usize,
}

/// Runtime allocation wrapper for mutable `VowVec` descriptors. The pointer
/// exposed across the Vow ABI addresses `desc`, so its public layout remains
/// the documented 24-byte `{ptr, len, cap}`. The high bit of `cap` marks the
/// descriptor as runtime-owned; the hidden prefix then records its arena for
/// O(1), fail-closed projection routing.
///
/// Rodata descriptors are emitted as bare `VowVec` values and never use this
/// wrapper; checked routing observes both `VOW_CAP_RODATA` and the runtime-owned
/// marker before reading the prefix.
#[repr(C)]
struct OwnedVowVec {
    owner: *mut VowArena,
    desc: VowVec,
}

const _: () = assert!(core::mem::size_of::<VowVec>() == 24);
const _: () = assert!(core::mem::offset_of!(OwnedVowVec, desc) == 8);

fn vow_vec_capacity(desc: &VowVec) -> usize {
    desc.cap & VOW_CAP_VALUE_MASK
}

fn set_vow_vec_capacity(desc: &mut VowVec, capacity: usize, op: &'static str) {
    // `VOW_CAP_VALUE_MASK` would encode as the rodata sentinel when combined
    // with the ownership bit. Reserve that otherwise-theoretical capacity and
    // report the same structured OOM used by the reserve path rather than
    // creating an ambiguous descriptor.
    if capacity >= VOW_CAP_VALUE_MASK {
        oom_trap(op);
    }
    desc.cap = capacity | (desc.cap & VOW_CAP_RUNTIME_OWNED);
}

/// The arena recorded in a mutable runtime descriptor allocated by
/// `__vow_vec_new_in_arena`, or `None` for rodata and foreign descriptors.
///
/// Safety: the in-descriptor ownership bit is checked before reading outside
/// the public descriptor. Only runtime-owned descriptors carry that bit and
/// have an `OwnedVowVec` prefix; valid foreign descriptors leave it clear.
unsafe fn vow_vec_owner(vec: *const u8) -> Option<*mut VowArena> {
    let desc = unsafe { &*(vec as *const VowVec) };
    if desc.cap == VOW_CAP_RODATA || desc.cap & VOW_CAP_RUNTIME_OWNED == 0 {
        return None;
    }
    let owner_ptr =
        unsafe { vec.sub(core::mem::size_of::<*mut VowArena>()) } as *const *mut VowArena;
    Some(unsafe { *owner_ptr })
}

/// Return true iff `vec` is a mutable runtime descriptor whose hidden owner
/// is `candidate`.
unsafe fn vow_vec_is_owned_by(vec: *const u8, candidate: *mut VowArena) -> bool {
    unsafe { vow_vec_owner(vec) }
        .is_some_and(|owner| std::ptr::eq(owner as *const VowArena, candidate as *const VowArena))
}

/// Run `f` with the arena a growing Vec or String must reallocate in: the arena
/// recorded in its descriptor, so a receiver reached through a struct field or
/// a `Vec` element (no provable region) keeps its buffer next to its header
/// instead of leaking it into the root arena. Rodata and foreign descriptors
/// fall back to the root arena. A root-owned receiver takes the root lock
/// unless this thread already holds it.
unsafe fn with_growth_arena<R>(recv: *const u8, f: impl FnOnce(*mut VowArena) -> R) -> R {
    if let Some(owner) = unsafe { vow_vec_owner(recv) }
        && !owner.is_null()
        && (!arena_is_root(owner) || ROOT_LOCK_HELD.with(Cell::get))
    {
        return f(owner);
    }
    unsafe { with_root_arena(f) }
}

unsafe fn alloc_owned_vow_vec_descriptor(arena: *mut VowArena) -> *mut VowVec {
    let owned_ptr = unsafe {
        __vow_arena_alloc(
            arena,
            core::mem::size_of::<OwnedVowVec>(),
            core::mem::align_of::<OwnedVowVec>(),
        )
    } as *mut OwnedVowVec;
    unsafe {
        (*owned_ptr).owner = arena;
        (*owned_ptr).desc = VowVec {
            ptr: std::ptr::dangling_mut::<u8>(),
            len: 0,
            cap: VOW_CAP_RUNTIME_OWNED,
        };
    }
    unsafe { core::ptr::addr_of_mut!((*owned_ptr).desc) }
}

struct StdinLineScratch {
    desc: VowVec,
    bytes: Vec<u8>,
}

impl StdinLineScratch {
    fn new() -> Self {
        Self {
            desc: VowVec {
                ptr: std::ptr::dangling_mut::<u8>(),
                len: 0,
                cap: VOW_CAP_RODATA,
            },
            bytes: Vec::new(),
        }
    }
}

thread_local! {
    // Each OS thread owns one stable descriptor. The returned pointer remains
    // valid until that same thread's next stdin_read_line call, and concurrent
    // callers never share descriptor or backing-buffer state.
    static STDIN_LINE_SCRATCH: RefCell<StdinLineScratch> =
        RefCell::new(StdinLineScratch::new());
}

fn read_stdin_line_into_scratch<R: std::io::BufRead>(
    reader: &mut R,
    scratch: &mut StdinLineScratch,
) -> *mut u8 {
    // clear preserves capacity: scratch memory follows the largest line seen,
    // not total input, and may retain that high-water mark for process lifetime.
    scratch.bytes.clear();
    // Vow strings are byte strings: accept arbitrary stdin bytes, including
    // invalid UTF-8, while still splitting on newline bytes.
    let bytes_read = match reader.read_until(b'\n', &mut scratch.bytes) {
        Ok(n) => n,
        Err(_) => {
            // Preserve the historical stdin_read_line contract: IO errors look like EOF.
            scratch.bytes.clear();
            0
        }
    };
    // bytes may reallocate while reading a longer line. Vow callers hold this
    // stable descriptor address, so refresh ptr/len after each read; unpinned
    // old values then observe the current scratch line instead of freed memory.
    if bytes_read == 0 {
        scratch.desc.ptr = std::ptr::dangling_mut::<u8>();
        scratch.desc.len = 0;
    } else {
        scratch.desc.ptr = scratch.bytes.as_mut_ptr();
        scratch.desc.len = scratch.bytes.len();
    }
    scratch.desc.cap = VOW_CAP_RODATA;
    // SAFETY: this raw pointer escapes the RefCell borrow in
    // __vow_stdin_read_line, but it targets this thread's stable thread-local
    // descriptor. The descriptor remains live for the thread lifetime; its
    // contents are semantically invalidated by the next call on this thread.
    &mut scratch.desc as *mut VowVec as *mut u8
}

const VEC_INITIAL_CAP: usize = 8;

#[unsafe(no_mangle)]
pub unsafe extern "C" fn __vow_vec_new_in_arena(
    arena: *mut VowArena,
    elem_size: usize,
    align: usize,
) -> *mut u8 {
    let _ = elem_size;
    if arena.is_null() {
        null_arena_trap("Vec::new");
    }
    let header_ptr = unsafe { alloc_owned_vow_vec_descriptor(arena) };
    // Lazy allocation: don't allocate buffer until first push.
    // Use a dangling aligned pointer so from_raw_parts with len=0 is safe.
    unsafe {
        (*header_ptr).ptr = align as *mut u8;
        (*header_ptr).len = 0;
        set_vow_vec_capacity(&mut *header_ptr, 0, "Vec::new");
    }
    sanitize_on_vec_new(header_ptr as usize);
    header_ptr as *mut u8
}

#[unsafe(no_mangle)]
pub extern "C" fn __vow_vec_new(elem_size: usize, align: usize) -> *mut u8 {
    unsafe { with_root_arena(|root| __vow_vec_new_in_arena(root, elem_size, align)) }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn __vow_vec_new_val_in_arena(arena: *mut VowArena) -> *mut u8 {
    unsafe { __vow_vec_new_in_arena(arena, 8, 8) }
}

#[unsafe(no_mangle)]
pub extern "C" fn __vow_vec_new_val() -> *mut u8 {
    unsafe { with_root_arena(|root| __vow_vec_new_val_in_arena(root)) }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn __vow_vec_from_raw_parts_copy_val(
    arena: *mut VowArena,
    ptr: *const i64,
    len: usize,
) -> *mut u8 {
    if arena.is_null() {
        null_arena_trap("Vec::from_raw_parts_copy");
    }
    let vec = unsafe { __vow_vec_new_val_in_arena(arena) };
    if len == 0 || ptr.is_null() {
        return vec;
    }
    let bytes = len
        .checked_mul(8)
        .unwrap_or_else(|| oom_trap("Vec::from_raw_parts_copy"));
    let v = unsafe { &mut *(vec as *mut VowVec) };
    v.ptr = unsafe { __vow_arena_alloc(arena, bytes, 8) };
    unsafe { std::ptr::copy_nonoverlapping(ptr as *const u8, v.ptr, bytes) };
    v.len = len;
    set_vow_vec_capacity(v, len, "Vec::from_raw_parts_copy");
    vec
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn __vow_vec_pin_to_root_val(source: *const u8) -> *mut u8 {
    if source.is_null() {
        return __vow_vec_new_val();
    }
    sanitize_on_read(source as usize, 0);
    let src = unsafe { &*(source as *const VowVec) };
    unsafe {
        with_root_arena(|root| {
            __vow_vec_from_raw_parts_copy_val(root, src.ptr as *const i64, src.len)
        })
    }
}

unsafe fn vec_reserve_in_arena_no_null_check(
    arena: *mut VowArena,
    vec: *mut u8,
    additional: usize,
    elem_size: usize,
    elem_align: usize,
) {
    let v = unsafe { &mut *(vec as *mut VowVec) };
    if v.cap == VOW_CAP_RODATA {
        region_literal_mutation_trap("Vec::reserve");
    }
    let old_cap = vow_vec_capacity(v);
    // Checked growth arithmetic (issue #435): an oversized `additional` or
    // `elem_size` must trap through the OutOfMemory envelope before the
    // descriptor is touched. Unchecked, `v.len + additional` and the
    // capacity/byte-size products could wrap (under-allocating backing that
    // later writes would overrun), and doubling `new_cap` past usize::MAX
    // wraps it to 0 so the `< required` loop never terminates.
    let required = match v.len.checked_add(additional) {
        Some(r) => r,
        None => oom_trap("Vec::reserve"),
    };
    if required <= old_cap {
        return;
    }
    let mut new_cap = if old_cap == 0 {
        VEC_INITIAL_CAP
    } else {
        old_cap
    };
    while new_cap < required {
        new_cap = match new_cap.checked_mul(2) {
            Some(c) if c < VOW_CAP_VALUE_MASK => c,
            _ => oom_trap("Vec::reserve"),
        };
    }
    let old_size = match old_cap.checked_mul(elem_size) {
        Some(s) => s,
        None => oom_trap("Vec::reserve"),
    };
    let new_size = match new_cap.checked_mul(elem_size) {
        Some(s) => s,
        None => oom_trap("Vec::reserve"),
    };
    let new_ptr = unsafe { arena_grow_backing(arena, v.ptr, old_size, new_size, elem_align) };
    v.ptr = new_ptr;
    set_vow_vec_capacity(v, new_cap, "Vec::reserve");
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn __vow_vec_reserve_in_arena(
    arena: *mut VowArena,
    vec: *mut u8,
    additional: usize,
    elem_size: usize,
    elem_align: usize,
) {
    if arena.is_null() {
        null_arena_trap("Vec::reserve");
    }
    unsafe { vec_reserve_in_arena_no_null_check(arena, vec, additional, elem_size, elem_align) };
}

unsafe fn __vow_vec_reserve(vec: *mut u8, additional: usize, elem_size: usize, elem_align: usize) {
    unsafe {
        with_growth_arena(vec, |arena| {
            vec_reserve_in_arena_no_null_check(arena, vec, additional, elem_size, elem_align)
        })
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn __vow_vec_push_in_arena(
    arena: *mut VowArena,
    vec: *mut u8,
    elem: *const u8,
    elem_size: usize,
    elem_align: usize,
) {
    if arena.is_null() {
        null_arena_trap("Vec::push");
    }
    // Sanitizer first — consults the shadow table by pointer value and
    // diagnoses UseAfterFree without dereferencing. The cap check must
    // dereference, so it has to run after the sanitizer.
    sanitize_on_push(vec as usize);
    unsafe { vec_push_no_sanitize_in_arena(arena, vec, elem, elem_size, elem_align, "Vec::push") };
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn __vow_vec_push(
    vec: *mut u8,
    elem: *const u8,
    elem_size: usize,
    elem_align: usize,
) {
    unsafe {
        with_growth_arena(vec, |arena| {
            __vow_vec_push_in_arena(arena, vec, elem, elem_size, elem_align)
        })
    }
}

unsafe fn vec_push_no_sanitize_in_arena(
    arena: *mut VowArena,
    vec: *mut u8,
    elem: *const u8,
    elem_size: usize,
    elem_align: usize,
    op: &'static str,
) {
    let v = unsafe { &*(vec as *const VowVec) };
    if v.cap == VOW_CAP_RODATA {
        region_literal_mutation_trap(op);
    }
    unsafe { vec_reserve_in_arena_no_null_check(arena, vec, 1, elem_size, elem_align) };
    let v = unsafe { &mut *(vec as *mut VowVec) };
    let dest = unsafe { v.ptr.add(v.len * elem_size) };
    unsafe { std::ptr::copy_nonoverlapping(elem, dest, elem_size) };
    v.len += 1;
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn __vow_vec_len(vec: *const u8) -> usize {
    sanitize_on_read(vec as usize, 0);
    let v = unsafe { &*(vec as *const VowVec) };
    v.len
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn __vow_vec_push_val_in_arena(
    arena: *mut VowArena,
    vec: *mut u8,
    value: i64,
) {
    if arena.is_null() {
        null_arena_trap("Vec::push_val");
    }
    // Sanitize + cap-check here with the precise operation name. Delegating
    // the whole path to __vow_vec_push would (a) double-sanitize and (b)
    // report the trap as "Vec::push" instead of "Vec::push_val". Delegate
    // the actual push to the no-sanitize helper so the shadow table records
    // a single generation per appended element.
    sanitize_on_push(vec as usize);
    let bytes = value.to_ne_bytes();
    unsafe { vec_push_no_sanitize_in_arena(arena, vec, bytes.as_ptr(), 8, 8, "Vec::push_val") };
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn __vow_vec_push_val(vec: *mut u8, value: i64) {
    unsafe { with_growth_arena(vec, |arena| __vow_vec_push_val_in_arena(arena, vec, value)) }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn __vow_vec_get_val(vec: *const u8, index: usize) -> i64 {
    let ptr = unsafe { __vow_vec_get_ptr(vec, index, 8) };
    unsafe { *(ptr as *const i64) }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn __vow_vec_pop(vec: *mut u8) {
    sanitize_on_pop(vec as usize);
    let v = unsafe { &mut *(vec as *mut VowVec) };
    if v.cap == VOW_CAP_RODATA {
        region_literal_mutation_trap("Vec::pop");
    }
    if v.len > 0 {
        v.len -= 1;
    }
}

/// Resets the Vec to an empty state. Arena-backed buffers are retained until
/// the region closes; the header remains valid and can be reused with push().
#[unsafe(no_mangle)]
pub unsafe extern "C" fn __vow_vec_clear(vec: *mut u8) {
    sanitize_on_clear(vec as usize);
    let v = unsafe { &mut *(vec as *mut VowVec) };
    if v.cap == VOW_CAP_RODATA {
        region_literal_mutation_trap("Vec::clear");
    }
    v.len = 0;
}

/// Truncates the Vec to `new_len` elements. Arena-backed buffers are not
/// shrunk; their storage is reclaimed when the containing region closes.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn __vow_vec_truncate(vec: *mut u8, new_len: usize) {
    sanitize_on_truncate(vec as usize, new_len);
    let v = unsafe { &mut *(vec as *mut VowVec) };
    if v.cap == VOW_CAP_RODATA {
        region_literal_mutation_trap("Vec::truncate");
    }
    if new_len >= v.len {
        return;
    }
    v.len = new_len;
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn __vow_vec_set_val(vec: *mut u8, index: usize, value: i64) {
    sanitize_on_set(vec as usize, index);
    let v = unsafe { &*(vec as *const VowVec) };
    if v.cap == VOW_CAP_RODATA {
        region_literal_mutation_trap("Vec::set");
    }
    if index >= v.len {
        let json = r#"{"error":"IndexOutOfBounds"}"#;
        let _ = writeln!(std::io::stderr(), "{json}");
        let _ = writeln!(std::io::stderr(), "index out of bounds");
        std::process::exit(VOW_RUNTIME_ABORT_EXIT);
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
    sanitize_on_read(vec as usize, index);
    let v = unsafe { &*(vec as *const VowVec) };
    if index >= v.len {
        let json = r#"{"error":"IndexOutOfBounds"}"#;
        let _ = writeln!(std::io::stderr(), "{json}");
        let _ = writeln!(std::io::stderr(), "index out of bounds");
        std::process::exit(VOW_RUNTIME_ABORT_EXIT);
    }
    unsafe { v.ptr.add(index * elem_size) as *const u8 }
}

// ---------------------------------------------------------------------------
// String (VowVec<u8>) runtime
// ---------------------------------------------------------------------------

#[unsafe(no_mangle)]
pub unsafe extern "C" fn __vow_string_new_in_arena(
    arena: *mut VowArena,
    ptr: *const c_char,
    len: usize,
) -> *mut u8 {
    if arena.is_null() {
        null_arena_trap("String::new");
    }
    let vec_ptr = unsafe { __vow_vec_new_in_arena(arena, 1, 1) };
    if len > 0 && !ptr.is_null() {
        unsafe { __vow_vec_reserve_in_arena(arena, vec_ptr, len, 1, 1) };
        let v = unsafe { &mut *(vec_ptr as *mut VowVec) };
        unsafe { std::ptr::copy_nonoverlapping(ptr as *const u8, v.ptr, len) };
        v.len = len;
    }
    vec_ptr
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn __vow_string_new(ptr: *const c_char, len: usize) -> *mut u8 {
    unsafe { with_root_arena(|root| __vow_string_new_in_arena(root, ptr, len)) }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn __vow_string_from_cstr_in_arena(
    arena: *mut VowArena,
    ptr: *const c_char,
) -> *mut u8 {
    if arena.is_null() {
        null_arena_trap("String::from_cstr");
    }
    if ptr.is_null() {
        return unsafe { __vow_string_new_in_arena(arena, std::ptr::null(), 0) };
    }
    let s = unsafe { CStr::from_ptr(ptr) };
    let bytes = s.to_bytes();
    unsafe { __vow_string_new_in_arena(arena, ptr, bytes.len()) }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn __vow_string_from_cstr(ptr: *const c_char) -> *mut u8 {
    unsafe { with_root_arena(|root| __vow_string_from_cstr_in_arena(root, ptr)) }
}

/// Deep-copy `source` (a `VowString` / `Vec<u8>` descriptor) into `arena`,
/// returning a freshly-allocated descriptor whose backing also lives in
/// `arena`. Used by Phase 4 / S5 return materialization (spec §5.1) to
/// satisfy the `FreshInCaller` representation promise when the source path
/// is a `.rodata` literal or a parameter alias whose backing is not in
/// `target_region`.
///
/// The new descriptor has logical capacity `len`; growth is up to the caller.
/// The source's `cap` is irrelevant — `VOW_CAP_RODATA` (read-only literal) is
/// handled transparently because we only read `source.ptr` / `source.len`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn __vow_string_clone_into_arena(
    arena: *mut VowArena,
    source: *const u8,
) -> *mut u8 {
    // A null `source` here is anomalous: well-formed compilation never
    // produces it. The only path that does is the codegen `ConstStr`
    // fallback to `iconst(0)` when a string global is missing — which
    // is itself an upstream compiler error. Surface it loudly in
    // debug builds; release falls through to a benign empty descriptor
    // (allocated on the arena) so a buggy build doesn't crash.
    debug_assert!(
        !source.is_null(),
        "__vow_string_clone_into_arena: null source — indicates a missing \
         ConstStr global (upstream codegen bug)"
    );
    let header = unsafe { alloc_owned_vow_vec_descriptor(arena) };
    if source.is_null() {
        unsafe {
            (*header).ptr = std::ptr::dangling_mut::<u8>(); // len=0
            (*header).len = 0;
            set_vow_vec_capacity(&mut *header, 0, "String::clone");
        }
        return header as *mut u8;
    }
    let src = unsafe { &*(source as *const VowVec) };
    let len = src.len;
    let data_ptr = if len == 0 {
        std::ptr::dangling_mut::<u8>() // len=0 — same convention as __vow_vec_new
    } else {
        let p = unsafe { __vow_arena_alloc(arena, len, 1) };
        unsafe { std::ptr::copy_nonoverlapping(src.ptr, p, len) };
        p
    };
    unsafe {
        (*header).ptr = data_ptr;
        (*header).len = len;
        set_vow_vec_capacity(&mut *header, len, "String::clone");
    }
    header as *mut u8
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn __vow_string_clone(source: *const u8) -> *mut u8 {
    unsafe { with_root_arena(|root| __vow_string_clone_into_arena(root, source)) }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn __vow_string_clone_in_arena(
    arena: *mut VowArena,
    source: *const u8,
) -> *mut u8 {
    // ABI wrapper: preserve the explicit-arena null guard before delegating.
    if arena.is_null() {
        null_arena_trap("String::clone");
    }
    unsafe { __vow_string_clone_into_arena(arena, source) }
}

// Kept distinct from `__vow_string_clone`: pin_to_root means "extend lifetime
// to root", not just "produce a mutable copy", even though both copy today.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn __vow_string_pin_to_root(source: *const u8) -> *mut u8 {
    unsafe { with_root_arena(|root| __vow_string_clone_into_arena(root, source)) }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn __vow_string_from_raw_parts_copy(
    arena: *mut VowArena,
    ptr: *const u8,
    len: usize,
) -> *mut u8 {
    if arena.is_null() {
        null_arena_trap("String::from_raw_parts_copy");
    }
    let header = unsafe { alloc_owned_vow_vec_descriptor(arena) };
    if len == 0 || ptr.is_null() {
        unsafe {
            (*header).ptr = std::ptr::dangling_mut::<u8>();
            (*header).len = 0;
            set_vow_vec_capacity(&mut *header, 0, "String::from_raw_parts_copy");
        }
        return header as *mut u8;
    }
    let data_ptr = unsafe { __vow_arena_alloc(arena, len, 1) };
    unsafe { std::ptr::copy_nonoverlapping(ptr, data_ptr, len) };
    unsafe {
        (*header).ptr = data_ptr;
        (*header).len = len;
        set_vow_vec_capacity(&mut *header, len, "String::from_raw_parts_copy");
    }
    header as *mut u8
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn __vow_string_len(s: *const u8) -> usize {
    unsafe { __vow_vec_len(s) }
}

/// Resets the String to empty. Arena-backed storage is retained until the
/// region closes; the header remains valid and can be reused.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn __vow_string_clear(s: *mut u8) {
    sanitize_on_read(s as usize, 0);
    let v = unsafe { &mut *(s as *mut VowVec) };
    if v.cap == VOW_CAP_RODATA {
        region_literal_mutation_trap("String::clear");
    }
    v.len = 0;
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn __vow_string_eq(a: *const u8, b: *const u8) -> i64 {
    sanitize_on_read(a as usize, 0);
    sanitize_on_read(b as usize, 0);
    let va = unsafe { &*(a as *const VowVec) };
    let vb = unsafe { &*(b as *const VowVec) };
    if va.len != vb.len {
        return 0;
    }
    let sa = unsafe { std::slice::from_raw_parts(va.ptr, va.len) };
    let sb = unsafe { std::slice::from_raw_parts(vb.ptr, vb.len) };
    if sa == sb { 1 } else { 0 }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn __vow_string_contains(haystack: *const u8, needle: *const u8) -> i64 {
    sanitize_on_read(haystack as usize, 0);
    sanitize_on_read(needle as usize, 0);
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
pub unsafe extern "C" fn __vow_string_matches_literal_at(
    s: *const u8,
    pos: u64,
    literal_ptr: *const u8,
    literal_len: u64,
) -> i64 {
    if s.is_null() || literal_ptr.is_null() {
        return 0;
    }
    sanitize_on_read(s as usize, 0);
    let v = unsafe { &*(s as *const VowVec) };
    let Ok(pos) = usize::try_from(pos) else {
        return 0;
    };
    let Ok(literal_len) = usize::try_from(literal_len) else {
        return 0;
    };
    let Some(end) = pos.checked_add(literal_len) else {
        return 0;
    };
    if end > v.len {
        return 0;
    }
    let bytes = unsafe { std::slice::from_raw_parts(v.ptr, v.len) };
    let literal = unsafe { std::slice::from_raw_parts(literal_ptr, literal_len) };
    if bytes[pos..end] == *literal { 1 } else { 0 }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn __vow_string_push_str_in_arena(
    arena: *mut VowArena,
    dest: *mut u8,
    src: *const u8,
) {
    if arena.is_null() {
        null_arena_trap("String::push_str");
    }
    sanitize_on_read(dest as usize, 0);
    sanitize_on_read(src as usize, 0);
    unsafe { string_push_str_in_arena_no_sanitize(arena, dest, src) };
}

unsafe fn string_push_str_in_arena_no_sanitize(
    arena: *mut VowArena,
    dest: *mut u8,
    src: *const u8,
) {
    let vd0 = unsafe { &*(dest as *const VowVec) };
    if vd0.cap == VOW_CAP_RODATA {
        region_literal_mutation_trap("String::push_str");
    }
    // Snapshot source length and detect self-append BEFORE the reserve. The
    // reserve may grow `dest` into a new chunk and `arena_grow_backing` may
    // libc::free the abandoned chunk (PR #392 fix for issue #391). If `src`
    // aliases `dest`, a captured `&*src` reference would dangle on the
    // post-reserve read. Use the post-reserve `dest` descriptor's `ptr` as
    // the read source in the self-append case — `arena_grow_backing` copies
    // the old contents into the new backing before freeing, so the source
    // bytes are present at the new pointer.
    let src_is_dest = std::ptr::eq(src as *const VowVec, dest as *const VowVec);
    let src_len = unsafe { (*(src as *const VowVec)).len };
    if src_len == 0 {
        return;
    }
    let src_ptr_before_reserve = if src_is_dest {
        core::ptr::null()
    } else {
        unsafe { (*(src as *const VowVec)).ptr as *const u8 }
    };
    unsafe { __vow_vec_reserve_in_arena(arena, dest, src_len, 1, 1) };
    let vd = unsafe { &mut *(dest as *mut VowVec) };
    let src_ptr = if src_is_dest {
        // Self-append: the reserve copied the original bytes into `vd.ptr`;
        // the captured pointer (if any) into the old backing may now point
        // at freed memory.
        vd.ptr as *const u8
    } else {
        src_ptr_before_reserve
    };
    unsafe { std::ptr::copy_nonoverlapping(src_ptr, vd.ptr.add(vd.len), src_len) };
    vd.len += src_len;
}

/// Projection-safe string append. `candidate` is obtained from the projected
/// container's inferred Block/Caller region, but is trusted only when the
/// receiver descriptor's runtime owner matches it. Foreign projections fall
/// back to the root arena, preserving PR #344's lifetime safety.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn __vow_string_push_str_in_candidate_arena(
    candidate: *mut VowArena,
    dest: *mut u8,
    src: *const u8,
) {
    if candidate.is_null() {
        null_arena_trap("String::push_str");
    }
    sanitize_on_read(dest as usize, 0);
    sanitize_on_read(src as usize, 0);
    if !arena_is_root(candidate) && unsafe { vow_vec_is_owned_by(dest, candidate) } {
        unsafe { string_push_str_in_arena_no_sanitize(candidate, dest, src) };
        return;
    }
    unsafe { with_root_arena(|root| string_push_str_in_arena_no_sanitize(root, dest, src)) }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn __vow_string_push_str(dest: *mut u8, src: *const u8) {
    unsafe {
        with_growth_arena(dest, |arena| {
            __vow_string_push_str_in_arena(arena, dest, src)
        })
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn __vow_string_from_i64_in_arena(arena: *mut VowArena, v: i64) -> *mut u8 {
    if arena.is_null() {
        null_arena_trap("String::from_i64");
    }
    let s = v.to_string();
    unsafe { __vow_string_new_in_arena(arena, s.as_ptr() as *const c_char, s.len()) }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn __vow_string_from_i64(v: i64) -> *mut u8 {
    unsafe { with_root_arena(|root| __vow_string_from_i64_in_arena(root, v)) }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn __vow_string_from_u64_in_arena(arena: *mut VowArena, v: u64) -> *mut u8 {
    if arena.is_null() {
        null_arena_trap("String::from_u64");
    }
    let s = v.to_string();
    unsafe { __vow_string_new_in_arena(arena, s.as_ptr() as *const c_char, s.len()) }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn __vow_string_from_u64(v: u64) -> *mut u8 {
    unsafe { with_root_arena(|root| __vow_string_from_u64_in_arena(root, v)) }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn __vow_string_print(s: *const u8) {
    sanitize_on_read(s as usize, 0);
    let v = unsafe { &*(s as *const VowVec) };
    let bytes = unsafe { std::slice::from_raw_parts(v.ptr, v.len) };
    let _ = std::io::stdout().write_all(bytes);
    let _ = std::io::stdout().flush();
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn __vow_string_byte_at(s: *const u8, idx: u64) -> i64 {
    sanitize_on_read(s as usize, 0);
    let v = unsafe { &*(s as *const VowVec) };
    if idx >= v.len as u64 {
        return -1;
    }
    let bytes = unsafe { std::slice::from_raw_parts(v.ptr, v.len) };
    bytes[idx as usize] as i64
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn __vow_string_push_byte_in_arena(
    arena: *mut VowArena,
    s: *mut u8,
    byte: u64,
) {
    if arena.is_null() {
        null_arena_trap("String::push_byte");
    }
    // Sanitize once here, then delegate to the no-sanitize inner helper with
    // a type-specific operation name. This keeps both orderings correct:
    // sanitizer runs before any dereference (UAF detected first), and the
    // shadow table records a single generation for the one appended byte.
    sanitize_on_push(s as usize);
    unsafe { string_push_byte_in_arena_no_sanitize(arena, s, byte as u8) };
}

unsafe fn string_push_byte_in_arena_no_sanitize(arena: *mut VowArena, s: *mut u8, byte: u8) {
    unsafe {
        vec_push_no_sanitize_in_arena(arena, s, &byte as *const u8, 1, 1, "String::push_byte")
    };
}

/// Projection-safe byte append; see
/// `__vow_string_push_str_in_candidate_arena` for the routing invariant.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn __vow_string_push_byte_in_candidate_arena(
    candidate: *mut VowArena,
    s: *mut u8,
    byte: u64,
) {
    if candidate.is_null() {
        null_arena_trap("String::push_byte");
    }
    sanitize_on_push(s as usize);
    if !arena_is_root(candidate) && unsafe { vow_vec_is_owned_by(s, candidate) } {
        unsafe { string_push_byte_in_arena_no_sanitize(candidate, s, byte as u8) };
        return;
    }
    unsafe { with_root_arena(|root| string_push_byte_in_arena_no_sanitize(root, s, byte as u8)) }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn __vow_string_push_byte(s: *mut u8, byte: u64) {
    unsafe { with_growth_arena(s, |arena| __vow_string_push_byte_in_arena(arena, s, byte)) }
}

// ---------------------------------------------------------------------------
// String utility builtins
// ---------------------------------------------------------------------------

#[unsafe(no_mangle)]
pub unsafe extern "C" fn __vow_string_substr_in_arena(
    arena: *mut VowArena,
    s: *const u8,
    start: u64,
    len: u64,
) -> *mut u8 {
    if arena.is_null() {
        null_arena_trap("String::substr");
    }
    if s.is_null() {
        return unsafe { __vow_string_new_in_arena(arena, std::ptr::null(), 0) };
    }
    sanitize_on_read(s as usize, 0);
    let v = unsafe { &*(s as *const VowVec) };
    let slen = v.len as u64;
    let clamped_start = start.min(slen);
    let clamped_len = len.min(slen - clamped_start) as usize;
    let clamped_start = clamped_start as usize;
    let bytes = unsafe { std::slice::from_raw_parts(v.ptr, v.len) };
    unsafe {
        __vow_string_new_in_arena(
            arena,
            bytes[clamped_start..].as_ptr() as *const c_char,
            clamped_len,
        )
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn __vow_string_substr(s: *const u8, start: u64, len: u64) -> *mut u8 {
    unsafe { with_root_arena(|root| __vow_string_substr_in_arena(root, s, start, len)) }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn __vow_string_substring_in_arena(
    arena: *mut VowArena,
    s: *const u8,
    start: u64,
    end: u64,
) -> *mut u8 {
    if arena.is_null() {
        null_arena_trap("String::substring");
    }
    if s.is_null() {
        return unsafe { __vow_string_new_in_arena(arena, std::ptr::null(), 0) };
    }
    sanitize_on_read(s as usize, 0);
    let v = unsafe { &*(s as *const VowVec) };
    let slen = v.len as u64;
    let clamped_start = start.min(slen) as usize;
    let clamped_end = end.clamp(clamped_start as u64, slen) as usize;
    let bytes = unsafe { std::slice::from_raw_parts(v.ptr, v.len) };
    let len = clamped_end - clamped_start;
    unsafe {
        __vow_string_new_in_arena(arena, bytes[clamped_start..].as_ptr() as *const c_char, len)
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn __vow_string_substring(s: *const u8, start: u64, end: u64) -> *mut u8 {
    unsafe { with_root_arena(|root| __vow_string_substring_in_arena(root, s, start, end)) }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn __vow_string_split_in_arena(
    arena: *mut VowArena,
    haystack: *const u8,
    separator: *const u8,
) -> *mut u8 {
    if arena.is_null() {
        null_arena_trap("String::split");
    }
    let result_vec = unsafe { __vow_vec_new_val_in_arena(arena) };
    if haystack.is_null() || separator.is_null() {
        return result_vec;
    }
    sanitize_on_read(haystack as usize, 0);
    sanitize_on_read(separator as usize, 0);
    let vh = unsafe { &*(haystack as *const VowVec) };
    let vs = unsafe { &*(separator as *const VowVec) };
    let h = unsafe { std::slice::from_raw_parts(vh.ptr, vh.len) };
    let s = unsafe { std::slice::from_raw_parts(vs.ptr, vs.len) };

    if s.is_empty() {
        let str_vec =
            unsafe { __vow_string_new_in_arena(arena, h.as_ptr() as *const c_char, h.len()) }
                as i64;
        unsafe { __vow_vec_push_val_in_arena(arena, result_vec, str_vec) };
        return result_vec;
    }

    let mut start = 0;
    while start <= h.len() {
        if let Some(pos) = h[start..].windows(s.len()).position(|w| w == s) {
            let piece = unsafe {
                __vow_string_new_in_arena(arena, h[start..].as_ptr() as *const c_char, pos)
            } as i64;
            unsafe { __vow_vec_push_val_in_arena(arena, result_vec, piece) };
            start += pos + s.len();
        } else {
            let piece = unsafe {
                __vow_string_new_in_arena(
                    arena,
                    h[start..].as_ptr() as *const c_char,
                    h.len() - start,
                )
            } as i64;
            unsafe { __vow_vec_push_val_in_arena(arena, result_vec, piece) };
            break;
        }
    }
    result_vec
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn __vow_string_split(haystack: *const u8, separator: *const u8) -> *mut u8 {
    unsafe { with_root_arena(|root| __vow_string_split_in_arena(root, haystack, separator)) }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn __vow_string_starts_with(s: *const u8, prefix: *const u8) -> i64 {
    if s.is_null() || prefix.is_null() {
        return 0;
    }
    sanitize_on_read(s as usize, 0);
    sanitize_on_read(prefix as usize, 0);
    let vs = unsafe { &*(s as *const VowVec) };
    let vp = unsafe { &*(prefix as *const VowVec) };
    let ss = unsafe { std::slice::from_raw_parts(vs.ptr, vs.len) };
    let sp = unsafe { std::slice::from_raw_parts(vp.ptr, vp.len) };
    if ss.starts_with(sp) { 1 } else { 0 }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn __vow_string_ends_with(s: *const u8, suffix: *const u8) -> i64 {
    if s.is_null() || suffix.is_null() {
        return 0;
    }
    sanitize_on_read(s as usize, 0);
    sanitize_on_read(suffix as usize, 0);
    let vs = unsafe { &*(s as *const VowVec) };
    let vp = unsafe { &*(suffix as *const VowVec) };
    let ss = unsafe { std::slice::from_raw_parts(vs.ptr, vs.len) };
    let sp = unsafe { std::slice::from_raw_parts(vp.ptr, vp.len) };
    if ss.ends_with(sp) { 1 } else { 0 }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn __vow_string_trim_in_arena(arena: *mut VowArena, s: *const u8) -> *mut u8 {
    if arena.is_null() {
        null_arena_trap("String::trim");
    }
    if s.is_null() {
        return unsafe { __vow_string_new_in_arena(arena, std::ptr::null(), 0) };
    }
    sanitize_on_read(s as usize, 0);
    let v = unsafe { &*(s as *const VowVec) };
    let bytes = unsafe { std::slice::from_raw_parts(v.ptr, v.len) };
    let trimmed = match std::str::from_utf8(bytes) {
        Ok(s) => s.trim(),
        Err(_) => return unsafe { __vow_string_new_in_arena(arena, std::ptr::null(), 0) },
    };
    unsafe { __vow_string_new_in_arena(arena, trimmed.as_ptr() as *const c_char, trimmed.len()) }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn __vow_string_trim(s: *const u8) -> *mut u8 {
    unsafe { with_root_arena(|root| __vow_string_trim_in_arena(root, s)) }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn __vow_string_to_upper_in_arena(
    arena: *mut VowArena,
    s: *const u8,
) -> *mut u8 {
    if arena.is_null() {
        null_arena_trap("String::to_upper");
    }
    if s.is_null() {
        return unsafe { __vow_string_new_in_arena(arena, std::ptr::null(), 0) };
    }
    sanitize_on_read(s as usize, 0);
    let v = unsafe { &*(s as *const VowVec) };
    let bytes = unsafe { std::slice::from_raw_parts(v.ptr, v.len) };
    let upper = match std::str::from_utf8(bytes) {
        Ok(s) => s.to_uppercase(),
        Err(_) => return unsafe { __vow_string_new_in_arena(arena, std::ptr::null(), 0) },
    };
    unsafe { __vow_string_new_in_arena(arena, upper.as_ptr() as *const c_char, upper.len()) }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn __vow_string_to_upper(s: *const u8) -> *mut u8 {
    unsafe { with_root_arena(|root| __vow_string_to_upper_in_arena(root, s)) }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn __vow_string_to_lower_in_arena(
    arena: *mut VowArena,
    s: *const u8,
) -> *mut u8 {
    if arena.is_null() {
        null_arena_trap("String::to_lower");
    }
    if s.is_null() {
        return unsafe { __vow_string_new_in_arena(arena, std::ptr::null(), 0) };
    }
    sanitize_on_read(s as usize, 0);
    let v = unsafe { &*(s as *const VowVec) };
    let bytes = unsafe { std::slice::from_raw_parts(v.ptr, v.len) };
    let lower = match std::str::from_utf8(bytes) {
        Ok(s) => s.to_lowercase(),
        Err(_) => return unsafe { __vow_string_new_in_arena(arena, std::ptr::null(), 0) },
    };
    unsafe { __vow_string_new_in_arena(arena, lower.as_ptr() as *const c_char, lower.len()) }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn __vow_string_to_lower(s: *const u8) -> *mut u8 {
    unsafe { with_root_arena(|root| __vow_string_to_lower_in_arena(root, s)) }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn __vow_string_replace_in_arena(
    arena: *mut VowArena,
    s: *const u8,
    from: *const u8,
    to: *const u8,
) -> *mut u8 {
    if arena.is_null() {
        null_arena_trap("String::replace");
    }
    if s.is_null() || from.is_null() || to.is_null() {
        return unsafe { __vow_string_new_in_arena(arena, std::ptr::null(), 0) };
    }
    sanitize_on_read(s as usize, 0);
    sanitize_on_read(from as usize, 0);
    sanitize_on_read(to as usize, 0);
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
        _ => return unsafe { __vow_string_new_in_arena(arena, std::ptr::null(), 0) },
    };
    let result = ss_str.replace(sf_str, st_str);
    unsafe { __vow_string_new_in_arena(arena, result.as_ptr() as *const c_char, result.len()) }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn __vow_string_replace(
    s: *const u8,
    from: *const u8,
    to: *const u8,
) -> *mut u8 {
    unsafe { with_root_arena(|root| __vow_string_replace_in_arena(root, s, from, to)) }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn __vow_string_join_in_arena(
    arena: *mut VowArena,
    vec_ptr: *const u8,
    sep: *const u8,
) -> *mut u8 {
    if arena.is_null() {
        null_arena_trap("String::join");
    }
    if vec_ptr.is_null() || sep.is_null() {
        return unsafe { __vow_string_new_in_arena(arena, std::ptr::null(), 0) };
    }
    sanitize_on_read(vec_ptr as usize, 0);
    sanitize_on_read(sep as usize, 0);
    let v = unsafe { &*(vec_ptr as *const VowVec) };
    let ptrs = unsafe { std::slice::from_raw_parts(v.ptr as *const i64, v.len) };

    let result = unsafe { __vow_string_new_in_arena(arena, std::ptr::null(), 0) };
    for (i, &str_ptr) in ptrs.iter().enumerate() {
        if i > 0 {
            unsafe { __vow_string_push_str_in_arena(arena, result, sep) };
        }
        unsafe { __vow_string_push_str_in_arena(arena, result, str_ptr as *const u8) };
    }
    result
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn __vow_string_join(vec_ptr: *const u8, sep: *const u8) -> *mut u8 {
    unsafe { with_root_arena(|root| __vow_string_join_in_arena(root, vec_ptr, sep)) }
}

/// A fresh `Option<N>` cell owned by `arena`: the bare 16-byte `[tag, payload]`
/// shape the compilers use for a source-level `Option`, not a `VowVec` descriptor.
unsafe fn alloc_option_in_arena(
    arena: *mut VowArena,
    operation: &'static str,
    value: Option<i64>,
) -> *mut u8 {
    if arena.is_null() {
        null_arena_trap(operation);
    }
    let cell = unsafe { __vow_arena_alloc(arena, 16, 8) } as *mut i64;
    let (tag, payload) = match value {
        Some(payload) => (1, payload),
        None => (0, 0),
    };
    unsafe {
        *cell = tag;
        *cell.add(1) = payload;
    }
    cell as *mut u8
}

unsafe fn parse_string_arg<T: std::str::FromStr>(s: *const u8) -> Option<T> {
    if s.is_null() {
        return None;
    }
    sanitize_on_read(s as usize, 0);
    let v = unsafe { &*(s as *const VowVec) };
    let bytes = unsafe { std::slice::from_raw_parts(v.ptr, v.len) };
    std::str::from_utf8(bytes)
        .ok()
        .and_then(|text| text.trim().parse::<T>().ok())
}

// Every `Option`-returning builtin comes as a pair: `<name>_in_arena`, which
// allocates the result cell in the region the compiler inferred for it, and the
// root-arena `<name>`, used when that region is the root (the result escapes
// the function).
macro_rules! define_option_parser {
    ($parse_name:ident, $parse_arena_name:ident, $ty:ty) => {
        #[unsafe(no_mangle)]
        pub unsafe extern "C" fn $parse_arena_name(arena: *mut VowArena, s: *const u8) -> *mut u8 {
            let value = unsafe { parse_string_arg::<$ty>(s) };
            unsafe {
                alloc_option_in_arena(
                    arena,
                    stringify!($parse_arena_name),
                    value.map(|v| v as i64),
                )
            }
        }

        #[unsafe(no_mangle)]
        pub unsafe extern "C" fn $parse_name(s: *const u8) -> *mut u8 {
            unsafe { with_root_arena(|arena| $parse_arena_name(arena, s)) }
        }
    };
}

define_option_parser!(
    __vow_string_parse_i64_opt,
    __vow_string_parse_i64_opt_in_arena,
    i64
);
define_option_parser!(
    __vow_string_parse_u64_opt,
    __vow_string_parse_u64_opt_in_arena,
    u64
);
define_option_parser!(
    __vow_string_parse_i8_opt,
    __vow_string_parse_i8_opt_in_arena,
    i8
);
define_option_parser!(
    __vow_string_parse_u8_opt,
    __vow_string_parse_u8_opt_in_arena,
    u8
);
define_option_parser!(
    __vow_string_parse_i16_opt,
    __vow_string_parse_i16_opt_in_arena,
    i16
);
define_option_parser!(
    __vow_string_parse_u16_opt,
    __vow_string_parse_u16_opt_in_arena,
    u16
);
define_option_parser!(
    __vow_string_parse_i32_opt,
    __vow_string_parse_i32_opt_in_arena,
    i32
);
define_option_parser!(
    __vow_string_parse_u32_opt,
    __vow_string_parse_u32_opt_in_arena,
    u32
);

macro_rules! define_try_conversion {
    ($try_name:ident, $try_arena_name:ident, $source:ty, $target:ty) => {
        #[unsafe(no_mangle)]
        pub unsafe extern "C" fn $try_arena_name(arena: *mut VowArena, value: $source) -> *mut u8 {
            unsafe {
                alloc_option_in_arena(
                    arena,
                    stringify!($try_arena_name),
                    <$target>::try_from(value).ok().map(|v| v as i64),
                )
            }
        }

        #[unsafe(no_mangle)]
        pub unsafe extern "C" fn $try_name(value: $source) -> *mut u8 {
            unsafe { with_root_arena(|arena| $try_arena_name(arena, value)) }
        }
    };
}
macro_rules! define_signed_to_u8 {
    ($try_name:ident, $try_arena_name:ident, $wrap_name:ident, $sat_name:ident, $ty:ty) => {
        define_try_conversion!($try_name, $try_arena_name, $ty, u8);

        #[unsafe(no_mangle)]
        pub extern "C" fn $wrap_name(value: $ty) -> u8 {
            value as u8
        }

        #[unsafe(no_mangle)]
        pub extern "C" fn $sat_name(value: $ty) -> u8 {
            value.clamp(0, 255) as u8
        }
    };
}

macro_rules! define_unsigned_to_u8 {
    ($try_name:ident, $try_arena_name:ident, $wrap_name:ident, $sat_name:ident, $ty:ty) => {
        define_try_conversion!($try_name, $try_arena_name, $ty, u8);

        #[unsafe(no_mangle)]
        pub extern "C" fn $wrap_name(value: $ty) -> u8 {
            value as u8
        }

        #[unsafe(no_mangle)]
        pub extern "C" fn $sat_name(value: $ty) -> u8 {
            value.min(255) as u8
        }
    };
}

define_signed_to_u8!(
    __vow_i16_to_u8_try,
    __vow_i16_to_u8_try_in_arena,
    __vow_i16_to_u8_wrap,
    __vow_i16_to_u8_sat,
    i16
);
define_signed_to_u8!(
    __vow_i32_to_u8_try,
    __vow_i32_to_u8_try_in_arena,
    __vow_i32_to_u8_wrap,
    __vow_i32_to_u8_sat,
    i32
);
define_signed_to_u8!(
    __vow_i64_to_u8_try,
    __vow_i64_to_u8_try_in_arena,
    __vow_i64_to_u8_wrap,
    __vow_i64_to_u8_sat,
    i64
);
define_signed_to_u8!(
    __vow_i128_to_u8_try,
    __vow_i128_to_u8_try_in_arena,
    __vow_i128_to_u8_wrap,
    __vow_i128_to_u8_sat,
    i128
);
define_unsigned_to_u8!(
    __vow_u16_to_u8_try,
    __vow_u16_to_u8_try_in_arena,
    __vow_u16_to_u8_wrap,
    __vow_u16_to_u8_sat,
    u16
);
define_unsigned_to_u8!(
    __vow_u32_to_u8_try,
    __vow_u32_to_u8_try_in_arena,
    __vow_u32_to_u8_wrap,
    __vow_u32_to_u8_sat,
    u32
);
define_unsigned_to_u8!(
    __vow_u64_to_u8_try,
    __vow_u64_to_u8_try_in_arena,
    __vow_u64_to_u8_wrap,
    __vow_u64_to_u8_sat,
    u64
);
define_unsigned_to_u8!(
    __vow_u128_to_u8_try,
    __vow_u128_to_u8_try_in_arena,
    __vow_u128_to_u8_wrap,
    __vow_u128_to_u8_sat,
    u128
);

macro_rules! define_signed_to_i32 {
    ($try_name:ident, $try_arena_name:ident, $wrap_name:ident, $sat_name:ident, $ty:ty) => {
        define_try_conversion!($try_name, $try_arena_name, $ty, i32);

        #[unsafe(no_mangle)]
        pub extern "C" fn $wrap_name(value: $ty) -> i32 {
            value as i32
        }

        #[unsafe(no_mangle)]
        pub extern "C" fn $sat_name(value: $ty) -> i32 {
            value.clamp(i32::MIN as $ty, i32::MAX as $ty) as i32
        }
    };
}

macro_rules! define_unsigned_to_i32 {
    ($try_name:ident, $try_arena_name:ident, $wrap_name:ident, $sat_name:ident, $ty:ty) => {
        define_try_conversion!($try_name, $try_arena_name, $ty, i32);

        #[unsafe(no_mangle)]
        pub extern "C" fn $wrap_name(value: $ty) -> i32 {
            value as i32
        }

        #[unsafe(no_mangle)]
        pub extern "C" fn $sat_name(value: $ty) -> i32 {
            value.min(i32::MAX as $ty) as i32
        }
    };
}

define_signed_to_i32!(
    __vow_i64_to_i32_try,
    __vow_i64_to_i32_try_in_arena,
    __vow_i64_to_i32_wrap,
    __vow_i64_to_i32_sat,
    i64
);
define_unsigned_to_i32!(
    __vow_u32_to_i32_try,
    __vow_u32_to_i32_try_in_arena,
    __vow_u32_to_i32_wrap,
    __vow_u32_to_i32_sat,
    u32
);
define_unsigned_to_i32!(
    __vow_u64_to_i32_try,
    __vow_u64_to_i32_try_in_arena,
    __vow_u64_to_i32_wrap,
    __vow_u64_to_i32_sat,
    u64
);

macro_rules! define_narrowing_intrinsic {
    ($try_name:ident, $try_arena_name:ident, $wrap_name:ident, $sat_name:ident, $source:ty, $target:ty, $sat:expr) => {
        define_try_conversion!($try_name, $try_arena_name, $source, $target);

        #[unsafe(no_mangle)]
        pub extern "C" fn $wrap_name(value: $source) -> $target {
            value as $target
        }

        #[unsafe(no_mangle)]
        pub extern "C" fn $sat_name(value: $source) -> $target {
            ($sat)(value)
        }
    };
}

macro_rules! define_signed_to_signed {
    ($try_name:ident, $try_arena_name:ident, $wrap_name:ident, $sat_name:ident, $source:ty, $target:ty) => {
        define_narrowing_intrinsic!(
            $try_name,
            $try_arena_name,
            $wrap_name,
            $sat_name,
            $source,
            $target,
            |value: $source| value.clamp(<$target>::MIN as $source, <$target>::MAX as $source)
                as $target
        );
    };
}

macro_rules! define_unsigned_to_signed {
    ($try_name:ident, $try_arena_name:ident, $wrap_name:ident, $sat_name:ident, $source:ty, $target:ty) => {
        define_narrowing_intrinsic!(
            $try_name,
            $try_arena_name,
            $wrap_name,
            $sat_name,
            $source,
            $target,
            |value: $source| value.min(<$target>::MAX as $source) as $target
        );
    };
}

macro_rules! define_signed_to_unsigned {
    ($try_name:ident, $try_arena_name:ident, $wrap_name:ident, $sat_name:ident, $source:ty, $target:ty) => {
        define_narrowing_intrinsic!(
            $try_name,
            $try_arena_name,
            $wrap_name,
            $sat_name,
            $source,
            $target,
            |value: $source| value.clamp(0, <$target>::MAX as $source) as $target
        );
    };
}

macro_rules! define_unsigned_to_unsigned {
    ($try_name:ident, $try_arena_name:ident, $wrap_name:ident, $sat_name:ident, $source:ty, $target:ty) => {
        define_narrowing_intrinsic!(
            $try_name,
            $try_arena_name,
            $wrap_name,
            $sat_name,
            $source,
            $target,
            |value: $source| value.min(<$target>::MAX as $source) as $target
        );
    };
}

define_signed_to_signed!(
    __vow_i16_to_i8_try,
    __vow_i16_to_i8_try_in_arena,
    __vow_i16_to_i8_wrap,
    __vow_i16_to_i8_sat,
    i16,
    i8
);
define_unsigned_to_signed!(
    __vow_u16_to_i8_try,
    __vow_u16_to_i8_try_in_arena,
    __vow_u16_to_i8_wrap,
    __vow_u16_to_i8_sat,
    u16,
    i8
);
define_signed_to_signed!(
    __vow_i32_to_i8_try,
    __vow_i32_to_i8_try_in_arena,
    __vow_i32_to_i8_wrap,
    __vow_i32_to_i8_sat,
    i32,
    i8
);
define_unsigned_to_signed!(
    __vow_u32_to_i8_try,
    __vow_u32_to_i8_try_in_arena,
    __vow_u32_to_i8_wrap,
    __vow_u32_to_i8_sat,
    u32,
    i8
);
define_signed_to_signed!(
    __vow_i64_to_i8_try,
    __vow_i64_to_i8_try_in_arena,
    __vow_i64_to_i8_wrap,
    __vow_i64_to_i8_sat,
    i64,
    i8
);
define_unsigned_to_signed!(
    __vow_u64_to_i8_try,
    __vow_u64_to_i8_try_in_arena,
    __vow_u64_to_i8_wrap,
    __vow_u64_to_i8_sat,
    u64,
    i8
);

define_signed_to_signed!(
    __vow_i32_to_i16_try,
    __vow_i32_to_i16_try_in_arena,
    __vow_i32_to_i16_wrap,
    __vow_i32_to_i16_sat,
    i32,
    i16
);
define_unsigned_to_signed!(
    __vow_u32_to_i16_try,
    __vow_u32_to_i16_try_in_arena,
    __vow_u32_to_i16_wrap,
    __vow_u32_to_i16_sat,
    u32,
    i16
);
define_signed_to_signed!(
    __vow_i64_to_i16_try,
    __vow_i64_to_i16_try_in_arena,
    __vow_i64_to_i16_wrap,
    __vow_i64_to_i16_sat,
    i64,
    i16
);
define_unsigned_to_signed!(
    __vow_u64_to_i16_try,
    __vow_u64_to_i16_try_in_arena,
    __vow_u64_to_i16_wrap,
    __vow_u64_to_i16_sat,
    u64,
    i16
);

define_signed_to_unsigned!(
    __vow_i32_to_u16_try,
    __vow_i32_to_u16_try_in_arena,
    __vow_i32_to_u16_wrap,
    __vow_i32_to_u16_sat,
    i32,
    u16
);
define_unsigned_to_unsigned!(
    __vow_u32_to_u16_try,
    __vow_u32_to_u16_try_in_arena,
    __vow_u32_to_u16_wrap,
    __vow_u32_to_u16_sat,
    u32,
    u16
);
define_signed_to_unsigned!(
    __vow_i64_to_u16_try,
    __vow_i64_to_u16_try_in_arena,
    __vow_i64_to_u16_wrap,
    __vow_i64_to_u16_sat,
    i64,
    u16
);
define_unsigned_to_unsigned!(
    __vow_u64_to_u16_try,
    __vow_u64_to_u16_try_in_arena,
    __vow_u64_to_u16_wrap,
    __vow_u64_to_u16_sat,
    u64,
    u16
);

define_signed_to_unsigned!(
    __vow_i64_to_u32_try,
    __vow_i64_to_u32_try_in_arena,
    __vow_i64_to_u32_wrap,
    __vow_i64_to_u32_sat,
    i64,
    u32
);
define_unsigned_to_unsigned!(
    __vow_u64_to_u32_try,
    __vow_u64_to_u32_try_in_arena,
    __vow_u64_to_u32_wrap,
    __vow_u64_to_u32_sat,
    u64,
    u32
);

define_signed_to_signed!(
    __vow_i128_to_i32_try,
    __vow_i128_to_i32_try_in_arena,
    __vow_i128_to_i32_wrap,
    __vow_i128_to_i32_sat,
    i128,
    i32
);
define_unsigned_to_signed!(
    __vow_u128_to_i32_try,
    __vow_u128_to_i32_try_in_arena,
    __vow_u128_to_i32_wrap,
    __vow_u128_to_i32_sat,
    u128,
    i32
);
define_signed_to_signed!(
    __vow_i128_to_i8_try,
    __vow_i128_to_i8_try_in_arena,
    __vow_i128_to_i8_wrap,
    __vow_i128_to_i8_sat,
    i128,
    i8
);
define_unsigned_to_signed!(
    __vow_u128_to_i8_try,
    __vow_u128_to_i8_try_in_arena,
    __vow_u128_to_i8_wrap,
    __vow_u128_to_i8_sat,
    u128,
    i8
);
define_signed_to_signed!(
    __vow_i128_to_i16_try,
    __vow_i128_to_i16_try_in_arena,
    __vow_i128_to_i16_wrap,
    __vow_i128_to_i16_sat,
    i128,
    i16
);
define_unsigned_to_signed!(
    __vow_u128_to_i16_try,
    __vow_u128_to_i16_try_in_arena,
    __vow_u128_to_i16_wrap,
    __vow_u128_to_i16_sat,
    u128,
    i16
);
define_signed_to_unsigned!(
    __vow_i128_to_u16_try,
    __vow_i128_to_u16_try_in_arena,
    __vow_i128_to_u16_wrap,
    __vow_i128_to_u16_sat,
    i128,
    u16
);
define_unsigned_to_unsigned!(
    __vow_u128_to_u16_try,
    __vow_u128_to_u16_try_in_arena,
    __vow_u128_to_u16_wrap,
    __vow_u128_to_u16_sat,
    u128,
    u16
);
define_signed_to_unsigned!(
    __vow_i128_to_u32_try,
    __vow_i128_to_u32_try_in_arena,
    __vow_i128_to_u32_wrap,
    __vow_i128_to_u32_sat,
    i128,
    u32
);
define_unsigned_to_unsigned!(
    __vow_u128_to_u32_try,
    __vow_u128_to_u32_try_in_arena,
    __vow_u128_to_u32_wrap,
    __vow_u128_to_u32_sat,
    u128,
    u32
);
// The same-width sign-change pairs expose `_wrap`/`_sat` only: `_try` would
// return `Option<i128>`/`Option<u128>`, and 128-bit enum payloads are not
// supported yet (epic #526 — an aggregate field slot is 8 bytes), so there is
// no representable option cell for a try conversion to fill.
#[unsafe(no_mangle)]
pub extern "C" fn __vow_u128_to_i128_wrap(value: u128) -> i128 {
    value as i128
}

#[unsafe(no_mangle)]
pub extern "C" fn __vow_u128_to_i128_sat(value: u128) -> i128 {
    value.min(i128::MAX as u128) as i128
}

#[unsafe(no_mangle)]
pub extern "C" fn __vow_i128_to_u128_wrap(value: i128) -> u128 {
    value as u128
}

#[unsafe(no_mangle)]
pub extern "C" fn __vow_i128_to_u128_sat(value: i128) -> u128 {
    value.max(0) as u128
}

#[unsafe(no_mangle)]
pub extern "C" fn __vow_add_sat_u8(a: u8, b: u8) -> u8 {
    a.saturating_add(b)
}

#[unsafe(no_mangle)]
pub extern "C" fn __vow_sub_sat_u8(a: u8, b: u8) -> u8 {
    a.saturating_sub(b)
}

#[unsafe(no_mangle)]
pub extern "C" fn __vow_mul_sat_u8(a: u8, b: u8) -> u8 {
    a.saturating_mul(b)
}

// ---------------------------------------------------------------------------
// 128-bit division, remainder, and multiply-overflow detection
//
// Cranelift 0.134 cannot lower `sdiv`/`udiv`/`urem` on I128 (it reports
// `Unsupported`), `srem.i128` panics inside Cranelift itself on x86_64, and
// `smul_overflow`/`umul_overflow` are rejected by Cranelift's own IR verifier
// because their type-constraint tables exclude I128. Both backends therefore
// emit calls to these helpers instead of the native opcodes.
//
// The backends call `__vow_arithmetic_overflow` before the operation for the
// same conditions Cranelift traps on at narrower widths — a zero divisor for
// all four operations, plus `MIN / -1` for signed division — so neither case
// reaches this code. The guards below are unreachable backstops that keep the
// helpers total: a Rust panic crossing an `extern "C"` boundary would abort
// with no diagnostic at all.
//
// `wrapping_div`/`wrapping_rem` (rather than `/`/`%`) make the `MIN / -1` and
// `MIN % -1` cases total too; `MIN % -1` is 0 at every width and deliberately
// does not trap, matching `srem`.
// ---------------------------------------------------------------------------

macro_rules! define_wide_div_rem {
    ($div_name:ident, $rem_name:ident, $ty:ty) => {
        #[unsafe(no_mangle)]
        pub extern "C" fn $div_name(a: $ty, b: $ty) -> $ty {
            if b == 0 {
                __vow_arithmetic_overflow();
            }
            a.wrapping_div(b)
        }

        #[unsafe(no_mangle)]
        pub extern "C" fn $rem_name(a: $ty, b: $ty) -> $ty {
            if b == 0 {
                __vow_arithmetic_overflow();
            }
            a.wrapping_rem(b)
        }
    };
}

define_wide_div_rem!(__vow_i128_div, __vow_i128_rem, i128);
define_wide_div_rem!(__vow_u128_div, __vow_u128_rem, u128);

/// Returns 1 when `a * b` overflows, 0 otherwise. The product itself is
/// computed by a native `imul.i128`, which Cranelift does lower; only the
/// overflow flag needs a helper, so the checked-multiply trap stays on the
/// backends' shared `emit_overflow_check` path.
#[unsafe(no_mangle)]
pub extern "C" fn __vow_i128_mul_overflow(a: i128, b: i128) -> i8 {
    a.checked_mul(b).is_none() as i8
}

#[unsafe(no_mangle)]
pub extern "C" fn __vow_u128_mul_overflow(a: u128, b: u128) -> i8 {
    a.checked_mul(b).is_none() as i8
}

// ---------------------------------------------------------------------------
// Utility builtins
// ---------------------------------------------------------------------------

#[unsafe(no_mangle)]
pub unsafe extern "C" fn __vow_vec_sort_in_arena(arena: *mut VowArena, vec: *const u8) -> *mut u8 {
    let sorted = unsafe { sorted_values(vec) };
    unsafe { alloc_vec_of_values(arena, &sorted) }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn __vow_vec_sort(vec: *const u8) -> *mut u8 {
    let sorted = unsafe { sorted_values(vec) };
    unsafe { with_root_arena(|arena| alloc_vec_of_values(arena, &sorted)) }
}

// `__vow_perf_count_vec_sort` mirrors this function's growth; update that cost
// model if this algorithm changes.
unsafe fn sorted_values(vec: *const u8) -> Vec<i64> {
    if vec.is_null() {
        return Vec::new();
    }
    sanitize_on_read(vec as usize, 0);
    let v = unsafe { &*(vec as *const VowVec) };
    let src = unsafe { std::slice::from_raw_parts(v.ptr as *const i64, v.len) };
    let mut sorted: Vec<i64> = src.to_vec();
    sorted.sort_unstable();
    sorted
}

unsafe fn alloc_vec_of_values(arena: *mut VowArena, values: &[i64]) -> *mut u8 {
    unsafe { __vow_vec_from_raw_parts_copy_val(arena, values.as_ptr(), values.len()) }
}

#[unsafe(no_mangle)]
pub extern "C" fn __vow_time_unix() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

#[unsafe(no_mangle)]
pub extern "C" fn __vow_time_unix_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

/// Monotonic microseconds since the first call (the process clock origin). Used
/// by the --perfetto tracer as its `now_us` source, mirroring the Rust driver's
/// `Instant`-based `Profiler::now_us` (issue #784). The first call returns ~0.
#[unsafe(no_mangle)]
pub extern "C" fn __vow_time_micros() -> i64 {
    use std::sync::OnceLock;
    use std::time::Instant;
    static EPOCH: OnceLock<Instant> = OnceLock::new();
    EPOCH.get_or_init(Instant::now).elapsed().as_micros() as i64
}

#[unsafe(no_mangle)]
pub extern "C" fn __vow_num_cpus() -> i64 {
    std::thread::available_parallelism()
        .map(|n| n.get() as i64)
        .unwrap_or(1)
}

// ── Perfetto resource sampling (issue #784) ────────────────────────────────
// One OS process snapshot, decoupled from /proc so the reducer is unit-testable
// with synthetic tables. Mirrors vow/src/perfetto.rs::ProcInfo.
#[derive(Clone, Debug)]
struct ProcInfo {
    pid: u64,
    parent: Option<u64>,
    name: String,
    rss_kb: f64,
    cpu_pct: f64,
}

/// One resource sample to emit as a counter group. Mirrors perfetto.rs::Sample.
#[derive(Clone, Debug, PartialEq)]
struct ProcSample {
    group: String,
    pid: u64,
    rss_kb: f64,
    cpu_pct: f64,
}

/// Reduce a process table to the compiler self sample (`group = "compiler"`)
/// plus one group per ESBMC child of `own_pid` (`group = "esbmc:<pid>"`) summing
/// that child's whole descendant subtree so the SMT solver's memory (z3/
/// boolector run as ESBMC's children) is counted. Excludes unrelated system
/// `esbmc` processes (parent check) and the linker child (name check). This is a
/// faithful port of vow/src/perfetto.rs::collect_samples.
fn collect_proc_samples(procs: &[ProcInfo], own_pid: u64) -> Vec<ProcSample> {
    let mut samples = Vec::new();
    if let Some(me) = procs.iter().find(|p| p.pid == own_pid) {
        samples.push(ProcSample {
            group: "compiler".to_string(),
            pid: own_pid,
            rss_kb: me.rss_kb,
            cpu_pct: me.cpu_pct,
        });
    }

    let mut children: std::collections::HashMap<u64, Vec<u64>> = std::collections::HashMap::new();
    for p in procs {
        if let Some(par) = p.parent {
            children.entry(par).or_default().push(p.pid);
        }
    }
    let by_pid: std::collections::HashMap<u64, &ProcInfo> =
        procs.iter().map(|p| (p.pid, p)).collect();

    let mut esbmc_children: Vec<&ProcInfo> = procs
        .iter()
        .filter(|p| p.parent == Some(own_pid) && p.name.starts_with("esbmc"))
        .collect();
    esbmc_children.sort_by_key(|p| p.pid);

    for child in esbmc_children {
        // Sum the child's whole descendant subtree (esbmc + its solver procs).
        let (mut rss_kb, mut cpu_pct) = (0.0, 0.0);
        let mut stack = vec![child.pid];
        while let Some(pid) = stack.pop() {
            if let Some(p) = by_pid.get(&pid) {
                rss_kb += p.rss_kb;
                cpu_pct += p.cpu_pct;
            }
            if let Some(kids) = children.get(&pid) {
                stack.extend(kids.iter().copied());
            }
        }
        samples.push(ProcSample {
            group: format!("esbmc:{}", child.pid),
            pid: child.pid,
            rss_kb,
            cpu_pct,
        });
    }

    samples
}

/// Read the live process table from /proc (Linux only). RSS comes from
/// /proc/<pid>/statm (resident pages); ppid/comm/jiffies from /proc/<pid>/stat.
/// `cpu_pct` is best-effort: a CPU-jiffy delta between successive calls divided
/// by the wall-time delta (0% on a process's first sighting). Returns an empty
/// table on non-Linux or on any read error, so callers degrade to no counters.
#[cfg(target_os = "linux")]
fn read_proc_table() -> Vec<ProcInfo> {
    use std::sync::{Mutex, OnceLock};
    use std::time::Instant;

    static SAMPLE_EPOCH: OnceLock<Instant> = OnceLock::new();
    static CPU_PREV: OnceLock<Mutex<std::collections::HashMap<u64, (u64, u128)>>> = OnceLock::new();

    let now_us = SAMPLE_EPOCH.get_or_init(Instant::now).elapsed().as_micros();
    let clk_tck = unsafe { libc::sysconf(libc::_SC_CLK_TCK) } as f64;
    let page_kb = (unsafe { libc::sysconf(libc::_SC_PAGESIZE) } as f64) / 1024.0;

    let prev_lock = CPU_PREV.get_or_init(|| Mutex::new(std::collections::HashMap::new()));
    let mut prev = match prev_lock.lock() {
        Ok(g) => g,
        Err(p) => p.into_inner(),
    };

    let dir = match std::fs::read_dir("/proc") {
        Ok(d) => d,
        Err(_) => return Vec::new(),
    };
    let mut out = Vec::new();
    for entry in dir.flatten() {
        let fname = entry.file_name();
        let name = match fname.to_str() {
            Some(s) => s,
            None => continue,
        };
        let pid: u64 = match name.parse() {
            Ok(p) => p,
            Err(_) => continue, // non-numeric /proc entry
        };

        let stat = match std::fs::read_to_string(format!("/proc/{pid}/stat")) {
            Ok(s) => s,
            Err(_) => continue, // process gone or unreadable
        };
        // comm is wrapped in parens and may contain spaces/parens: fields after
        // the last ')' are state(0) ppid(1) ... utime(11) stime(12).
        let open = stat.find('(');
        let close = stat.rfind(')');
        let (comm, after) = match (open, close) {
            (Some(o), Some(c)) if c > o => (stat[o + 1..c].to_string(), &stat[c + 1..]),
            _ => continue,
        };
        let fields: Vec<&str> = after.split_whitespace().collect();
        if fields.len() < 13 {
            continue;
        }
        let parent: Option<u64> = fields[1].parse().ok();
        let utime: u64 = fields[11].parse().unwrap_or(0);
        let stime: u64 = fields[12].parse().unwrap_or(0);
        let total_jiffies = utime + stime;

        let rss_kb = std::fs::read_to_string(format!("/proc/{pid}/statm"))
            .ok()
            .and_then(|s| {
                s.split_whitespace()
                    .nth(1)
                    .and_then(|r| r.parse::<f64>().ok())
            })
            .map(|pages| pages * page_kb)
            .unwrap_or(0.0);

        let cpu_pct = match prev.get(&pid) {
            Some(&(prev_j, prev_us)) if clk_tck > 0.0 && now_us > prev_us => {
                let dj = total_jiffies.saturating_sub(prev_j) as f64;
                let dt_s = (now_us - prev_us) as f64 / 1_000_000.0;
                if dt_s > 0.0 {
                    (dj / clk_tck) / dt_s * 100.0
                } else {
                    0.0
                }
            }
            _ => 0.0,
        };
        prev.insert(pid, (total_jiffies, now_us));

        out.push(ProcInfo {
            pid,
            parent,
            name: comm,
            rss_kb,
            cpu_pct,
        });
    }
    out
}

#[cfg(not(target_os = "linux"))]
fn read_proc_table() -> Vec<ProcInfo> {
    Vec::new()
}

/// Sample this process and its ESBMC children, returning a compact newline-
/// separated string `"<group>|<rss_kb>|<cpu_pct>"` per line (integers), e.g.
/// `"compiler|10240|3\nesbmc:8123|512000|198"`. Empty string on non-Linux or
/// when nothing could be read — the Vow tracer then emits no counters.
#[unsafe(no_mangle)]
pub extern "C" fn __vow_proc_sample() -> *mut u8 {
    let own_pid = std::process::id() as u64;
    let procs = read_proc_table();
    let samples = collect_proc_samples(&procs, own_pid);
    let mut s = String::new();
    for smp in &samples {
        if !s.is_empty() {
            s.push('\n');
        }
        s.push_str(&format!(
            "{}|{}|{}",
            smp.group, smp.rss_kb as i64, smp.cpu_pct as i64
        ));
    }
    unsafe { __vow_string_new(s.as_ptr() as *const c_char, s.len()) }
}

/// Gzip-compress `data` and write it to `path` (both Vow Strings). Returns 0 on
/// success, non-zero on error. Used by the self-hosted --perfetto tracer to emit
/// the gzipped Chrome Trace Event Format file (issue #784). ui.perfetto.dev
/// auto-decompresses a single gzip stream.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn __vow_gzip_write_file(path_ptr: *const u8, data_ptr: *const u8) -> i64 {
    use std::io::Write;
    if path_ptr.is_null() || data_ptr.is_null() {
        return -1;
    }
    sanitize_on_read(path_ptr as usize, 0);
    sanitize_on_read(data_ptr as usize, 0);
    let vp = unsafe { &*(path_ptr as *const VowVec) };
    let path_bytes = unsafe { std::slice::from_raw_parts(vp.ptr, vp.len) };
    let path = match std::str::from_utf8(path_bytes) {
        Ok(s) => s,
        Err(_) => return -1,
    };
    let vd = unsafe { &*(data_ptr as *const VowVec) };
    let bytes = unsafe { std::slice::from_raw_parts(vd.ptr, vd.len) };
    let file = match std::fs::File::create(path) {
        Ok(f) => f,
        Err(_) => return -1,
    };
    let mut enc = flate2::write::GzEncoder::new(file, flate2::Compression::default());
    if enc.write_all(bytes).is_err() {
        return -1;
    }
    match enc.finish() {
        Ok(_) => 0,
        Err(_) => -1,
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn __vow_hex_encode_in_arena(
    arena: *mut VowArena,
    vec: *const u8,
) -> *mut u8 {
    let hex = unsafe { hex_text(vec) };
    unsafe { alloc_hex_result(arena, hex.as_deref()) }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn __vow_hex_encode(vec: *const u8) -> *mut u8 {
    let hex = unsafe { hex_text(vec) };
    unsafe { with_root_arena(|arena| alloc_hex_result(arena, hex.as_deref())) }
}

const HEX_DIGITS: &[u8; 16] = b"0123456789abcdef";

unsafe fn hex_text(vec: *const u8) -> Option<String> {
    if vec.is_null() {
        return None;
    }
    sanitize_on_read(vec as usize, 0);
    let v = unsafe { &*(vec as *const VowVec) };
    let vals = unsafe { std::slice::from_raw_parts(v.ptr as *const i64, v.len) };
    let mut hex = String::with_capacity(vals.len() * 2);
    for &val in vals {
        let byte = (val & 0xff) as usize;
        hex.push(HEX_DIGITS[byte >> 4] as char);
        hex.push(HEX_DIGITS[byte & 0xf] as char);
    }
    Some(hex)
}

unsafe fn alloc_hex_result(arena: *mut VowArena, hex: Option<&str>) -> *mut u8 {
    match hex {
        None => unsafe { __vow_vec_new_in_arena(arena, 1, 1) },
        Some(hex) => unsafe { alloc_bytes_string(arena, hex.as_bytes()) },
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn __vow_hex_decode_in_arena(arena: *mut VowArena, s: *const u8) -> *mut u8 {
    let decoded = unsafe { decoded_hex(s) };
    unsafe { alloc_vec_of_values(arena, &decoded) }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn __vow_hex_decode(s: *const u8) -> *mut u8 {
    let decoded = unsafe { decoded_hex(s) };
    unsafe { with_root_arena(|arena| alloc_vec_of_values(arena, &decoded)) }
}

/// The decoded bytes, or an empty list when `s` is null, not UTF-8, of odd
/// length or not entirely hexadecimal.
unsafe fn decoded_hex(s: *const u8) -> Vec<i64> {
    if s.is_null() {
        return Vec::new();
    }
    sanitize_on_read(s as usize, 0);
    let v = unsafe { &*(s as *const VowVec) };
    let bytes = unsafe { std::slice::from_raw_parts(v.ptr, v.len) };
    let Ok(hex_str) = std::str::from_utf8(bytes) else {
        return Vec::new();
    };
    if hex_str.len() % 2 != 0 {
        return Vec::new();
    }
    let mut out = Vec::with_capacity(hex_str.len() / 2);
    let mut i = 0;
    while i < hex_str.len() {
        match hex_str
            .get(i..i + 2)
            .and_then(|pair| u8::from_str_radix(pair, 16).ok())
        {
            Some(byte) => out.push(byte as i64),
            None => return Vec::new(),
        }
        i += 2;
    }
    out
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn __vow_parse_f64_bits(s: *const u8) -> u64 {
    if s.is_null() {
        return 0;
    }
    sanitize_on_read(s as usize, 0);
    let v = unsafe { &*(s as *const VowVec) };
    let bytes = unsafe { std::slice::from_raw_parts(v.ptr, v.len) };
    let value = std::str::from_utf8(bytes)
        .ok()
        .and_then(|text| text.trim().parse::<f64>().ok())
        .unwrap_or(0.0);
    value.to_bits()
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn __vow_format_f64_bits_in_arena(
    arena: *mut VowArena,
    bits: u64,
) -> *mut u8 {
    let text = f64::from_bits(bits).to_string();
    unsafe { alloc_bytes_string(arena, text.as_bytes()) }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn __vow_format_f64_bits(bits: u64) -> *mut u8 {
    let text = f64::from_bits(bits).to_string();
    unsafe { with_root_arena(|arena| alloc_bytes_string(arena, text.as_bytes())) }
}

// ---------------------------------------------------------------------------
// File I/O runtime
// ---------------------------------------------------------------------------

/// File contents for `fs_read`, or `None` on any error; takes no arena lock.
unsafe fn fs_read_bytes(path_ptr: *const u8) -> Option<Vec<u8>> {
    if path_ptr.is_null() {
        return None;
    }
    sanitize_on_read(path_ptr as usize, 0);
    let v = unsafe { &*(path_ptr as *const VowVec) };
    let bytes = unsafe { std::slice::from_raw_parts(v.ptr, v.len) };
    std::fs::read(std::str::from_utf8(bytes).ok()?).ok()
}

unsafe fn alloc_fs_read_result(arena: *mut VowArena, data: Option<Vec<u8>>) -> *mut u8 {
    match data {
        Some(bytes) => unsafe { alloc_bytes_string(arena, &bytes) },
        None => unsafe { __vow_vec_new_in_arena(arena, 1, 1) },
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn __vow_fs_read_in_arena(
    arena: *mut VowArena,
    path_ptr: *const u8,
) -> *mut u8 {
    unsafe { alloc_fs_read_result(arena, fs_read_bytes(path_ptr)) }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn __vow_fs_read(path_ptr: *const u8) -> *mut u8 {
    let data = unsafe { fs_read_bytes(path_ptr) };
    unsafe { with_root_arena(|arena| alloc_fs_read_result(arena, data)) }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn __vow_fs_open(path_ptr: *const u8) -> i64 {
    if path_ptr.is_null() {
        return -1;
    }
    sanitize_on_read(path_ptr as usize, 0);
    let v = unsafe { &*(path_ptr as *const VowVec) };
    let bytes = unsafe { std::slice::from_raw_parts(v.ptr, v.len) };
    let path = match std::str::from_utf8(bytes) {
        Ok(s) => s,
        Err(_) => return -1,
    };
    let file = match std::fs::File::open(path) {
        Ok(file) => file,
        Err(_) => return -1,
    };
    let handle = NEXT_FILE_READ_HANDLE.fetch_add(1, Ordering::Relaxed);
    if handle <= 0 {
        return -1;
    }
    let state = FileReadState {
        reader: std::io::BufReader::new(file),
        line_buf: Vec::new(),
        status: 0,
    };
    let mut map_guard = FILE_READ_MAP.lock().unwrap();
    let map = file_read_map_init(&mut map_guard);
    map.insert(handle, state);
    handle
}

/// Reads the next line of `handle` (empty at EOF, for an invalid handle or
/// after a read error) and copies it out so the handle table is unlocked before
/// the caller allocates.
fn read_file_line(handle: i64) -> Vec<u8> {
    use std::io::BufRead;

    let mut map_guard = FILE_READ_MAP.lock().unwrap();
    let Some(state) = map_guard.as_mut().and_then(|map| map.get_mut(&handle)) else {
        return Vec::new();
    };
    state.line_buf.clear();
    // The process-global handle table lock is intentionally held while reading;
    // docs/spec/grammar.md documents the concurrency tradeoff for this API.
    match state.reader.read_until(b'\n', &mut state.line_buf) {
        Ok(0) => {
            state.status = 1;
            Vec::new()
        }
        Ok(_) => {
            state.status = 0;
            state.line_buf.clone()
        }
        Err(_) => {
            state.status = -1;
            Vec::new()
        }
    }
}

unsafe fn alloc_bytes_string(arena: *mut VowArena, bytes: &[u8]) -> *mut u8 {
    unsafe { __vow_string_new_in_arena(arena, bytes.as_ptr() as *const c_char, bytes.len()) }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn __vow_fs_read_line_in_arena(arena: *mut VowArena, handle: i64) -> *mut u8 {
    let line = read_file_line(handle);
    unsafe { alloc_bytes_string(arena, &line) }
}

#[unsafe(no_mangle)]
pub extern "C" fn __vow_fs_read_line(handle: i64) -> *mut u8 {
    let line = read_file_line(handle);
    unsafe { with_root_arena(|arena| alloc_bytes_string(arena, &line)) }
}

#[unsafe(no_mangle)]
pub extern "C" fn __vow_fs_status(handle: i64) -> i64 {
    let map_guard = FILE_READ_MAP.lock().unwrap();
    let Some(map) = map_guard.as_ref() else {
        return -1;
    };
    match map.get(&handle) {
        Some(state) => state.status,
        None => -1,
    }
}

#[unsafe(no_mangle)]
pub extern "C" fn __vow_fs_close(handle: i64) -> i64 {
    let mut map_guard = FILE_READ_MAP.lock().unwrap();
    let Some(map) = map_guard.as_mut() else {
        return -1;
    };
    if map.remove(&handle).is_some() { 0 } else { -1 }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn __vow_fs_write(path_ptr: *const u8, data_ptr: *const u8) -> i32 {
    if path_ptr.is_null() || data_ptr.is_null() {
        return -1;
    }
    sanitize_on_read(path_ptr as usize, 0);
    sanitize_on_read(data_ptr as usize, 0);
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
    sanitize_on_read(path_ptr as usize, 0);
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
    sanitize_on_read(path_ptr as usize, 0);
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

/// Sorted entry names for `fs_listdir`; empty on any error.
unsafe fn fs_listdir_names(path_ptr: *const u8) -> Vec<String> {
    if path_ptr.is_null() {
        return Vec::new();
    }
    sanitize_on_read(path_ptr as usize, 0);
    let v = unsafe { &*(path_ptr as *const VowVec) };
    let bytes = unsafe { std::slice::from_raw_parts(v.ptr, v.len) };
    let Ok(path) = std::str::from_utf8(bytes) else {
        return Vec::new();
    };
    let Ok(entries) = std::fs::read_dir(path) else {
        return Vec::new();
    };
    let mut names: Vec<String> = entries
        .flatten()
        .map(|e| e.file_name().to_string_lossy().into_owned())
        .collect();
    names.sort();
    names
}

/// A `Vec<String>` whose elements live in the same arena as the vector.
unsafe fn alloc_string_vec(arena: *mut VowArena, items: &[String]) -> *mut u8 {
    let result = unsafe { __vow_vec_new_val_in_arena(arena) };
    for item in items {
        let str_vec = unsafe { alloc_bytes_string(arena, item.as_bytes()) } as i64;
        unsafe { __vow_vec_push_val_in_arena(arena, result, str_vec) };
    }
    result
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn __vow_fs_listdir_in_arena(
    arena: *mut VowArena,
    path_ptr: *const u8,
) -> *mut u8 {
    unsafe { alloc_string_vec(arena, &fs_listdir_names(path_ptr)) }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn __vow_fs_listdir(path_ptr: *const u8) -> *mut u8 {
    let names = unsafe { fs_listdir_names(path_ptr) };
    unsafe { with_root_arena(|arena| alloc_string_vec(arena, &names)) }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn __vow_fs_remove(path_ptr: *const u8) -> i64 {
    if path_ptr.is_null() {
        return -1;
    }
    sanitize_on_read(path_ptr as usize, 0);
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
    sanitize_on_read(path_ptr as usize, 0);
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
    sanitize_on_read(path_ptr as usize, 0);
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

// Symlink predicate. Uses `symlink_metadata` (lstat-equivalent) so that
// a symlink itself returns 1 even when its target is a regular file or
// directory — matches Rust's `DirEntry::file_type()` behaviour, which
// returns the symlink type without following. Returns 0 on any error
// (broken symlink, missing path, permission denied).
#[unsafe(no_mangle)]
pub unsafe extern "C" fn __vow_fs_is_symlink(path_ptr: *const u8) -> i64 {
    if path_ptr.is_null() {
        return 0;
    }
    sanitize_on_read(path_ptr as usize, 0);
    let v = unsafe { &*(path_ptr as *const VowVec) };
    let bytes = unsafe { std::slice::from_raw_parts(v.ptr, v.len) };
    let path = match std::str::from_utf8(bytes) {
        Ok(s) => s,
        Err(_) => return 0,
    };
    match std::fs::symlink_metadata(path) {
        Ok(md) => {
            if md.file_type().is_symlink() {
                1
            } else {
                0
            }
        }
        Err(_) => 0,
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn __vow_fs_rename(old_ptr: *const u8, new_ptr: *const u8) -> i64 {
    if old_ptr.is_null() || new_ptr.is_null() {
        return -1;
    }
    sanitize_on_read(old_ptr as usize, 0);
    sanitize_on_read(new_ptr as usize, 0);
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
        sanitize_on_read(s as usize, 0);
        let v = unsafe { &*(s as *const VowVec) };
        let bytes = unsafe { std::slice::from_raw_parts(v.ptr, v.len) };
        let _ = std::io::stderr().write_all(bytes);
        let _ = writeln!(std::io::stderr());
    }
}

/// All of stdin; blocks until EOF, so callers read it before locking an arena.
fn read_all_stdin() -> Vec<u8> {
    use std::io::Read;
    let mut buf = Vec::new();
    let _ = std::io::stdin().read_to_end(&mut buf);
    buf
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn __vow_stdin_read_in_arena(arena: *mut VowArena) -> *mut u8 {
    unsafe { alloc_bytes_string(arena, &read_all_stdin()) }
}

#[unsafe(no_mangle)]
pub extern "C" fn __vow_stdin_read() -> *mut u8 {
    let buf = read_all_stdin();
    unsafe { with_root_arena(|arena| alloc_bytes_string(arena, &buf)) }
}

#[unsafe(no_mangle)]
pub extern "C" fn __vow_stdin_read_line() -> *mut u8 {
    let stdin = std::io::stdin();
    let mut handle = stdin.lock();
    STDIN_LINE_SCRATCH.with(|cell| {
        let mut scratch = cell.borrow_mut();
        read_stdin_line_into_scratch(&mut handle, &mut scratch)
    })
}

#[unsafe(no_mangle)]
pub extern "C" fn __vow_stdin_ready() -> i64 {
    use std::os::unix::io::AsRawFd;
    let fd = std::io::stdin().as_raw_fd();
    let mut pollfd = libc::pollfd {
        fd,
        events: libc::POLLIN,
        revents: 0,
    };
    let ret = unsafe { libc::poll(&mut pollfd, 1, 0) };
    let ready_events = libc::POLLIN | libc::POLLHUP | libc::POLLERR | libc::POLLNVAL;
    if ret > 0 && (pollfd.revents & ready_events) != 0 {
        1
    } else {
        0
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn __vow_args_in_arena(arena: *mut VowArena) -> *mut u8 {
    let args: Vec<String> = std::env::args().collect();
    unsafe { alloc_string_vec(arena, &args) }
}

#[unsafe(no_mangle)]
pub extern "C" fn __vow_args() -> *mut u8 {
    let args: Vec<String> = std::env::args().collect();
    unsafe { with_root_arena(|arena| alloc_string_vec(arena, &args)) }
}

#[unsafe(no_mangle)]
pub extern "C" fn __vow_process_exit(code: i64) {
    std::process::exit(code as i32);
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn __vow_process_run(cmd_ptr: i64, args_ptr: i64) -> i64 {
    sanitize_on_read(cmd_ptr as usize, 0);
    sanitize_on_read(args_ptr as usize, 0);
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
pub unsafe extern "C" fn __vow_process_get_stdout_in_arena(arena: *mut VowArena) -> *mut u8 {
    LAST_STDOUT.with(|cell| unsafe { alloc_bytes_string(arena, &cell.borrow()) })
}

#[unsafe(no_mangle)]
pub extern "C" fn __vow_process_get_stdout() -> *mut u8 {
    LAST_STDOUT
        .with(|cell| unsafe { with_root_arena(|arena| alloc_bytes_string(arena, &cell.borrow())) })
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn __vow_process_get_stderr_in_arena(arena: *mut VowArena) -> *mut u8 {
    LAST_STDERR.with(|cell| unsafe { alloc_bytes_string(arena, &cell.borrow()) })
}

#[unsafe(no_mangle)]
pub extern "C" fn __vow_process_get_stderr() -> *mut u8 {
    LAST_STDERR
        .with(|cell| unsafe { with_root_arena(|arena| alloc_bytes_string(arena, &cell.borrow())) })
}

// ---------------------------------------------------------------------------
// Non-blocking subprocess management
// ---------------------------------------------------------------------------

#[unsafe(no_mangle)]
pub unsafe extern "C" fn __vow_process_start(cmd_ptr: i64, args_ptr: i64) -> i64 {
    sanitize_on_read(cmd_ptr as usize, 0);
    sanitize_on_read(args_ptr as usize, 0);
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

/// Copies the captured stdout (or stderr) of a finished process out of the
/// process table; empty if the handle is unknown or unfinished.
fn process_stream_bytes(handle: i64, stderr: bool) -> Vec<u8> {
    let guard = PROCESS_MAP.lock().unwrap();
    match guard.as_ref().and_then(|m| m.get(&handle)) {
        Some(ProcessState::Completed {
            stdout,
            stderr: captured_stderr,
        }) => if stderr { captured_stderr } else { stdout }.clone(),
        _ => Vec::new(),
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn __vow_process_stdout_for_in_arena(
    arena: *mut VowArena,
    handle: i64,
) -> *mut u8 {
    let bytes = process_stream_bytes(handle, false);
    unsafe { alloc_bytes_string(arena, &bytes) }
}

#[unsafe(no_mangle)]
pub extern "C" fn __vow_process_stdout_for(handle: i64) -> *mut u8 {
    let bytes = process_stream_bytes(handle, false);
    unsafe { with_root_arena(|arena| alloc_bytes_string(arena, &bytes)) }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn __vow_process_stderr_for_in_arena(
    arena: *mut VowArena,
    handle: i64,
) -> *mut u8 {
    let bytes = process_stream_bytes(handle, true);
    unsafe { alloc_bytes_string(arena, &bytes) }
}

#[unsafe(no_mangle)]
pub extern "C" fn __vow_process_stderr_for(handle: i64) -> *mut u8 {
    let bytes = process_stream_bytes(handle, true);
    unsafe { with_root_arena(|arena| alloc_bytes_string(arena, &bytes)) }
}

/// Wait for a process with a timeout in milliseconds.
/// Returns exit code on success, -2 on timeout, -1 on error.
#[unsafe(no_mangle)]
pub extern "C" fn __vow_process_wait_timeout(handle: i64, timeout_ms: i64) -> i64 {
    let mut guard = PROCESS_MAP.lock().unwrap();
    let map = process_map_init(&mut guard);
    let state = match map.remove(&handle) {
        Some(s) => s,
        None => return -1,
    };
    match state {
        ProcessState::Running(mut child) => {
            // Take stdout/stderr handles and spawn reader threads to prevent
            // pipe buffer deadlock when the child writes >64KB before exiting.
            use std::io::Read;
            let stdout_handle = child.stdout.take();
            let stderr_handle = child.stderr.take();
            let stdout_thread = std::thread::spawn(move || {
                let mut buf = Vec::new();
                if let Some(mut r) = stdout_handle {
                    let _ = r.read_to_end(&mut buf);
                }
                buf
            });
            let stderr_thread = std::thread::spawn(move || {
                let mut buf = Vec::new();
                if let Some(mut r) = stderr_handle {
                    let _ = r.read_to_end(&mut buf);
                }
                buf
            });

            // Drop the lock during polling so other process operations aren't blocked.
            drop(guard);

            let timeout = std::time::Duration::from_millis(timeout_ms.max(0) as u64);
            let start = std::time::Instant::now();
            let result = loop {
                match child.try_wait() {
                    Ok(Some(status)) => {
                        break Ok(status.code().unwrap_or(-1) as i64);
                    }
                    Ok(None) => {
                        if start.elapsed() >= timeout {
                            break Err(-2i64); // timeout
                        }
                        std::thread::sleep(std::time::Duration::from_millis(10));
                    }
                    Err(_) => {
                        break Err(-1i64); // error
                    }
                }
            };

            // Re-acquire the lock to update state.
            let mut guard = PROCESS_MAP.lock().unwrap();
            let map = process_map_init(&mut guard);

            match result {
                Ok(exit_code) => {
                    let stdout = stdout_thread.join().unwrap_or_default();
                    let stderr = stderr_thread.join().unwrap_or_default();
                    map.insert(handle, ProcessState::Completed { stdout, stderr });
                    exit_code
                }
                Err(code) => {
                    // On timeout or error, kill the child so pipes close,
                    // then join reader threads to reclaim their buffers.
                    let _ = child.kill();
                    let _ = child.wait();
                    let stdout = stdout_thread.join().unwrap_or_default();
                    let stderr = stderr_thread.join().unwrap_or_default();
                    map.insert(handle, ProcessState::Completed { stdout, stderr });
                    code
                }
            }
        }
        ProcessState::Completed { stdout, stderr } => {
            map.insert(handle, ProcessState::Completed { stdout, stderr });
            0
        }
    }
}

/// Sentinel returned by `__vow_process_poll_wait` when the child is still
/// running (left alive, not killed). Chosen well outside any real process exit
/// code or the -1/-2/-3 error sentinels, and easily representable in Vow source.
/// The self-hosted poll loop (verifier.vow) compares against this exact value.
pub const VOW_PROC_STILL_RUNNING: i64 = -999_999;

/// Non-killing bounded poll of a process, for the --perfetto tracer (issue
/// #784). Waits up to `ms` for the child; returns its exit code if it exited,
/// `VOW_PROC_STILL_RUNNING` if it is still alive (LEFT RUNNING, not killed),
/// or -1 on an unknown handle / wait error. Unlike `__vow_process_wait_timeout`
/// it never kills on timeout, so the caller can sample resources between polls
/// and re-impose its own watchdog deadline. stdout/stderr are drained by
/// persistent reader threads (POLL_READERS) so a verbose child cannot deadlock.
#[unsafe(no_mangle)]
pub extern "C" fn __vow_process_poll_wait(handle: i64, ms: i64) -> i64 {
    use std::io::Read;
    let mut guard = PROCESS_MAP.lock().unwrap();
    let map = process_map_init(&mut guard);
    let mut child = match map.remove(&handle) {
        Some(ProcessState::Running(c)) => c,
        Some(ProcessState::Completed { stdout, stderr }) => {
            map.insert(handle, ProcessState::Completed { stdout, stderr });
            return 0;
        }
        None => return -1,
    };

    // On first poll of this handle, take stdout/stderr and spawn persistent
    // drain threads so the child never blocks on a full pipe between polls.
    {
        let mut rguard = POLL_READERS.lock().unwrap();
        let readers = rguard.get_or_insert_with(HashMap::new);
        if let std::collections::hash_map::Entry::Vacant(e) = readers.entry(handle) {
            let stdout_handle = child.stdout.take();
            let stderr_handle = child.stderr.take();
            let stdout_thread = std::thread::spawn(move || {
                let mut buf = Vec::new();
                if let Some(mut r) = stdout_handle {
                    let _ = r.read_to_end(&mut buf);
                }
                buf
            });
            let stderr_thread = std::thread::spawn(move || {
                let mut buf = Vec::new();
                if let Some(mut r) = stderr_handle {
                    let _ = r.read_to_end(&mut buf);
                }
                buf
            });
            e.insert((stdout_thread, stderr_thread));
        }
    }

    // Poll without holding the process-map lock across sleeps.
    drop(guard);
    let budget = std::time::Duration::from_millis(ms.max(0) as u64);
    let start = std::time::Instant::now();
    let outcome: Result<i64, ()> = loop {
        match child.try_wait() {
            Ok(Some(status)) => break Ok(status.code().unwrap_or(-1) as i64),
            Ok(None) => {
                if start.elapsed() >= budget {
                    break Err(()); // still running
                }
                std::thread::sleep(std::time::Duration::from_millis(5));
            }
            Err(_) => break Ok(-1),
        }
    };

    let mut guard = PROCESS_MAP.lock().unwrap();
    let map = process_map_init(&mut guard);
    match outcome {
        Ok(exit_code) => {
            let readers = POLL_READERS
                .lock()
                .unwrap()
                .as_mut()
                .and_then(|m| m.remove(&handle));
            let (stdout, stderr) = match readers {
                Some((so, se)) => (so.join().unwrap_or_default(), se.join().unwrap_or_default()),
                None => (Vec::new(), Vec::new()),
            };
            map.insert(handle, ProcessState::Completed { stdout, stderr });
            exit_code
        }
        Err(()) => {
            // Still running — reinsert and report the sentinel; do NOT kill.
            map.insert(handle, ProcessState::Running(child));
            VOW_PROC_STILL_RUNNING
        }
    }
}

/// Kill a running process. Returns 0 on success, -1 on error.
#[unsafe(no_mangle)]
pub extern "C" fn __vow_process_kill(handle: i64) -> i64 {
    let mut guard = PROCESS_MAP.lock().unwrap();
    let map = process_map_init(&mut guard);
    let state = match map.remove(&handle) {
        Some(s) => s,
        None => return -1,
    };
    let rc = match state {
        ProcessState::Running(mut child) => {
            // Kill (or reap if already exited), then close pipes by dropping the
            // child so any poll-drain threads can finish.
            let _ = child.kill();
            let _ = child.wait();
            0
        }
        ProcessState::Completed { .. } => 0,
    };
    drop(guard);
    // Reclaim any poll-drain threads for this handle (issue #784) so killing a
    // polled child during the watchdog path does not leak its reader threads.
    let readers = POLL_READERS
        .lock()
        .unwrap()
        .as_mut()
        .and_then(|m| m.remove(&handle));
    if let Some((so, se)) = readers {
        let _ = so.join();
        let _ = se.join();
    }
    rc
}

// ---------------------------------------------------------------------------
// HashMap runtime (open VowVec of (key:i64, val:i64) pairs — O(n) scan MVP)
// ---------------------------------------------------------------------------

#[repr(C)]
pub struct VowMap {
    pub ptr: *mut u8,
    pub len: usize,
    pub cap: usize,
    /// Arena that owns the header and the backing buffer.
    pub owner: *mut VowArena,
}

const MAP_ENTRY_BYTES: usize = 16;
const MAP_INITIAL_CAP: usize = 8;

#[unsafe(no_mangle)]
pub unsafe extern "C" fn __vow_map_new_in_arena(arena: *mut VowArena) -> *mut u8 {
    if arena.is_null() {
        null_arena_trap("HashMap::new");
    }
    let header_ptr =
        unsafe { __vow_arena_alloc(arena, std::mem::size_of::<VowMap>(), 8) } as *mut VowMap;
    let buf_size = MAP_INITIAL_CAP * MAP_ENTRY_BYTES;
    let buf_ptr = unsafe { __vow_arena_alloc(arena, buf_size, 8) };
    unsafe { std::ptr::write_bytes(buf_ptr, 0, buf_size) };
    unsafe {
        (*header_ptr).ptr = buf_ptr;
        (*header_ptr).len = 0;
        (*header_ptr).cap = MAP_INITIAL_CAP;
        (*header_ptr).owner = arena;
    }
    header_ptr as *mut u8
}

#[unsafe(no_mangle)]
pub extern "C" fn __vow_map_new() -> *mut u8 {
    unsafe { with_root_arena(|root| __vow_map_new_in_arena(root)) }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn __vow_map_insert_in_arena(
    arena: *mut VowArena,
    map: *mut u8,
    key: i64,
    val: i64,
) {
    if arena.is_null() {
        null_arena_trap("HashMap::insert");
    }
    let m = unsafe { &mut *(map as *mut VowMap) };
    if m.cap == VOW_CAP_RODATA {
        region_literal_mutation_trap("HashMap::insert");
    }
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
        let [new_ptr] = unsafe { arena_grow_map_buffers(m.owner, [m.ptr], old_size, new_size) };
        m.ptr = new_ptr;
        m.cap = new_cap;
    }
    let entries = unsafe { std::slice::from_raw_parts_mut(m.ptr as *mut i64, (m.len + 1) * 2) };
    entries[m.len * 2] = key;
    entries[m.len * 2 + 1] = val;
    m.len += 1;
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn __vow_map_insert(map: *mut u8, key: i64, val: i64) {
    // `HashMap::insert` allocates only in the map's owner arena, so the root
    // wrapper needs no root-arena lock: it hands the entry point that owner.
    let m = unsafe { &*(map as *const VowMap) };
    if m.cap == VOW_CAP_RODATA {
        region_literal_mutation_trap("HashMap::insert");
    }
    unsafe { __vow_map_insert_in_arena(m.owner, map, key, val) }
}

/// `HashMap::get`: a fresh `Option<V>` allocated in `arena` (tag 1 and the
/// stored value when `key` is bound, tag 0 otherwise). A missing key is never
/// reported as a default value.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn __vow_map_get_in_arena(
    arena: *mut VowArena,
    map: *const u8,
    key: i64,
) -> *mut u8 {
    let m = unsafe { &*(map as *const VowMap) };
    let entries = unsafe { std::slice::from_raw_parts(m.ptr as *const i64, m.len * 2) };
    let value = (0..m.len)
        .find(|&i| entries[i * 2] == key)
        .map(|i| entries[i * 2 + 1]);
    unsafe { alloc_option_in_arena(arena, "HashMap::get", value) }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn __vow_map_get(map: *const u8, key: i64) -> *mut u8 {
    unsafe { with_root_arena(|arena| __vow_map_get_in_arena(arena, map, key)) }
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
pub unsafe extern "C" fn __vow_map_remove_in_arena(arena: *mut VowArena, map: *mut u8, key: i64) {
    if arena.is_null() {
        null_arena_trap("HashMap::remove");
    }
    // remove never allocates; the arena is accepted only for ABI symmetry
    // with __vow_map_new_in_arena and __vow_map_insert_in_arena.
    unsafe { __vow_map_remove(map, key) };
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn __vow_map_remove(map: *mut u8, key: i64) {
    let m = unsafe { &mut *(map as *mut VowMap) };
    if m.cap == VOW_CAP_RODATA {
        region_literal_mutation_trap("HashMap::remove");
    }
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
// BTreeMap runtime — sorted parallel-Vec backing (i64 keys, i64 values).
// Iteration is ascending-by-key; binary search for lookup, sorted-insert for
// writes. Like HashMap, the header and both backing buffers live in the arena
// the compiler inferred for the map; the root-arena entry points are used when
// that region is the root.
// ---------------------------------------------------------------------------

// `entries_len` is the shared logical length of both parallel arrays; both
// arrays are always grown together so `vals_cap == keys_cap` is a kept
// invariant — duplicate `vals_cap` field retained for ABI symmetry with the
// keys side and to make per-array growth tracking obvious to readers.
#[repr(C)]
pub struct VowBTreeMap {
    pub keys_ptr: *mut u8,
    pub entries_len: usize,
    pub keys_cap: usize,
    pub vals_ptr: *mut u8,
    pub vals_cap: usize,
    /// Arena that owns the header and both backing buffers.
    pub owner: *mut VowArena,
}

const BTREEMAP_INITIAL_CAP: usize = 8;
const BTREEMAP_ENTRY_BYTES: usize = 8;

#[unsafe(no_mangle)]
pub unsafe extern "C" fn __vow_btreemap_new_in_arena(arena: *mut VowArena) -> *mut u8 {
    if arena.is_null() {
        null_arena_trap("BTreeMap::new");
    }
    let header_ptr = unsafe { __vow_arena_alloc(arena, std::mem::size_of::<VowBTreeMap>(), 8) }
        as *mut VowBTreeMap;
    let buf_size = BTREEMAP_INITIAL_CAP * BTREEMAP_ENTRY_BYTES;
    let keys_buf = unsafe { __vow_arena_alloc(arena, buf_size, 8) };
    let vals_buf = unsafe { __vow_arena_alloc(arena, buf_size, 8) };
    unsafe {
        std::ptr::write_bytes(keys_buf, 0, buf_size);
        std::ptr::write_bytes(vals_buf, 0, buf_size);
        (*header_ptr).keys_ptr = keys_buf;
        (*header_ptr).entries_len = 0;
        (*header_ptr).keys_cap = BTREEMAP_INITIAL_CAP;
        (*header_ptr).vals_ptr = vals_buf;
        (*header_ptr).vals_cap = BTREEMAP_INITIAL_CAP;
        (*header_ptr).owner = arena;
    }
    header_ptr as *mut u8
}

#[unsafe(no_mangle)]
pub extern "C" fn __vow_btreemap_new() -> *mut u8 {
    unsafe { with_root_arena(|arena| __vow_btreemap_new_in_arena(arena)) }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn __vow_btreemap_len(map: *const u8) -> usize {
    let m = unsafe { &*(map as *const VowBTreeMap) };
    m.entries_len
}

// Binary-search the keys array for `key`. Returns Ok(index of equal key) or
// Err(insertion point that preserves ascending order).
fn btreemap_search(keys: &[i64], key: i64) -> Result<usize, usize> {
    let mut lo: usize = 0;
    let mut hi: usize = keys.len();
    while lo < hi {
        let mid = lo + (hi - lo) / 2;
        let mid_key = keys[mid];
        if mid_key == key {
            return Ok(mid);
        } else if mid_key < key {
            lo = mid + 1;
        } else {
            hi = mid;
        }
    }
    Err(lo)
}

/// `BTreeMap::insert`: returns a fresh `Option<V>` in `arena` holding the
/// replaced value (tag 0 when the key was new); buffers grow in the map's owner.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn __vow_btreemap_insert_in_arena(
    arena: *mut VowArena,
    map: *mut u8,
    key: i64,
    val: i64,
) -> *mut u8 {
    if arena.is_null() {
        null_arena_trap("BTreeMap::insert");
    }
    let m = unsafe { &mut *(map as *mut VowBTreeMap) };
    let keys = unsafe { std::slice::from_raw_parts(m.keys_ptr as *const i64, m.entries_len) };
    let replaced = match btreemap_search(keys, key) {
        Ok(idx) => {
            let vals =
                unsafe { std::slice::from_raw_parts_mut(m.vals_ptr as *mut i64, m.entries_len) };
            Some(std::mem::replace(&mut vals[idx], val))
        }
        Err(idx) => {
            if m.entries_len == m.keys_cap {
                let old_size = m.keys_cap * BTREEMAP_ENTRY_BYTES;
                let new_cap = m.keys_cap * 2;
                let new_size = new_cap * BTREEMAP_ENTRY_BYTES;
                [m.keys_ptr, m.vals_ptr] = unsafe {
                    arena_grow_map_buffers(m.owner, [m.keys_ptr, m.vals_ptr], old_size, new_size)
                };
                m.keys_cap = new_cap;
                m.vals_cap = new_cap;
            }
            let keys = unsafe {
                std::slice::from_raw_parts_mut(m.keys_ptr as *mut i64, m.entries_len + 1)
            };
            let vals = unsafe {
                std::slice::from_raw_parts_mut(m.vals_ptr as *mut i64, m.entries_len + 1)
            };
            // Shift right to make room at idx.
            let mut i = m.entries_len;
            while i > idx {
                keys[i] = keys[i - 1];
                vals[i] = vals[i - 1];
                i -= 1;
            }
            keys[idx] = key;
            vals[idx] = val;
            m.entries_len += 1;
            None
        }
    };
    unsafe { alloc_option_in_arena(arena, "BTreeMap::insert", replaced) }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn __vow_btreemap_insert(map: *mut u8, key: i64, val: i64) -> *mut u8 {
    unsafe { with_root_arena(|arena| __vow_btreemap_insert_in_arena(arena, map, key, val)) }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn __vow_btreemap_get_in_arena(
    arena: *mut VowArena,
    map: *const u8,
    key: i64,
) -> *mut u8 {
    let m = unsafe { &*(map as *const VowBTreeMap) };
    let keys = unsafe { std::slice::from_raw_parts(m.keys_ptr as *const i64, m.entries_len) };
    let value = btreemap_search(keys, key).ok().map(|idx| {
        let vals = unsafe { std::slice::from_raw_parts(m.vals_ptr as *const i64, m.entries_len) };
        vals[idx]
    });
    unsafe { alloc_option_in_arena(arena, "BTreeMap::get", value) }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn __vow_btreemap_get(map: *const u8, key: i64) -> *mut u8 {
    unsafe { with_root_arena(|arena| __vow_btreemap_get_in_arena(arena, map, key)) }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn __vow_btreemap_contains(map: *const u8, key: i64) -> bool {
    let m = unsafe { &*(map as *const VowBTreeMap) };
    let keys = unsafe { std::slice::from_raw_parts(m.keys_ptr as *const i64, m.entries_len) };
    btreemap_search(keys, key).is_ok()
}

// ---------------------------------------------------------------------------
// Sanitize mode — Vec provenance tracking
// ---------------------------------------------------------------------------

static SANITIZE_ENABLED: AtomicBool = AtomicBool::new(false);
static SANITIZE_GLOBAL_GEN: AtomicU64 = AtomicU64::new(1);

struct ShadowVec {
    generations: Vec<u64>,
    freed: bool,
}

static SHADOW_TABLE: Mutex<Option<HashMap<usize, ShadowVec>>> = Mutex::new(None);

fn shadow_table_get_or_init(
    table: &mut Option<HashMap<usize, ShadowVec>>,
) -> &mut HashMap<usize, ShadowVec> {
    table.get_or_insert_with(HashMap::new)
}

#[unsafe(no_mangle)]
pub extern "C" fn __vow_sanitize_init() {
    SANITIZE_ENABLED.store(true, Ordering::SeqCst);
    let mut table = SHADOW_TABLE.lock().unwrap();
    *table = Some(HashMap::new());
}

fn sanitize_is_enabled() -> bool {
    SANITIZE_ENABLED.load(Ordering::Relaxed)
}

fn sanitize_emit_error(error_type: &str, details: &str) {
    let _ = writeln!(std::io::stderr(), r#"{{"error":"{error_type}",{details}}}"#);
    let _ = writeln!(std::io::stderr(), "sanitizer: {error_type}: {details}");
    std::process::exit(VOW_RUNTIME_ABORT_EXIT);
}

fn sanitize_on_vec_new(vec_addr: usize) {
    if !sanitize_is_enabled() {
        return;
    }
    let mut table = SHADOW_TABLE.lock().unwrap();
    let map = shadow_table_get_or_init(&mut table);
    map.insert(
        vec_addr,
        ShadowVec {
            generations: Vec::new(),
            freed: false,
        },
    );
}

fn sanitize_on_push(vec_addr: usize) {
    if !sanitize_is_enabled() {
        return;
    }
    let generation = SANITIZE_GLOBAL_GEN.fetch_add(1, Ordering::Relaxed);
    let mut table = SHADOW_TABLE.lock().unwrap();
    let map = shadow_table_get_or_init(&mut table);
    if let Some(shadow) = map.get_mut(&vec_addr) {
        if shadow.freed {
            sanitize_emit_error(
                "UseAfterFree",
                &format!("\"op\":\"push\",\"vec\":\"0x{vec_addr:x}\""),
            );
        }
        shadow.generations.push(generation);
    }
}

fn sanitize_on_set(vec_addr: usize, index: usize) {
    if !sanitize_is_enabled() {
        return;
    }
    let generation = SANITIZE_GLOBAL_GEN.fetch_add(1, Ordering::Relaxed);
    let mut table = SHADOW_TABLE.lock().unwrap();
    let map = shadow_table_get_or_init(&mut table);
    if let Some(shadow) = map.get_mut(&vec_addr) {
        if shadow.freed {
            sanitize_emit_error(
                "UseAfterFree",
                &format!("\"op\":\"set\",\"vec\":\"0x{vec_addr:x}\""),
            );
        }
        if index < shadow.generations.len() {
            shadow.generations[index] = generation;
        }
    }
}

fn sanitize_on_truncate(vec_addr: usize, new_len: usize) {
    if !sanitize_is_enabled() {
        return;
    }
    let mut table = SHADOW_TABLE.lock().unwrap();
    let map = shadow_table_get_or_init(&mut table);
    if let Some(shadow) = map.get_mut(&vec_addr) {
        if shadow.freed {
            sanitize_emit_error(
                "UseAfterFree",
                &format!("\"op\":\"truncate\",\"vec\":\"0x{vec_addr:x}\""),
            );
        }
        shadow.generations.truncate(new_len);
    }
}

fn sanitize_on_clear(vec_addr: usize) {
    if !sanitize_is_enabled() {
        return;
    }
    let mut table = SHADOW_TABLE.lock().unwrap();
    let map = shadow_table_get_or_init(&mut table);
    if let Some(shadow) = map.get_mut(&vec_addr) {
        if shadow.freed {
            sanitize_emit_error(
                "UseAfterFree",
                &format!("\"op\":\"clear\",\"vec\":\"0x{vec_addr:x}\""),
            );
        }
        shadow.generations.clear();
    }
}

fn sanitize_on_pop(vec_addr: usize) {
    if !sanitize_is_enabled() {
        return;
    }
    let mut table = SHADOW_TABLE.lock().unwrap();
    let map = shadow_table_get_or_init(&mut table);
    if let Some(shadow) = map.get_mut(&vec_addr) {
        if shadow.freed {
            sanitize_emit_error(
                "UseAfterFree",
                &format!("\"op\":\"pop\",\"vec\":\"0x{vec_addr:x}\""),
            );
        }
        shadow.generations.pop();
    }
}

fn sanitize_on_read(vec_addr: usize, _index: usize) {
    if !sanitize_is_enabled() {
        return;
    }
    let table = SHADOW_TABLE.lock().unwrap();
    if let Some(map) = table.as_ref()
        && let Some(shadow) = map.get(&vec_addr)
        && shadow.freed
    {
        sanitize_emit_error(
            "UseAfterFree",
            &format!("\"op\":\"read\",\"vec\":\"0x{vec_addr:x}\""),
        );
    }
}

/// Query the generation of a Vec slot. Returns 0 if unknown.
#[unsafe(no_mangle)]
pub extern "C" fn __vow_sanitize_vec_generation(vec: *const u8, index: usize) -> u64 {
    if !sanitize_is_enabled() || vec.is_null() {
        return 0;
    }
    let vec_addr = vec as usize;
    let table = SHADOW_TABLE.lock().unwrap();
    if let Some(map) = table.as_ref()
        && let Some(shadow) = map.get(&vec_addr)
        && index < shadow.generations.len()
    {
        return shadow.generations[index];
    }
    0
}

/// Check that a Vec slot's generation matches the expected value.
/// Aborts with StaleIndex error if it doesn't match.
#[unsafe(no_mangle)]
pub extern "C" fn __vow_sanitize_check_generation(vec: *const u8, index: usize, expected_gen: u64) {
    if !sanitize_is_enabled() || vec.is_null() {
        return;
    }
    let actual = __vow_sanitize_vec_generation(vec, index);
    if actual != expected_gen && expected_gen != 0 {
        sanitize_emit_error(
            "StaleIndex",
            &format!(
                "\"index\":{index},\"expected_gen\":{expected_gen},\"actual_gen\":{actual},\"vec\":\"0x{:x}\"",
                vec as usize
            ),
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The 128-bit division helpers must agree with native Rust `i128`/`u128`
    /// arithmetic across both limbs, including the sign rules that differ
    /// between the signed and unsigned symbol.
    #[test]
    fn wide_division_helpers_match_native_arithmetic() {
        for (a, b) in [
            (10i128, 3i128),
            (-7, 3),
            (7, -3),
            (-7, -3),
            (i128::MAX, 5),
            (i128::MIN, 5),
            (3154393236604333326336, 3),
        ] {
            assert_eq!(__vow_i128_div(a, b), a / b, "{a} / {b}");
            assert_eq!(__vow_i128_rem(a, b), a % b, "{a} % {b}");
        }
        for (a, b) in [
            (10u128, 3u128),
            (u128::MAX, 5),
            (u128::MAX, 7),
            (3781582535110458081280, 5),
        ] {
            assert_eq!(__vow_u128_div(a, b), a / b, "{a} / {b}");
            assert_eq!(__vow_u128_rem(a, b), a % b, "{a} % {b}");
        }
    }

    /// The zero guards are unreachable in a compiled program — both backends
    /// trap before the call — but they must abort rather than let a Rust
    /// divide-by-zero panic cross the `extern "C"` boundary. Spawned as
    /// subprocesses because the guard calls `std::process::exit`.
    #[test]
    fn wide_division_helpers_abort_on_a_zero_divisor() {
        for (helper, arg) in [
            ("i128_div", "0"),
            ("i128_rem", "0"),
            ("u128_div", "0"),
            ("u128_rem", "0"),
        ] {
            let exe = std::env::current_exe().expect("test binary path");
            let output = std::process::Command::new(exe)
                .args(["--exact", "tests::wide_zero_divisor_child", "--nocapture"])
                .env("VOW_WIDE_ZERO_HELPER", helper)
                .env("VOW_WIDE_ZERO_ARG", arg)
                .output()
                .expect("spawn child");
            assert_eq!(
                output.status.code(),
                Some(VOW_RUNTIME_ABORT_EXIT),
                "{helper}: expected the runtime abort exit code"
            );
            let stderr = String::from_utf8_lossy(&output.stderr);
            assert!(
                stderr.contains("ArithmeticOverflow"),
                "{helper}: expected the ArithmeticOverflow envelope, got: {stderr}"
            );
        }
    }

    /// Child half of `wide_division_helpers_abort_on_a_zero_divisor`. Inert
    /// unless the parent sets `VOW_WIDE_ZERO_HELPER`, so a normal test run
    /// executes it as a no-op.
    #[test]
    fn wide_zero_divisor_child() {
        let Ok(helper) = std::env::var("VOW_WIDE_ZERO_HELPER") else {
            return;
        };
        match helper.as_str() {
            "i128_div" => {
                __vow_i128_div(10, 0);
            }
            "i128_rem" => {
                __vow_i128_rem(10, 0);
            }
            "u128_div" => {
                __vow_u128_div(10, 0);
            }
            "u128_rem" => {
                __vow_u128_rem(10, 0);
            }
            other => panic!("unknown helper {other}"),
        }
        unreachable!("the zero guard must have exited the process");
    }

    /// `MIN / -1` and `MIN % -1` are the two cases native `/` and `%` would
    /// panic on. The backends trap on the division before the call, so these
    /// wrap rather than panic across the `extern "C"` boundary; `MIN % -1` is
    /// 0, which is also what `srem` gives at the narrower widths.
    #[test]
    fn wide_division_helpers_are_total_at_the_signed_extreme() {
        assert_eq!(__vow_i128_div(i128::MIN, -1), i128::MIN);
        assert_eq!(__vow_i128_rem(i128::MIN, -1), 0);
    }

    #[test]
    fn wide_multiply_overflow_helpers_match_checked_mul() {
        for (a, b) in [(0i128, 0i128), (10, 3), (-10, 3), (i128::MAX, 1)] {
            assert_eq!(__vow_i128_mul_overflow(a, b), 0, "{a} * {b} fits");
        }
        for (a, b) in [(i128::MAX, 2i128), (i128::MIN, 2), (i128::MIN, -1)] {
            assert_eq!(__vow_i128_mul_overflow(a, b), 1, "{a} * {b} overflows");
        }
        assert_eq!(__vow_u128_mul_overflow(10, 3), 0);
        assert_eq!(__vow_u128_mul_overflow(u128::MAX, 1), 0);
        assert_eq!(__vow_u128_mul_overflow(u128::MAX, 2), 1);
    }

    fn borrowed_vow_string(text: &str) -> VowVec {
        VowVec {
            ptr: text.as_ptr() as *mut u8,
            len: text.len(),
            cap: text.len(),
        }
    }

    unsafe fn option_parts(ptr: *const u8) -> (i64, i64) {
        let words = ptr as *const i64;
        unsafe { (*words, *words.add(1)) }
    }

    #[test]
    fn narrow_parsers_accept_bounds_and_reject_out_of_range_values() {
        type ParseFn = unsafe extern "C" fn(*const u8) -> *mut u8;
        let cases: [(ParseFn, &str, i64, &str); 4] = [
            (__vow_string_parse_i8_opt, "-128", -128, "128"),
            (__vow_string_parse_i16_opt, "-32768", -32768, "32768"),
            (__vow_string_parse_u16_opt, "65535", 65535, "65536"),
            (
                __vow_string_parse_u32_opt,
                "4294967295",
                4294967295,
                "4294967296",
            ),
        ];

        for (parse, valid, expected, invalid) in cases {
            let valid = borrowed_vow_string(valid);
            assert_eq!(
                unsafe { option_parts(parse(&raw const valid as *const u8)) },
                (1, expected)
            );

            let invalid = borrowed_vow_string(invalid);
            assert_eq!(
                unsafe { option_parts(parse(&raw const invalid as *const u8)) },
                (0, 0)
            );
        }

        assert_eq!(
            unsafe { option_parts(__vow_string_parse_i8_opt(std::ptr::null())) },
            (0, 0)
        );
    }

    #[test]
    fn narrow_conversions_distinguish_try_wrap_and_saturate() {
        assert_eq!(unsafe { option_parts(__vow_i16_to_i8_try(127)) }, (1, 127));
        assert_eq!(unsafe { option_parts(__vow_i16_to_i8_try(128)) }, (0, 0));
        assert_eq!(unsafe { option_parts(__vow_i128_to_u8_try(255)) }, (1, 255));
        assert_eq!(__vow_i16_to_i8_wrap(130), -126);
        assert_eq!(__vow_i16_to_i8_sat(128), i8::MAX);
        assert_eq!(__vow_i16_to_i8_sat(-129), i8::MIN);

        assert_eq!(unsafe { option_parts(__vow_u16_to_i8_try(127)) }, (1, 127));
        assert_eq!(unsafe { option_parts(__vow_u16_to_i8_try(128)) }, (0, 0));
        assert_eq!(__vow_u16_to_i8_wrap(255), -1);
        assert_eq!(__vow_u16_to_i8_sat(128), i8::MAX);

        assert_eq!(
            unsafe { option_parts(__vow_i32_to_u16_try(65535)) },
            (1, 65535)
        );
        assert_eq!(unsafe { option_parts(__vow_i32_to_u16_try(-1)) }, (0, 0));
        assert_eq!(__vow_i32_to_u16_wrap(-1), u16::MAX);
        assert_eq!(__vow_i32_to_u16_sat(-1), 0);
        assert_eq!(__vow_i32_to_u16_sat(70000), u16::MAX);

        assert_eq!(
            unsafe { option_parts(__vow_u64_to_u32_try(u64::from(u32::MAX))) },
            (1, i64::from(u32::MAX))
        );
        assert_eq!(
            unsafe { option_parts(__vow_u64_to_u32_try(u64::from(u32::MAX) + 1)) },
            (0, 0)
        );
        assert_eq!(__vow_u64_to_u32_wrap(u64::from(u32::MAX) + 1), 0);
        assert_eq!(__vow_u64_to_u32_sat(u64::from(u32::MAX) + 1), u32::MAX);
    }

    /// The seam-5a (issue #1060) widening of the narrowing matrix: i128/u128
    /// sources into every sub-64-bit target, plus the same-width sign-change
    /// pairs (wrap/sat only — no try exists for 128-bit option payloads).
    /// Every helper is exercised in its fitting, overflowing, and saturating
    /// regimes so the invocation sites read as covered.
    #[test]
    fn wide_source_narrowing_distinguishes_try_wrap_and_saturate() {
        // i128 sources, signed targets.
        assert_eq!(unsafe { option_parts(__vow_i128_to_i8_try(127)) }, (1, 127));
        assert_eq!(
            unsafe { option_parts(__vow_i128_to_i8_try(-128)) },
            (1, -128)
        );
        assert_eq!(unsafe { option_parts(__vow_i128_to_i8_try(128)) }, (0, 0));
        assert_eq!(unsafe { option_parts(__vow_i128_to_i8_try(-129)) }, (0, 0));
        assert_eq!(__vow_i128_to_i8_wrap(130), -126);
        assert_eq!(__vow_i128_to_i8_sat(200), i8::MAX);
        assert_eq!(__vow_i128_to_i8_sat(-200), i8::MIN);

        assert_eq!(
            unsafe { option_parts(__vow_i128_to_i16_try(32767)) },
            (1, 32767)
        );
        assert_eq!(
            unsafe { option_parts(__vow_i128_to_i16_try(32768)) },
            (0, 0)
        );
        assert_eq!(
            unsafe { option_parts(__vow_i128_to_i16_try(-32769)) },
            (0, 0)
        );
        assert_eq!(__vow_i128_to_i16_wrap(32769), -32767);
        assert_eq!(__vow_i128_to_i16_sat(40000), i16::MAX);
        assert_eq!(__vow_i128_to_i16_sat(-40000), i16::MIN);

        assert_eq!(
            unsafe { option_parts(__vow_i128_to_i32_try(2147483647)) },
            (1, 2147483647)
        );
        assert_eq!(
            unsafe { option_parts(__vow_i128_to_i32_try(2147483648)) },
            (0, 0)
        );
        assert_eq!(
            unsafe { option_parts(__vow_i128_to_i32_try(-2147483649)) },
            (0, 0)
        );
        assert_eq!(__vow_i128_to_i32_wrap(2147483649), -2147483647);
        assert_eq!(__vow_i128_to_i32_sat(5000000000), i32::MAX);
        assert_eq!(__vow_i128_to_i32_sat(-5000000000), i32::MIN);

        // i128 sources, unsigned targets.
        assert_eq!(
            unsafe { option_parts(__vow_i128_to_u16_try(65535)) },
            (1, 65535)
        );
        assert_eq!(unsafe { option_parts(__vow_i128_to_u16_try(-1)) }, (0, 0));
        assert_eq!(
            unsafe { option_parts(__vow_i128_to_u16_try(65536)) },
            (0, 0)
        );
        assert_eq!(__vow_i128_to_u16_wrap(-1), u16::MAX);
        assert_eq!(__vow_i128_to_u16_sat(-1), 0);
        assert_eq!(__vow_i128_to_u16_sat(70000), u16::MAX);

        assert_eq!(
            unsafe { option_parts(__vow_i128_to_u32_try(4294967295)) },
            (1, 4294967295)
        );
        assert_eq!(unsafe { option_parts(__vow_i128_to_u32_try(-1)) }, (0, 0));
        assert_eq!(
            unsafe { option_parts(__vow_i128_to_u32_try(4294967296)) },
            (0, 0)
        );
        assert_eq!(__vow_i128_to_u32_wrap(-1), u32::MAX);
        assert_eq!(__vow_i128_to_u32_sat(-1), 0);
        assert_eq!(__vow_i128_to_u32_sat(5000000000), u32::MAX);

        // u128 sources.
        assert_eq!(unsafe { option_parts(__vow_u128_to_i8_try(127)) }, (1, 127));
        assert_eq!(unsafe { option_parts(__vow_u128_to_i8_try(128)) }, (0, 0));
        assert_eq!(__vow_u128_to_i8_wrap(255), -1);
        assert_eq!(__vow_u128_to_i8_sat(300), i8::MAX);

        assert_eq!(
            unsafe { option_parts(__vow_u128_to_i16_try(32767)) },
            (1, 32767)
        );
        assert_eq!(
            unsafe { option_parts(__vow_u128_to_i16_try(32768)) },
            (0, 0)
        );
        assert_eq!(__vow_u128_to_i16_wrap(65535), -1);
        assert_eq!(__vow_u128_to_i16_sat(40000), i16::MAX);

        assert_eq!(
            unsafe { option_parts(__vow_u128_to_i32_try(2147483647)) },
            (1, 2147483647)
        );
        assert_eq!(
            unsafe { option_parts(__vow_u128_to_i32_try(2147483648)) },
            (0, 0)
        );
        assert_eq!(__vow_u128_to_i32_wrap(4294967295), -1);
        assert_eq!(__vow_u128_to_i32_sat(5000000000), i32::MAX);

        assert_eq!(
            unsafe { option_parts(__vow_u128_to_u16_try(65535)) },
            (1, 65535)
        );
        assert_eq!(
            unsafe { option_parts(__vow_u128_to_u16_try(65536)) },
            (0, 0)
        );
        assert_eq!(__vow_u128_to_u16_wrap(65536), 0);
        assert_eq!(__vow_u128_to_u16_sat(70000), u16::MAX);

        assert_eq!(
            unsafe { option_parts(__vow_u128_to_u32_try(4294967295)) },
            (1, 4294967295)
        );
        assert_eq!(
            unsafe { option_parts(__vow_u128_to_u32_try(4294967296)) },
            (0, 0)
        );
        assert_eq!(__vow_u128_to_u32_wrap(4294967296), 0);
        assert_eq!(__vow_u128_to_u32_sat(5000000000), u32::MAX);

        // Same-width sign-change pairs expose wrap/sat only — a 128-bit option
        // payload is unrepresentable, so no _try variant exists.
        assert_eq!(__vow_u128_to_i128_wrap(u128::MAX), -1);
        assert_eq!(__vow_u128_to_i128_sat(i128::MAX as u128 + 1), i128::MAX);

        assert_eq!(__vow_i128_to_u128_wrap(-1), u128::MAX);
        assert_eq!(__vow_i128_to_u128_sat(-1), 0);
        assert_eq!(__vow_i128_to_u128_sat(5), 5);
    }

    fn vec_sort_cost(len: usize) -> u64 {
        let vec = VowVec {
            ptr: std::ptr::dangling_mut(),
            len,
            cap: len,
        };
        __vow_perf_counter_reset();
        unsafe { __vow_perf_count_vec_sort(&raw const vec as *const u8) };
        __vow_perf_counter_read()
    }

    // Every counter case shares the process-global PERF_OPERATION_COUNT, so they
    // stay in one test rather than racing across cargo's parallel test threads.
    #[test]
    fn performance_operation_counter_attributes_helper_work_and_saturates() {
        __vow_perf_counter_reset();
        __vow_perf_count();
        __vow_perf_count();
        assert_eq!(__vow_perf_counter_read(), 2);

        __vow_perf_counter_reset();
        unsafe { __vow_perf_count_vec_sort(std::ptr::null()) };
        assert_eq!(
            __vow_perf_counter_read(),
            1,
            "a null helper call still has its caller-side operation cost"
        );

        assert_eq!(vec_sort_cost(0), 1);
        assert_eq!(vec_sort_cost(1), 3);
        assert_eq!(vec_sort_cost(4), 17);
        assert_eq!(vec_sort_cost(8), 41);
        // Powers of two alone cannot tell ceil(log2 n) from floor(log2 n).
        // n = 5 charges 5 * (3 + 2) + 1, not the floor model's 5 * (2 + 2) + 1.
        assert_eq!(vec_sort_cost(5), 26);

        let map = VowMap {
            len: 4,
            cap: 4,
            ..make_rodata_map()
        };
        let map_ptr = &raw const map as *const u8;
        __vow_perf_counter_reset();
        unsafe { __vow_perf_count_map_contains(map_ptr, -1) };
        assert_eq!(__vow_perf_counter_read(), 5);
        __vow_perf_counter_reset();
        unsafe { __vow_perf_count_map_get(map_ptr, -1) };
        assert_eq!(__vow_perf_counter_read(), 5);
        __vow_perf_counter_reset();
        unsafe { __vow_perf_count_map_remove(map_ptr, -1) };
        assert_eq!(__vow_perf_counter_read(), 5);
        __vow_perf_counter_reset();
        unsafe { __vow_perf_count_map_insert(map_ptr, -1, 0) };
        assert_eq!(
            __vow_perf_counter_read(),
            9,
            "scan plus possible buffer copy"
        );

        let a = borrowed_vow_string("abcd");
        let b = borrowed_vow_string("abcde");
        __vow_perf_counter_reset();
        unsafe { __vow_perf_count_string_eq(&raw const a as *const u8, &raw const a as *const u8) };
        assert_eq!(__vow_perf_counter_read(), 5);
        __vow_perf_counter_reset();
        unsafe { __vow_perf_count_string_eq(&raw const a as *const u8, &raw const b as *const u8) };
        assert_eq!(
            __vow_perf_counter_read(),
            1,
            "unequal lengths return before byte scan"
        );

        #[cfg(target_pointer_width = "64")]
        {
            assert_eq!(vec_sort_cost(usize::MAX), u64::MAX);
            __vow_perf_count();
            assert_eq!(__vow_perf_counter_read(), u64::MAX);

            let huge_map = VowMap {
                len: usize::MAX,
                ..map
            };
            __vow_perf_counter_reset();
            unsafe { __vow_perf_count_map_insert(&raw const huge_map as *const u8, 0, 0) };
            assert_eq!(__vow_perf_counter_read(), u64::MAX);
        }

        __vow_perf_counter_reset();
        assert_eq!(__vow_perf_counter_read(), 0);
    }

    #[test]
    fn collect_proc_samples_reports_compiler_self() {
        let procs = vec![ProcInfo {
            pid: 1,
            parent: None,
            name: "vowc".into(),
            rss_kb: 1000.0,
            cpu_pct: 10.0,
        }];
        let samples = collect_proc_samples(&procs, 1);
        assert_eq!(samples.len(), 1);
        assert_eq!(samples[0].group, "compiler");
        assert_eq!(samples[0].pid, 1);
        assert_eq!(samples[0].rss_kb, 1000.0);
        assert_eq!(samples[0].cpu_pct, 10.0);
    }

    #[test]
    fn collect_proc_samples_sums_esbmc_subtree_and_filters() {
        let procs = vec![
            ProcInfo {
                pid: 1,
                parent: None,
                name: "vowc".into(),
                rss_kb: 1000.0,
                cpu_pct: 5.0,
            },
            // ESBMC child of ours.
            ProcInfo {
                pid: 2,
                parent: Some(1),
                name: "esbmc".into(),
                rss_kb: 2000.0,
                cpu_pct: 20.0,
            },
            // Solver grandchild — must fold into the esbmc:2 group.
            ProcInfo {
                pid: 3,
                parent: Some(2),
                name: "z3".into(),
                rss_kb: 5000.0,
                cpu_pct: 50.0,
            },
            // Linker child of ours — excluded by name.
            ProcInfo {
                pid: 4,
                parent: Some(1),
                name: "cc".into(),
                rss_kb: 800.0,
                cpu_pct: 1.0,
            },
            // Unrelated system esbmc (different parent) — excluded.
            ProcInfo {
                pid: 5,
                parent: Some(99),
                name: "esbmc".into(),
                rss_kb: 9999.0,
                cpu_pct: 9.0,
            },
        ];
        let samples = collect_proc_samples(&procs, 1);

        let esbmc: Vec<&ProcSample> = samples
            .iter()
            .filter(|s| s.group.starts_with("esbmc"))
            .collect();
        assert_eq!(esbmc.len(), 1, "exactly one esbmc group");
        assert_eq!(esbmc[0].pid, 2);
        assert_eq!(esbmc[0].rss_kb, 7000.0, "esbmc + z3 subtree summed");
        assert_eq!(esbmc[0].cpu_pct, 70.0);
        // Linker (pid 4) and unrelated esbmc (pid 5) never appear as groups.
        assert!(samples.iter().all(|s| s.pid != 4 && s.pid != 5));
    }

    #[test]
    fn process_poll_wait_captures_stdout() {
        // Polling a process to completion must still capture its stdout (drained
        // by the poll reader threads) so verification output is not lost.
        let child = std::process::Command::new("sh")
            .arg("-c")
            .arg("printf 'VERIFICATION SUCCESSFUL'; sleep 0.05")
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped())
            .spawn()
            .expect("spawn sh");
        let handle = 999_002;
        {
            let mut g = PROCESS_MAP.lock().unwrap();
            let m = process_map_init(&mut g);
            m.insert(handle, ProcessState::Running(child));
        }
        let mut code = VOW_PROC_STILL_RUNNING;
        for _ in 0..400 {
            code = __vow_process_poll_wait(handle, 20);
            if code != VOW_PROC_STILL_RUNNING {
                break;
            }
        }
        assert_eq!(code, 0, "sh exits 0");
        let out_ptr = __vow_process_stdout_for(handle);
        let v = unsafe { &*(out_ptr as *const VowVec) };
        let bytes = unsafe { std::slice::from_raw_parts(v.ptr, v.len) };
        let s = std::str::from_utf8(bytes).unwrap_or("");
        assert!(
            s.contains("VERIFICATION SUCCESSFUL"),
            "captured stdout was: {s:?}"
        );
    }

    #[test]
    fn process_poll_wait_does_not_kill_running_child() {
        // Insert a real sleeping child directly into the process map.
        let child = std::process::Command::new("sleep")
            .arg("0.4")
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped())
            .spawn()
            .expect("spawn sleep");
        let handle = 999_001;
        {
            let mut g = PROCESS_MAP.lock().unwrap();
            let m = process_map_init(&mut g);
            m.insert(handle, ProcessState::Running(child));
        }
        // First poll: child should still be running and NOT killed.
        let r1 = __vow_process_poll_wait(handle, 10);
        assert_eq!(
            r1, VOW_PROC_STILL_RUNNING,
            "still-running sentinel, child left alive"
        );
        // Keep polling until it exits; it must exit 0 (was never killed early).
        let mut code = VOW_PROC_STILL_RUNNING;
        for _ in 0..200 {
            code = __vow_process_poll_wait(handle, 20);
            if code != VOW_PROC_STILL_RUNNING {
                break;
            }
        }
        assert_eq!(code, 0, "sleep exits 0 — poll_wait did not kill it");
    }

    #[test]
    fn time_micros_is_monotonic() {
        let a = __vow_time_micros();
        let b = __vow_time_micros();
        assert!(b >= a, "monotonic, non-decreasing");
        std::thread::sleep(std::time::Duration::from_millis(2));
        let c = __vow_time_micros();
        assert!(c > a, "advances after a sleep");
    }

    #[test]
    fn gzip_write_file_roundtrips() {
        use std::io::Read;
        let dir = std::env::temp_dir().join(format!("vow_gz_test_{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("t.json.gz");
        let payload = r#"{"traceEvents":[{"ph":"X","name":"parse"}],"displayTimeUnit":"ms"}"#;

        let path_s = path.to_string_lossy().into_owned();
        let path_v = unsafe { __vow_string_new(path_s.as_ptr() as *const c_char, path_s.len()) };
        let data_v = unsafe { __vow_string_new(payload.as_ptr() as *const c_char, payload.len()) };
        let rc = unsafe { __vow_gzip_write_file(path_v, data_v) };
        assert_eq!(rc, 0, "gzip write should succeed");

        // Decode the file back and confirm it matches the original payload.
        let f = std::fs::File::open(&path).unwrap();
        let mut gz = flate2::read::GzDecoder::new(f);
        let mut decoded = String::new();
        gz.read_to_string(&mut decoded).unwrap();
        assert_eq!(decoded, payload);

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn malloc_free_roundtrip() {
        let ptr = __vow_malloc(64, 8);
        assert!(!ptr.is_null());
        unsafe { __vow_free(ptr, 64, 8) };
    }

    #[test]
    fn free_null_is_noop() {
        unsafe { __vow_free(std::ptr::null_mut(), 64, 8) };
    }

    #[test]
    fn free_zero_size_is_noop() {
        unsafe { __vow_free(0x8 as *mut u8, 0, 8) };
    }

    #[test]
    fn malloc_zero_returns_sentinel() {
        let ptr = __vow_malloc(0, 8);
        assert_eq!(ptr, 8 as *mut u8);
    }

    #[test]
    fn vec_new_lazy_allocation() {
        let v = __vow_vec_new_val();
        let vec = unsafe { &*(v as *const VowVec) };
        assert_eq!(vec.len, 0);
        assert_eq!(
            vow_vec_capacity(vec),
            0,
            "empty Vec should have cap=0 (lazy)"
        );
    }

    #[test]
    fn vec_first_push_allocates() {
        let v = __vow_vec_new_val();
        unsafe { __vow_vec_push_val(v, 42) };
        let vec = unsafe { &*(v as *const VowVec) };
        assert_eq!(vec.len, 1);
        assert!(
            vow_vec_capacity(vec) >= 1,
            "cap should be allocated after first push"
        );
        assert_eq!(unsafe { __vow_vec_get_val(v, 0) }, 42);
    }

    #[test]
    fn string_new_empty_lazy() {
        let s = __vow_vec_new(1, 1);
        let vec = unsafe { &*(s as *const VowVec) };
        assert_eq!(vec.len, 0);
        assert_eq!(
            vow_vec_capacity(vec),
            0,
            "empty String should have cap=0 (lazy)"
        );
    }

    #[test]
    fn string_from_empty_lazy() {
        let s = unsafe { __vow_string_new(std::ptr::null(), 0) };
        let vec = unsafe { &*(s as *const VowVec) };
        assert_eq!(vec.len, 0);
        assert_eq!(
            vow_vec_capacity(vec),
            0,
            "String::from(\"\") should have cap=0 (lazy)"
        );
    }

    #[test]
    fn string_from_nonempty_allocates() {
        let data = b"hello";
        let s = unsafe { __vow_string_new(data.as_ptr() as *const c_char, 5) };
        let vec = unsafe { &*(s as *const VowVec) };
        assert_eq!(vec.len, 5);
        assert!(vow_vec_capacity(vec) >= 5);
    }

    #[test]
    fn vec_multiple_push_after_lazy() {
        let v = __vow_vec_new_val();
        for i in 0..20 {
            unsafe { __vow_vec_push_val(v, i) };
        }
        let vec = unsafe { &*(v as *const VowVec) };
        assert_eq!(vec.len, 20);
        assert!(vow_vec_capacity(vec) >= 20);
        for i in 0..20 {
            assert_eq!(unsafe { __vow_vec_get_val(v, i as usize) }, i);
        }
    }

    #[test]
    fn vec_pop_basic() {
        let v = __vow_vec_new_val();
        unsafe { __vow_vec_push_val(v, 10) };
        unsafe { __vow_vec_push_val(v, 20) };
        unsafe { __vow_vec_push_val(v, 30) };
        assert_eq!(unsafe { &*(v as *const VowVec) }.len, 3);
        unsafe { __vow_vec_pop(v) };
        assert_eq!(unsafe { &*(v as *const VowVec) }.len, 2);
        assert_eq!(unsafe { __vow_vec_get_val(v, 0) }, 10);
        assert_eq!(unsafe { __vow_vec_get_val(v, 1) }, 20);
    }

    #[test]
    fn vec_pop_empty_is_noop() {
        let v = __vow_vec_new_val();
        unsafe { __vow_vec_pop(v) };
        assert_eq!(unsafe { &*(v as *const VowVec) }.len, 0);
    }

    #[test]
    fn vec_pop_to_empty() {
        let v = __vow_vec_new_val();
        unsafe { __vow_vec_push_val(v, 42) };
        unsafe { __vow_vec_pop(v) };
        assert_eq!(unsafe { &*(v as *const VowVec) }.len, 0);
    }

    #[test]
    fn vec_pop_then_push() {
        let v = __vow_vec_new_val();
        unsafe { __vow_vec_push_val(v, 1) };
        unsafe { __vow_vec_push_val(v, 2) };
        unsafe { __vow_vec_push_val(v, 3) };
        unsafe { __vow_vec_pop(v) };
        unsafe { __vow_vec_pop(v) };
        unsafe { __vow_vec_push_val(v, 99) };
        let vec = unsafe { &*(v as *const VowVec) };
        assert_eq!(vec.len, 2);
        assert_eq!(unsafe { __vow_vec_get_val(v, 0) }, 1);
        assert_eq!(unsafe { __vow_vec_get_val(v, 1) }, 99);
    }

    #[test]
    #[allow(
        clippy::while_immutable_condition,
        reason = "loop body mutates *v through __vow_vec_pop; clippy can't see through raw pointer"
    )]
    fn vec_pop_truncate_loop() {
        let v = __vow_vec_new_val();
        for i in 0..10 {
            unsafe { __vow_vec_push_val(v, i) };
        }
        while unsafe { &*(v as *const VowVec) }.len > 3 {
            unsafe { __vow_vec_pop(v) };
        }
        assert_eq!(unsafe { &*(v as *const VowVec) }.len, 3);
        assert_eq!(unsafe { __vow_vec_get_val(v, 0) }, 0);
        assert_eq!(unsafe { __vow_vec_get_val(v, 1) }, 1);
        assert_eq!(unsafe { __vow_vec_get_val(v, 2) }, 2);
    }

    // All sanitize tests consolidated into one test to avoid parallel test races
    // on the global SANITIZE_ENABLED flag.
    #[test]
    fn sanitize_generation_tracking() {
        __vow_sanitize_init();

        // -- Push generation tracking --
        let v = __vow_vec_new_val();
        unsafe { __vow_vec_push_val(v, 10) };
        unsafe { __vow_vec_push_val(v, 20) };
        let gen0 = __vow_sanitize_vec_generation(v, 0);
        let gen1 = __vow_sanitize_vec_generation(v, 1);
        assert!(gen0 > 0, "generation should be nonzero after push");
        assert!(gen1 > gen0, "second push should have higher generation");

        // -- Set increments generation --
        unsafe { __vow_vec_set_val(v, 0, 99) };
        let gen0_after = __vow_sanitize_vec_generation(v, 0);
        assert!(gen0_after > gen0, "set should increment generation");
        assert_eq!(
            __vow_sanitize_vec_generation(v, 1),
            gen1,
            "unmodified slot should keep its generation"
        );

        // -- Check generation pass --
        let slot_gen = __vow_sanitize_vec_generation(v, 0);
        __vow_sanitize_check_generation(v, 0, slot_gen);

        // -- Truncate clears generations --
        let v2 = __vow_vec_new_val();
        unsafe { __vow_vec_push_val(v2, 1) };
        unsafe { __vow_vec_push_val(v2, 2) };
        unsafe { __vow_vec_push_val(v2, 3) };
        assert!(
            __vow_sanitize_vec_generation(v2, 2) > 0,
            "slot 2 should have generation"
        );
        unsafe { __vow_vec_truncate(v2, 1) };
        assert_eq!(
            __vow_sanitize_vec_generation(v2, 2),
            0,
            "truncated slot should have no generation"
        );

        // -- Pop removes generation --
        let v3 = __vow_vec_new_val();
        unsafe { __vow_vec_push_val(v3, 1) };
        unsafe { __vow_vec_push_val(v3, 2) };
        assert!(__vow_sanitize_vec_generation(v3, 1) > 0);
        unsafe { __vow_vec_pop(v3) };
        assert_eq!(
            __vow_sanitize_vec_generation(v3, 1),
            0,
            "popped slot should have no generation"
        );

        // -- Vec operations work without crash when sanitize enabled --
        let v4 = __vow_vec_new_val();
        unsafe { __vow_vec_push_val(v4, 42) };
    }

    // -----------------------------------------------------------------------
    // Arena primitive tests (docs/design/arena_memory.md §3, §10.4)
    // -----------------------------------------------------------------------

    fn empty_arena_header() -> VowArena {
        VowArena {
            first_chunk: core::ptr::null_mut(),
            current_chunk: core::ptr::null_mut(),
            cursor: 0,
            chunk_end: 0,
            last_alloc_start: core::ptr::null_mut(),
            last_alloc_size: 0,
            retained_bytes: 0,
        }
    }

    #[test]
    fn arena_open_close_roundtrip() {
        let mut a = empty_arena_header();
        unsafe { __vow_arena_init_closed(&mut a) };
        unsafe { __vow_arena_open(&mut a) };
        assert!(!a.first_chunk.is_null());
        assert_eq!(a.first_chunk, a.current_chunk);
        assert!(a.cursor >= a.first_chunk as usize + CHUNK_LINK_BYTES);
        assert_eq!(a.chunk_end, a.first_chunk as usize + normal_chunk_total());
        unsafe { __vow_arena_close(&mut a) };
        assert!(a.first_chunk.is_null());
    }

    #[test]
    fn parse_i64_option_can_be_owned_by_a_local_arena() {
        let mut arena = empty_arena_header();
        unsafe { __vow_arena_open(&mut arena) };
        let input = unsafe { __vow_string_new_in_arena(&mut arena, c"42".as_ptr(), 2) };
        let parsed =
            unsafe { __vow_string_parse_i64_opt_in_arena(&mut arena, input) } as *const i64;

        assert_eq!(unsafe { *parsed }, 1);
        assert_eq!(unsafe { *parsed.add(1) }, 42);

        unsafe { __vow_arena_close(&mut arena) };
        assert!(arena.first_chunk.is_null());
    }

    fn option_pair(cell: *mut u8) -> (i64, i64) {
        let cell = cell as *const i64;
        unsafe { (*cell, *cell.add(1)) }
    }

    #[test]
    fn option_cell_is_a_bare_two_word_arena_allocation() {
        let mut a = empty_arena_header();
        unsafe { __vow_arena_open(&mut a) };

        let some = unsafe { alloc_option_in_arena(&mut a, "test", Some(-9)) };
        let none = unsafe { alloc_option_in_arena(&mut a, "test", None) };
        assert_eq!(option_pair(some), (1, -9));
        assert_eq!(option_pair(none), (0, 0));
        assert_eq!(none as usize - some as usize, 16, "16 bytes per cell");

        unsafe { __vow_arena_close(&mut a) };
    }

    /// Runs in a worker process: `SHADOW_TABLE` is process-global and never
    /// shrinks, so parallel tests would leave stale entries at reused addresses.
    #[test]
    fn option_cells_leave_no_sanitize_shadow_entry() {
        let (out, stderr) = spawn_trap_worker("option_cells_shadow_untracked");
        assert_eq!(
            out.status.code(),
            Some(0),
            "Option cells must not be shadow-tracked as Vecs; stderr:\n{stderr}"
        );
    }

    fn vow_text(s: *mut u8) -> String {
        let v = unsafe { &*(s as *const VowVec) };
        let bytes = unsafe { std::slice::from_raw_parts(v.ptr, v.len) };
        String::from_utf8(bytes.to_vec()).unwrap()
    }

    fn vow_words(v: *mut u8) -> Vec<i64> {
        let v = unsafe { &*(v as *const VowVec) };
        unsafe { std::slice::from_raw_parts(v.ptr as *const i64, v.len) }.to_vec()
    }

    #[test]
    fn fresh_string_and_vec_builtins_allocate_in_the_requested_arena() {
        let mut a = empty_arena_header();
        let ap: *mut VowArena = &mut a;
        unsafe { __vow_arena_open(ap) };
        let before = unsafe { (*ap).cursor };

        let unsorted = unsafe { alloc_vec_of_values(ap, &[3, 1, 2]) };
        let sorted = unsafe { __vow_vec_sort_in_arena(ap, unsorted) };
        assert_eq!(vow_words(sorted), vec![1, 2, 3]);
        assert_eq!(vow_words(unsorted), vec![3, 1, 2], "input is untouched");

        let bytes = unsafe { alloc_vec_of_values(ap, &[255, 1]) };
        let hex = unsafe { __vow_hex_encode_in_arena(ap, bytes) };
        assert_eq!(vow_text(hex), "ff01");
        let decoded = unsafe { __vow_hex_decode_in_arena(ap, hex) };
        assert_eq!(vow_words(decoded), vec![255, 1]);
        let bad = unsafe { __vow_string_new_in_arena(ap, c"zz".as_ptr(), 2) };
        assert!(vow_words(unsafe { __vow_hex_decode_in_arena(ap, bad) }).is_empty());

        let text = unsafe { __vow_format_f64_bits_in_arena(ap, 1.5f64.to_bits()) };
        assert_eq!(vow_text(text), "1.5");

        let args = unsafe { __vow_args_in_arena(ap) };
        assert!(!vow_words(args).is_empty(), "argv[0] is always present");

        assert!(unsafe { (*ap).cursor } > before);
        unsafe { __vow_arena_close(ap) };
    }

    #[test]
    fn fresh_file_builtins_allocate_in_the_requested_arena() {
        let dir = std::env::temp_dir().join(format!("vow_fresh_fs_{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("b.txt"), "line1\nline2\n").unwrap();
        std::fs::write(dir.join("a.txt"), "").unwrap();
        let dir_s = dir.to_str().unwrap().to_string();
        let file_s = dir.join("b.txt").to_str().unwrap().to_string();

        let mut a = empty_arena_header();
        let ap: *mut VowArena = &mut a;
        unsafe { __vow_arena_open(ap) };
        let dir_v = unsafe { __vow_string_new_in_arena(ap, dir_s.as_ptr().cast(), dir_s.len()) };
        let file_v = unsafe { __vow_string_new_in_arena(ap, file_s.as_ptr().cast(), file_s.len()) };

        assert_eq!(
            vow_text(unsafe { __vow_fs_read_in_arena(ap, file_v) }),
            "line1\nline2\n"
        );
        let missing = unsafe { __vow_string_new_in_arena(ap, c"/nonexistent/x".as_ptr(), 14) };
        assert_eq!(vow_text(unsafe { __vow_fs_read_in_arena(ap, missing) }), "");
        assert_eq!(
            vow_text(unsafe { __vow_fs_read_in_arena(ap, std::ptr::null()) }),
            ""
        );

        let names: Vec<String> = vow_words(unsafe { __vow_fs_listdir_in_arena(ap, dir_v) })
            .into_iter()
            .map(|p| vow_text(p as *mut u8))
            .collect();
        assert_eq!(names, vec!["a.txt", "b.txt"]);

        let handle = unsafe { __vow_fs_open(file_v) };
        assert!(handle > 0);
        assert_eq!(
            vow_text(unsafe { __vow_fs_read_line_in_arena(ap, handle) }),
            "line1\n"
        );
        assert_eq!(
            vow_text(unsafe { __vow_fs_read_line_in_arena(ap, handle) }),
            "line2\n"
        );
        assert_eq!(
            vow_text(unsafe { __vow_fs_read_line_in_arena(ap, handle) }),
            ""
        );
        assert_eq!(__vow_fs_status(handle), 1);
        assert_eq!(__vow_fs_close(handle), 0);
        assert_eq!(
            vow_text(unsafe { __vow_fs_read_line_in_arena(ap, handle) }),
            ""
        );

        unsafe { __vow_arena_close(ap) };
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn fresh_process_builtins_allocate_in_the_requested_arena() {
        let child = std::process::Command::new("sh")
            .arg("-c")
            .arg("printf 'out-text'; printf 'err-text' 1>&2")
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped())
            .spawn()
            .expect("spawn sh");
        let handle = 999_103;
        {
            let mut g = PROCESS_MAP.lock().unwrap();
            process_map_init(&mut g).insert(handle, ProcessState::Running(child));
        }
        let mut a = empty_arena_header();
        let ap: *mut VowArena = &mut a;
        unsafe { __vow_arena_open(ap) };
        assert_eq!(
            vow_text(unsafe { __vow_process_stdout_for_in_arena(ap, handle) }),
            "",
            "a running process has no captured output yet"
        );
        assert_eq!(__vow_process_wait(handle), 0);
        assert_eq!(
            vow_text(unsafe { __vow_process_stdout_for_in_arena(ap, handle) }),
            "out-text"
        );
        assert_eq!(
            vow_text(unsafe { __vow_process_stderr_for_in_arena(ap, handle) }),
            "err-text"
        );
        assert_eq!(vow_text(__vow_process_stdout_for(handle)), "out-text");
        assert_eq!(vow_text(__vow_process_stderr_for(handle)), "err-text");
        assert_eq!(
            vow_text(unsafe { __vow_process_stdout_for_in_arena(ap, -1) }),
            "",
            "unknown handle"
        );

        LAST_STDOUT.with(|c| *c.borrow_mut() = b"last-out".to_vec());
        LAST_STDERR.with(|c| *c.borrow_mut() = b"last-err".to_vec());
        assert_eq!(
            vow_text(unsafe { __vow_process_get_stdout_in_arena(ap) }),
            "last-out"
        );
        assert_eq!(
            vow_text(unsafe { __vow_process_get_stderr_in_arena(ap) }),
            "last-err"
        );
        assert_eq!(vow_text(__vow_process_get_stdout()), "last-out");
        assert_eq!(vow_text(__vow_process_get_stderr()), "last-err");

        unsafe { __vow_arena_close(ap) };
    }

    #[test]
    fn fresh_builtin_root_wrappers_agree_with_the_arena_variants() {
        let v = __vow_vec_new_val();
        for val in [9, 7, 8] {
            unsafe { __vow_vec_push_val(v, val) };
        }
        assert_eq!(vow_words(unsafe { __vow_vec_sort(v) }), vec![7, 8, 9]);
        assert_eq!(vow_text(unsafe { __vow_hex_encode(v) }), "090708");
        assert_eq!(
            vow_text(unsafe { __vow_format_f64_bits(2.5f64.to_bits()) }),
            "2.5"
        );
        let hex = unsafe { __vow_string_new(c"0a0b".as_ptr(), 4) };
        assert_eq!(vow_words(unsafe { __vow_hex_decode(hex) }), vec![10, 11]);
        assert!(!vow_words(__vow_args()).is_empty());
        let dir = std::env::temp_dir().join(format!("vow_fresh_root_{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("f.txt"), "one\ntwo\n").unwrap();
        let file_s = dir.join("f.txt").to_str().unwrap().to_string();
        let dir_s = dir.to_str().unwrap().to_string();
        let file_v = unsafe { __vow_string_new(file_s.as_ptr().cast(), file_s.len()) };
        let dir_v = unsafe { __vow_string_new(dir_s.as_ptr().cast(), dir_s.len()) };
        assert_eq!(vow_text(unsafe { __vow_fs_read(file_v) }), "one\ntwo\n");
        assert_eq!(vow_text(unsafe { __vow_fs_read(std::ptr::null()) }), "");
        let names = vow_words(unsafe { __vow_fs_listdir(dir_v) });
        assert_eq!(names.len(), 1);
        assert_eq!(vow_text(names[0] as *mut u8), "f.txt");
        assert!(vow_words(unsafe { __vow_fs_listdir(std::ptr::null()) }).is_empty());
        let handle = unsafe { __vow_fs_open(file_v) };
        assert_eq!(vow_text(__vow_fs_read_line(handle)), "one\n");
        assert_eq!(vow_text(__vow_fs_read_line(handle)), "two\n");
        assert_eq!(vow_text(__vow_fs_read_line(handle)), "");
        assert_eq!(__vow_fs_close(handle), 0);
        std::fs::remove_dir_all(&dir).unwrap();

        let odd = unsafe { __vow_string_new(c"abc".as_ptr(), 3) };
        assert!(vow_words(unsafe { __vow_hex_decode(odd) }).is_empty());
        let bad = unsafe { __vow_string_new(c"zz".as_ptr(), 2) };
        assert!(vow_words(unsafe { __vow_hex_decode(bad) }).is_empty());
        assert!(vow_words(unsafe { __vow_hex_decode(std::ptr::null()) }).is_empty());
        assert_eq!(vow_text(unsafe { __vow_hex_encode(std::ptr::null()) }), "");
        assert!(vow_words(unsafe { __vow_vec_sort(std::ptr::null()) }).is_empty());
    }

    #[test]
    fn option_builtins_allocate_only_in_the_requested_arena() {
        let mut a = empty_arena_header();
        unsafe { __vow_arena_open(&mut a) };
        let input = unsafe { __vow_string_new_in_arena(&mut a, c"200".as_ptr(), 3) };

        let before = a.cursor;
        let parsed = unsafe { __vow_string_parse_u64_opt_in_arena(&mut a, input) };
        assert_eq!(option_pair(parsed), (1, 200));
        let narrowed = unsafe { __vow_i64_to_u8_try_in_arena(&mut a, 300) };
        assert_eq!(option_pair(narrowed), (0, 0));
        let widened = unsafe { __vow_u64_to_i8_try_in_arena(&mut a, 100) };
        assert_eq!(option_pair(widened), (1, 100));
        let small = unsafe { __vow_string_parse_i8_opt_in_arena(&mut a, input) };
        assert_eq!(option_pair(small), (0, 0), "200 does not fit i8");
        assert_eq!(a.cursor - before, 4 * 16);

        unsafe { __vow_arena_close(&mut a) };
    }

    #[test]
    fn btreemap_lookups_do_not_touch_the_map_arena() {
        let mut map_arena = empty_arena_header();
        let mut call_arena = empty_arena_header();
        unsafe { __vow_arena_open(&mut map_arena) };
        unsafe { __vow_arena_open(&mut call_arena) };

        let m = unsafe { __vow_btreemap_new_in_arena(&mut map_arena) };
        let none = unsafe { __vow_btreemap_insert_in_arena(&mut call_arena, m, 5, 50) };
        assert_eq!(option_pair(none), (0, 0));
        let replaced = unsafe { __vow_btreemap_insert_in_arena(&mut call_arena, m, 5, 51) };
        assert_eq!(option_pair(replaced), (1, 50));

        let map_cursor = map_arena.cursor;
        let call_cursor = call_arena.cursor;
        for _ in 0..100 {
            let hit = unsafe { __vow_btreemap_get_in_arena(&mut call_arena, m, 5) };
            assert_eq!(option_pair(hit), (1, 51));
        }
        let miss = unsafe { __vow_btreemap_get_in_arena(&mut call_arena, m, 6) };
        assert_eq!(option_pair(miss), (0, 0));
        assert_eq!(
            map_arena.cursor, map_cursor,
            "lookups never allocate in the map's arena"
        );
        assert_eq!(call_arena.cursor - call_cursor, 101 * 16);

        unsafe { __vow_arena_close(&mut call_arena) };
        let hit = unsafe { __vow_btreemap_get_in_arena(&mut map_arena, m, 5) };
        assert_eq!(option_pair(hit), (1, 51), "map outlives the call arena");
        unsafe { __vow_arena_close(&mut map_arena) };
    }

    struct OwnedMapOps {
        initial_cap: usize,
        cell_bytes: usize,
        new: unsafe fn(*mut VowArena) -> *mut u8,
        insert: unsafe fn(*mut VowArena, *mut u8, i64, i64),
        insert_root: unsafe fn(*mut u8, i64, i64),
        get: unsafe fn(*mut VowArena, *mut u8, i64) -> *mut u8,
        len: unsafe fn(*mut u8) -> usize,
    }

    #[test]
    fn map_growth_stays_in_the_owning_arena_whatever_arena_the_caller_names() {
        let kinds = [
            OwnedMapOps {
                initial_cap: BTREEMAP_INITIAL_CAP,
                cell_bytes: 16,
                new: |a| unsafe { __vow_btreemap_new_in_arena(a) },
                insert: |a, m, k, v| unsafe {
                    __vow_btreemap_insert_in_arena(a, m, k, v);
                },
                insert_root: |m, k, v| unsafe {
                    __vow_btreemap_insert(m, k, v);
                },
                get: |a, m, k| unsafe { __vow_btreemap_get_in_arena(a, m, k) },
                len: |m| unsafe { __vow_btreemap_len(m) },
            },
            OwnedMapOps {
                initial_cap: MAP_INITIAL_CAP,
                cell_bytes: 0,
                new: |a| unsafe { __vow_map_new_in_arena(a) },
                insert: |a, m, k, v| unsafe { __vow_map_insert_in_arena(a, m, k, v) },
                insert_root: |m, k, v| unsafe { __vow_map_insert(m, k, v) },
                get: |a, m, k| unsafe { __vow_map_get_in_arena(a, m, k) },
                len: |m| unsafe { (*(m as *const VowMap)).len },
            },
        ];
        for ops in kinds {
            let mut owner = empty_arena_header();
            let mut call = empty_arena_header();
            unsafe { __vow_arena_open(&mut owner) };
            unsafe { __vow_arena_open(&mut call) };
            let n = (ops.initial_cap * 4) as i64;

            let map = unsafe { (ops.new)(&mut owner) };
            let (owner_before, call_before) = (owner.cursor, call.cursor);
            for i in (0..n).rev() {
                unsafe { (ops.insert)(&mut call, map, i, i * 10) };
            }
            assert!(owner.cursor > owner_before, "buffers grew in the owner");
            assert_eq!(
                call.cursor - call_before,
                n as usize * ops.cell_bytes,
                "the call arena only receives Option cells"
            );

            let owner_cursor = owner.cursor;
            unsafe { (ops.insert_root)(map, n, n * 10) };
            assert!(
                owner.cursor > owner_cursor,
                "root-wrapper growth of an owned map still lands in the owner"
            );

            unsafe { __vow_arena_close(&mut call) };
            assert_eq!(unsafe { (ops.len)(map) }, n as usize + 1);
            for i in 0..=n {
                let hit = unsafe { (ops.get)(&mut owner, map, i) };
                assert_eq!(option_pair(hit), (1, i * 10));
            }
            unsafe { __vow_arena_close(&mut owner) };
        }
    }

    fn root_arena_ptr() -> *mut VowArena {
        &raw mut __vow_root_arena
    }

    #[test]
    fn with_root_arena_marks_the_lock_as_held_only_inside_the_closure() {
        assert!(!ROOT_LOCK_HELD.with(Cell::get));
        let map = __vow_map_new();
        unsafe {
            with_root_arena(|root| {
                assert!(ROOT_LOCK_HELD.with(Cell::get));
                for i in 0..(MAP_INITIAL_CAP as i64 * 4) {
                    __vow_map_insert_in_arena(root, map, i, i);
                }
            })
        };
        assert!(!ROOT_LOCK_HELD.with(Cell::get));
        let header = unsafe { &*(map as *const VowMap) };
        assert_eq!(header.len, MAP_INITIAL_CAP * 4);
    }

    #[test]
    fn root_owned_map_growth_from_the_root_arena_entry_point_takes_the_root_lock() {
        use std::sync::mpsc::{RecvTimeoutError, channel};
        use std::time::Duration;

        let map = __vow_map_new() as usize;
        let root = root_arena_ptr() as usize;
        let n = MAP_INITIAL_CAP as i64 * 4;
        let (done_tx, done_rx) = channel();
        let guard = ROOT_ARENA_LOCK.lock().unwrap();
        let worker = std::thread::spawn(move || {
            for i in 0..n {
                unsafe { __vow_map_insert_in_arena(root as *mut VowArena, map as *mut u8, i, i) };
            }
            done_tx.send(()).unwrap();
        });
        assert_eq!(
            done_rx.recv_timeout(Duration::from_millis(50)),
            Err(RecvTimeoutError::Timeout),
            "growth of a root-owned map must wait for the root-arena lock"
        );
        drop(guard);
        done_rx.recv().unwrap();
        worker.join().unwrap();
        for i in 0..n {
            assert_eq!(
                option_pair(unsafe { __vow_map_get(map as *const u8, i) }),
                (1, i)
            );
        }
    }

    #[test]
    fn root_wrappers_never_take_the_root_lock_for_a_non_root_owner() {
        use std::sync::mpsc::channel;
        use std::time::Duration;

        let mut owner = empty_arena_header();
        unsafe { __vow_arena_open(&mut owner) };
        let map = unsafe { __vow_map_new_in_arena(&mut owner) } as usize;
        let vec = unsafe { __vow_vec_new_val_in_arena(&mut owner) } as usize;
        let text = unsafe { __vow_string_new_in_arena(&mut owner, c"".as_ptr(), 0) } as usize;
        let n = MAP_INITIAL_CAP as i64 * 4;
        let (done_tx, done_rx) = channel();
        let guard = ROOT_ARENA_LOCK.lock().unwrap();
        let worker = std::thread::spawn(move || {
            for i in 0..n {
                unsafe { __vow_map_insert(map as *mut u8, i, i) };
                unsafe { __vow_vec_push_val(vec as *mut u8, i) };
                unsafe { __vow_string_push_byte(text as *mut u8, b'x' as u64) };
            }
            done_tx.send(()).unwrap();
        });
        done_rx
            .recv_timeout(Duration::from_secs(10))
            .expect("growth in a non-root owner must not wait for the root-arena lock");
        drop(guard);
        worker.join().unwrap();
        let header = unsafe { &*(map as *const VowMap) };
        assert_eq!(header.len, n as usize);
        let desc = unsafe { &*(vec as *const VowVec) };
        assert_eq!(desc.len, n as usize);
        let text = unsafe { &*(text as *const VowVec) };
        assert_eq!(text.len, n as usize);
        unsafe { __vow_arena_close(&mut owner) };
    }

    #[test]
    fn concurrent_growth_of_root_owned_maps_through_the_arena_entry_point() {
        let root = root_arena_ptr() as usize;
        let n = 2000i64;
        let workers: Vec<_> = (0..8i64)
            .map(|t| {
                std::thread::spawn(move || {
                    let map = __vow_map_new() as usize;
                    let bt = __vow_btreemap_new() as usize;
                    for i in 0..n {
                        let root = root as *mut VowArena;
                        unsafe { __vow_map_insert_in_arena(root, map as *mut u8, i, i + t) };
                        unsafe { __vow_btreemap_insert(bt as *mut u8, i, i + t) };
                        if i % 64 == 0 {
                            let s = format!("worker-{t}-{i}");
                            unsafe { __vow_string_new(s.as_ptr() as *const c_char, s.len()) };
                        }
                    }
                    (t, map, bt)
                })
            })
            .collect();
        for worker in workers {
            let (t, map, bt) = worker.join().unwrap();
            for i in 0..n {
                assert_eq!(
                    option_pair(unsafe { __vow_map_get(map as *const u8, i) }),
                    (1, i + t)
                );
                assert_eq!(
                    option_pair(unsafe { __vow_btreemap_get(bt as *const u8, i) }),
                    (1, i + t)
                );
            }
        }
    }

    #[test]
    fn hashmap_lookups_allocate_one_cell_in_the_requested_arena() {
        let mut a = empty_arena_header();
        unsafe { __vow_arena_open(&mut a) };
        let m = unsafe { __vow_map_new_in_arena(&mut a) };
        unsafe { __vow_map_insert_in_arena(&mut a, m, 1, 10) };

        let before = a.cursor;
        let hit = unsafe { __vow_map_get_in_arena(&mut a, m, 1) };
        let miss = unsafe { __vow_map_get_in_arena(&mut a, m, 2) };
        assert_eq!(option_pair(hit), (1, 10));
        assert_eq!(option_pair(miss), (0, 0));
        assert_eq!(a.cursor - before, 2 * 16);

        unsafe { __vow_arena_close(&mut a) };
    }

    #[test]
    fn parse_i64_option_root_wrapper_uses_the_root_arena() {
        let mut input_arena = empty_arena_header();
        unsafe { __vow_arena_open(&mut input_arena) };
        let input = unsafe { __vow_string_new_in_arena(&mut input_arena, c"-17".as_ptr(), 3) };
        let parsed = unsafe { __vow_string_parse_i64_opt(input) } as *const i64;

        assert_eq!(unsafe { *parsed }, 1);
        assert_eq!(unsafe { *parsed.add(1) }, -17);

        unsafe { __vow_arena_close(&mut input_arena) };
    }

    #[test]
    fn arena_open_on_open_arena_is_noop() {
        let mut a = empty_arena_header();
        unsafe { __vow_arena_init_closed(&mut a) };
        unsafe { __vow_arena_open(&mut a) };
        let first = a.first_chunk;
        let p = unsafe { __vow_arena_alloc(&mut a, 64, 8) };
        unsafe { __vow_arena_open(&mut a) };
        assert!(!a.first_chunk.is_null());
        assert_eq!(a.first_chunk, first);
        assert_eq!(a.first_chunk, a.current_chunk);
        assert_eq!(a.last_alloc_start, p);
        assert_eq!(a.last_alloc_size, 64);
        unsafe { __vow_arena_close(&mut a) };
        assert!(a.first_chunk.is_null());
    }

    #[test]
    fn arena_small_alloc_in_first_chunk() {
        let mut a = empty_arena_header();
        unsafe { __vow_arena_open(&mut a) };
        let first_base = a.first_chunk as usize;
        let p = unsafe { __vow_arena_alloc(&mut a, 64, 8) };
        assert!(!p.is_null());
        let addr = p as usize;
        assert!(addr >= first_base + CHUNK_LINK_BYTES);
        assert!(addr + 64 <= a.chunk_end);
        assert_eq!(a.last_alloc_start, p);
        assert_eq!(a.last_alloc_size, 64);
        unsafe { __vow_arena_close(&mut a) };
    }

    #[test]
    fn arena_overflow_triggers_new_chunk() {
        let mut a = empty_arena_header();
        unsafe { __vow_arena_open(&mut a) };
        let first = a.first_chunk;
        // 8 × 512 = 4096 bytes fits exactly in the first chunk (payload=4096).
        for _ in 0..8 {
            let _ = unsafe { __vow_arena_alloc(&mut a, 512, 8) };
        }
        assert_eq!(
            a.current_chunk, first,
            "still in first chunk after 4096 bytes"
        );
        // One more 512-byte alloc overflows; must spill into a new chunk.
        let _ = unsafe { __vow_arena_alloc(&mut a, 512, 8) };
        assert_ne!(a.current_chunk, first, "new chunk allocated on overflow");
        unsafe { __vow_arena_close(&mut a) };
    }

    #[test]
    fn arena_oversized_allocation_custom_chunk() {
        let mut a = empty_arena_header();
        unsafe { __vow_arena_open(&mut a) };
        // Bump past the first chunk to force the next alloc through the
        // new-chunk path. A 64-byte prefix alloc + a 4096-byte oversized
        // request won't fit in the remaining 4032 bytes, so spec §3.2's
        // oversized path fires.
        let _ = unsafe { __vow_arena_alloc(&mut a, 64, 8) };
        let first = a.current_chunk;
        let p = unsafe { __vow_arena_alloc(&mut a, 4096, 8) };
        assert!(!p.is_null());
        assert_ne!(
            a.current_chunk, first,
            "oversized alloc lives in its own chunk"
        );
        let expected_total = oversized_chunk_total(4096, 8);
        assert_eq!(a.chunk_end, a.current_chunk as usize + expected_total);
        unsafe { __vow_arena_close(&mut a) };
    }

    #[test]
    fn arena_growth_releases_oversized_chunk_for_abandoned_backing() {
        // Regression for issue #391: when a Vec/String/HashMap backing grows
        // out of an oversized chunk into a new oversized chunk, the abandoned
        // chunk must be returned to libc immediately rather than retained
        // until arena close.
        let mut a = empty_arena_header();
        unsafe { __vow_arena_open(&mut a) };

        // Bump the cursor inside the first (normal) chunk so the next alloc
        // forces an oversized chunk.
        let _prefix = unsafe { __vow_arena_alloc(&mut a, 64, 8) };
        let initial_retained = a.retained_bytes;

        let big1 = 4096usize;
        let p1 = unsafe { __vow_arena_alloc(&mut a, big1, 8) };
        assert!(!p1.is_null());
        let after_first_oversized = a.retained_bytes;
        assert!(after_first_oversized > initial_retained);

        // Grow it: arena_grow_backing must free the first oversized chunk
        // and only the new (larger) oversized chunk remains for this backing.
        let big2 = 8192usize;
        let p2 = unsafe { arena_grow_backing(&mut a, p1, big1, big2, 8) };
        assert!(!p2.is_null());
        assert_ne!(p2, p1, "growth must move the backing to a fresh chunk");

        // Walk the chunk chain — the chunk that contained `p1` must be gone.
        let mut found_old = false;
        let mut chunk = a.first_chunk;
        while !chunk.is_null() {
            let total = unsafe { chunk_total(chunk) };
            let base = chunk as usize;
            if (p1 as usize) >= base + CHUNK_LINK_BYTES && (p1 as usize) < base + total {
                found_old = true;
                break;
            }
            chunk = unsafe { next_chunk(chunk) };
        }
        assert!(
            !found_old,
            "abandoned oversized chunk must be unlinked from the chain"
        );

        // Retained bytes should reflect only the chunks still in the chain.
        let mut walked = 0usize;
        let mut chunk = a.first_chunk;
        while !chunk.is_null() {
            walked += unsafe { chunk_total(chunk) };
            chunk = unsafe { next_chunk(chunk) };
        }
        assert_eq!(
            a.retained_bytes, walked,
            "retained_bytes must match the live chunk chain"
        );

        unsafe { __vow_arena_close(&mut a) };
    }

    #[test]
    fn arena_growth_releases_mid_size_oversized_chunk() {
        // Edge case for issue #391: oversized-path allocations whose `total`
        // is ≤ `normal_chunk_total()` (e.g. a 3000-byte single-resident
        // string backing has total 3016 < 4112) are still single-resident
        // and reclaimable. A size-only classifier would skip them; the
        // path-flag classifier (`chunk_is_oversized`) correctly frees them.
        let mut a = empty_arena_header();
        unsafe { __vow_arena_open(&mut a) };

        // Push the first chunk's cursor past `chunk_end - 3000` so the
        // 3000-byte alloc cannot fit there and must take the new-chunk
        // (oversized) path.
        let _filler = unsafe { __vow_arena_alloc(&mut a, 2000, 1) };

        // Allocate a path-oversized 3000-byte buffer (bytes > 2048,
        // align=1). total = 16 + 3000 + 0 = 3016, well under 4112.
        let small_oversized = 3000usize;
        let p1 = unsafe { __vow_arena_alloc(&mut a, small_oversized, 1) };
        let oversized_chunk = a.current_chunk;
        let oversized_total = unsafe { chunk_total(oversized_chunk) };
        assert!(
            oversized_total <= normal_chunk_total(),
            "test setup: chunk total must sit in the historical classifier gap"
        );
        assert!(
            unsafe { chunk_is_oversized(oversized_chunk) },
            "alloc must record the oversized-path flag in the chunk header"
        );

        // Grow it. arena_grow_backing allocates a new chunk and must free
        // the abandoned mid-size oversized chunk via the path-flag check.
        let larger = 6000usize;
        let p2 = unsafe { arena_grow_backing(&mut a, p1, small_oversized, larger, 1) };
        assert_ne!(p2, p1, "growth must move to a fresh chunk");

        // The old oversized chunk must be gone from the chain.
        let mut chunk = a.first_chunk;
        while !chunk.is_null() {
            assert_ne!(
                chunk, oversized_chunk,
                "mid-size oversized chunk must be unlinked despite total ≤ normal_chunk_total()"
            );
            chunk = unsafe { next_chunk(chunk) };
        }

        unsafe { __vow_arena_close(&mut a) };
    }

    #[test]
    fn arena_growth_keeps_normal_chunk_for_small_backing() {
        // Inverse of the above: a small backing lives in a normal chunk shared
        // with other allocations and must NOT be freed when it grows. The
        // chunk still holds the prefix allocation, so freeing it would corrupt
        // memory.
        let mut a = empty_arena_header();
        unsafe { __vow_arena_open(&mut a) };

        let prefix = unsafe { __vow_arena_alloc(&mut a, 64, 8) };
        let p_small = unsafe { __vow_arena_alloc(&mut a, 64, 8) };
        // Interpose an allocation so `last_alloc_start != p_small` — without
        // this, `arena_grow_backing` would take `__vow_arena_try_extend`'s
        // in-place fast path and `arena_try_free_oversized_chunk` would never
        // run, making the test vacuous for the stated invariant.
        let _interpose = unsafe { __vow_arena_alloc(&mut a, 8, 8) };
        let head_before = a.first_chunk;

        // Trigger growth via arena_grow_backing — try_extend now fails
        // (last_alloc_start is `_interpose`), so the fallback path runs and
        // calls arena_try_free_oversized_chunk on the abandoned backing.
        let p_grown = unsafe { arena_grow_backing(&mut a, p_small, 64, 128, 8) };
        assert_ne!(
            p_grown, p_small,
            "growth must take the copy+free fallback path (try_extend should have been skipped)"
        );

        assert_eq!(
            a.first_chunk, head_before,
            "small abandoned backing must not unlink its (shared) normal chunk"
        );
        // The prefix allocation must still be readable.
        let _byte = unsafe { *prefix };

        unsafe { __vow_arena_close(&mut a) };
    }

    #[test]
    fn arena_try_extend_succeeds_for_last_alloc() {
        let mut a = empty_arena_header();
        unsafe { __vow_arena_open(&mut a) };
        let p = unsafe { __vow_arena_alloc(&mut a, 64, 8) };
        let r1 = unsafe { __vow_arena_try_extend(&mut a, p, 64, 128) };
        assert_eq!(r1, 1);
        assert_eq!(a.last_alloc_size, 128, "size updated post-extend");
        // Back-to-back: subsequent extend must see the post-extend size.
        let r2 = unsafe { __vow_arena_try_extend(&mut a, p, 128, 256) };
        assert_eq!(r2, 1);
        assert_eq!(a.last_alloc_size, 256);
        unsafe { __vow_arena_close(&mut a) };
    }

    #[test]
    fn arena_try_extend_fails_not_last_alloc() {
        let mut a = empty_arena_header();
        unsafe { __vow_arena_open(&mut a) };
        let pa = unsafe { __vow_arena_alloc(&mut a, 64, 8) };
        let _pb = unsafe { __vow_arena_alloc(&mut a, 64, 8) };
        let r = unsafe { __vow_arena_try_extend(&mut a, pa, 64, 128) };
        assert_eq!(r, 0, "try_extend must fail when ptr is not the last alloc");
        unsafe { __vow_arena_close(&mut a) };
    }

    #[test]
    fn arena_try_extend_fails_old_size_mismatch() {
        let mut a = empty_arena_header();
        unsafe { __vow_arena_open(&mut a) };
        let p = unsafe { __vow_arena_alloc(&mut a, 64, 8) };
        // ptr matches last_alloc_start but old_size does not.
        let r = unsafe { __vow_arena_try_extend(&mut a, p, 32, 64) };
        assert_eq!(
            r, 0,
            "try_extend must fail when old_size != last_alloc_size"
        );
        unsafe { __vow_arena_close(&mut a) };
    }

    #[test]
    fn arena_try_extend_fails_chunk_overflow() {
        let mut a = empty_arena_header();
        unsafe { __vow_arena_open(&mut a) };
        let p = unsafe { __vow_arena_alloc(&mut a, 64, 8) };
        let before_cursor = a.cursor;
        let before_end = a.chunk_end;
        // Request an extension that exceeds the chunk.
        let r = unsafe { __vow_arena_try_extend(&mut a, p, 64, 1 << 30) };
        assert_eq!(r, 0);
        assert_eq!(a.cursor, before_cursor, "cursor unchanged on failure");
        assert_eq!(a.chunk_end, before_end, "chunk_end unchanged");
        assert_eq!(a.last_alloc_size, 64, "last_alloc_size unchanged");
        unsafe { __vow_arena_close(&mut a) };
    }

    #[test]
    fn explicit_arena_vec_pushes_values() {
        let mut a = empty_arena_header();
        unsafe { __vow_arena_open(&mut a) };

        let v = unsafe { __vow_vec_new_in_arena(&mut a, 8, 8) };
        let header = unsafe { &*(v as *const VowVec) };
        assert_eq!(header.len, 0);
        assert_eq!(vow_vec_capacity(header), 0);

        let first = 17_i64;
        let second = 23_i64;
        unsafe { __vow_vec_push_in_arena(&mut a, v, &first as *const _ as *const u8, 8, 8) };
        unsafe { __vow_vec_push_in_arena(&mut a, v, &second as *const _ as *const u8, 8, 8) };

        assert_eq!(unsafe { __vow_vec_len(v) }, 2);
        assert_eq!(unsafe { __vow_vec_get_val(v, 0) }, 17);
        assert_eq!(unsafe { __vow_vec_get_val(v, 1) }, 23);

        unsafe { __vow_arena_close(&mut a) };
    }

    #[test]
    fn explicit_arena_vec_new_val_reserve_and_push_val() {
        let mut a = empty_arena_header();
        unsafe { __vow_arena_open(&mut a) };

        let v = unsafe { __vow_vec_new_val_in_arena(&mut a) };
        unsafe { __vow_vec_reserve_in_arena(&mut a, v, 12, 8, 8) };
        let header = unsafe { &*(v as *const VowVec) };
        assert_eq!(header.len, 0, "reserve must not change len");
        assert!(vow_vec_capacity(header) >= 12);

        unsafe { __vow_vec_push_val_in_arena(&mut a, v, 99) };
        assert_eq!(unsafe { __vow_vec_len(v) }, 1);
        assert_eq!(unsafe { __vow_vec_get_val(v, 0) }, 99);

        unsafe { __vow_arena_close(&mut a) };
    }

    #[test]
    fn explicit_arena_vec_growth_preserves_values_after_copy_fallback() {
        let mut a = empty_arena_header();
        unsafe { __vow_arena_open(&mut a) };

        let v = unsafe { __vow_vec_new_val_in_arena(&mut a) };
        for i in 0..8 {
            unsafe { __vow_vec_push_val_in_arena(&mut a, v, i) };
        }
        let before = unsafe { &*(v as *const VowVec) }.ptr;
        let _intervening = unsafe { __vow_arena_alloc(&mut a, 16, 8) };
        unsafe { __vow_vec_push_val_in_arena(&mut a, v, 8) };

        let after = unsafe { &*(v as *const VowVec) }.ptr;
        assert_ne!(
            after, before,
            "intervening allocation should force allocate-copy growth"
        );
        for i in 0..9 {
            assert_eq!(unsafe { __vow_vec_get_val(v, i as usize) }, i);
        }

        unsafe { __vow_arena_close(&mut a) };
    }

    #[test]
    fn explicit_arena_vecs_remain_independent_across_two_open_arenas() {
        let mut a = empty_arena_header();
        let mut b = empty_arena_header();
        unsafe { __vow_arena_open(&mut a) };
        unsafe { __vow_arena_open(&mut b) };

        let va = unsafe { __vow_vec_new_val_in_arena(&mut a) };
        let vb = unsafe { __vow_vec_new_val_in_arena(&mut b) };
        unsafe { __vow_vec_push_val_in_arena(&mut a, va, 1) };
        unsafe { __vow_vec_push_val_in_arena(&mut b, vb, 10) };
        unsafe { __vow_vec_push_val_in_arena(&mut b, vb, 20) };

        assert_eq!(unsafe { __vow_vec_get_val(va, 0) }, 1);
        assert_eq!(unsafe { __vow_vec_get_val(vb, 0) }, 10);
        assert_eq!(unsafe { __vow_vec_get_val(vb, 1) }, 20);

        unsafe { __vow_arena_close(&mut a) };
        unsafe { __vow_vec_push_val_in_arena(&mut b, vb, 30) };
        assert_eq!(unsafe { __vow_vec_len(vb) }, 3);
        assert_eq!(unsafe { __vow_vec_get_val(vb, 2) }, 30);
        unsafe { __vow_arena_close(&mut b) };
    }

    #[test]
    fn explicit_arena_vec_allocation_works_after_close_and_reopen() {
        let mut a = empty_arena_header();
        unsafe { __vow_arena_open(&mut a) };
        let first = unsafe { __vow_vec_new_val_in_arena(&mut a) };
        unsafe { __vow_vec_push_val_in_arena(&mut a, first, 1) };
        unsafe { __vow_arena_close(&mut a) };

        unsafe { __vow_arena_open(&mut a) };
        let second = unsafe { __vow_vec_new_val_in_arena(&mut a) };
        unsafe { __vow_vec_push_val_in_arena(&mut a, second, 2) };
        assert_eq!(unsafe { __vow_vec_len(second) }, 1);
        assert_eq!(unsafe { __vow_vec_get_val(second, 0) }, 2);
        unsafe { __vow_arena_close(&mut a) };
    }

    #[test]
    fn explicit_arena_string_constructors_and_growth() {
        let mut a = empty_arena_header();
        unsafe { __vow_arena_open(&mut a) };

        let hello = unsafe { __vow_string_new_in_arena(&mut a, c"hello".as_ptr(), "hello".len()) };
        let comma = unsafe { __vow_string_from_cstr_in_arena(&mut a, c", ".as_ptr()) };
        unsafe { __vow_string_push_str_in_arena(&mut a, hello, comma) };
        unsafe { __vow_string_push_byte_in_arena(&mut a, hello, b'w' as u64) };

        let header = unsafe { &*(hello as *const VowVec) };
        let bytes = unsafe { std::slice::from_raw_parts(header.ptr, header.len) };
        assert_eq!(bytes, b"hello, w");

        let sub = unsafe { __vow_string_substring_in_arena(&mut a, hello, 7, 8) };
        let sub_header = unsafe { &*(sub as *const VowVec) };
        let sub_bytes = unsafe { std::slice::from_raw_parts(sub_header.ptr, sub_header.len) };
        assert_eq!(sub_bytes, b"w");

        let tail = unsafe { __vow_string_substr_in_arena(&mut a, hello, 5, 3) };
        let tail_header = unsafe { &*(tail as *const VowVec) };
        let tail_bytes = unsafe { std::slice::from_raw_parts(tail_header.ptr, tail_header.len) };
        assert_eq!(tail_bytes, b", w");

        let digits = unsafe { __vow_string_from_i64_in_arena(&mut a, -42) };
        let digits_header = unsafe { &*(digits as *const VowVec) };
        let digits_bytes =
            unsafe { std::slice::from_raw_parts(digits_header.ptr, digits_header.len) };
        assert_eq!(digits_bytes, b"-42");

        let unsigned_zero = unsafe { __vow_string_from_u64_in_arena(&mut a, 0) };
        let unsigned_zero_header = unsafe { &*(unsigned_zero as *const VowVec) };
        let unsigned_zero_bytes = unsafe {
            std::slice::from_raw_parts(unsigned_zero_header.ptr, unsigned_zero_header.len)
        };
        assert_eq!(unsigned_zero_bytes, b"0");

        let unsigned_max = unsafe { __vow_string_from_u64_in_arena(&mut a, u64::MAX) };
        let unsigned_max_header = unsafe { &*(unsigned_max as *const VowVec) };
        let unsigned_max_bytes =
            unsafe { std::slice::from_raw_parts(unsigned_max_header.ptr, unsigned_max_header.len) };
        assert_eq!(unsigned_max_bytes, b"18446744073709551615");

        unsafe { __vow_arena_close(&mut a) };
    }

    #[test]
    fn root_wrappers_grow_owned_vec_and_string_in_their_owner_arena() {
        let mut owner = empty_arena_header();
        unsafe { __vow_arena_open(&mut owner) };

        let v = unsafe { __vow_vec_new_val_in_arena(&mut owner) };
        for i in 0..64 {
            unsafe { __vow_vec_push_val(v, i) };
        }
        let desc = unsafe { &*(v as *const VowVec) };
        assert_eq!(desc.len, 64);
        assert_eq!(unsafe { *(desc.ptr as *const i64).add(63) }, 63);
        assert_eq!(
            owner.last_alloc_start, desc.ptr,
            "Vec::push_val growth through the root wrapper stays in the owner arena"
        );

        let w = unsafe { __vow_vec_new_in_arena(&mut owner, 4, 4) };
        for i in 0..40u32 {
            unsafe { __vow_vec_push(w, &i as *const u32 as *const u8, 4, 4) };
        }
        let desc = unsafe { &*(w as *const VowVec) };
        assert_eq!(desc.len, 40);
        assert_eq!(owner.last_alloc_start, desc.ptr, "Vec::push growth");

        let s = unsafe { __vow_string_new_in_arena(&mut owner, c"".as_ptr(), 0) };
        for _ in 0..40 {
            unsafe { __vow_string_push_byte(s, b'x' as u64) };
        }
        let desc = unsafe { &*(s as *const VowVec) };
        assert_eq!(desc.len, 40);
        assert_eq!(owner.last_alloc_start, desc.ptr, "String::push_byte growth");

        let chunk = unsafe { __vow_string_new_in_arena(&mut owner, c"0123456789".as_ptr(), 10) };
        for _ in 0..8 {
            unsafe { __vow_string_push_str(s, chunk) };
        }
        let desc = unsafe { &*(s as *const VowVec) };
        assert_eq!(desc.len, 120);
        assert_eq!(unsafe { *desc.ptr.add(119) }, b'9');
        assert_eq!(
            unsafe { *desc.ptr.add(40) },
            b'0',
            "the appended bytes follow the original 40"
        );
        assert_eq!(owner.last_alloc_start, desc.ptr, "String::push_str growth");

        unsafe { __vow_arena_close(&mut owner) };
    }

    #[test]
    fn candidate_string_push_byte_uses_matching_owner_arena() {
        let mut owner = empty_arena_header();
        unsafe { __vow_arena_open(&mut owner) };

        let s = unsafe { __vow_string_new_in_arena(&mut owner, c"".as_ptr(), 0) };
        unsafe { __vow_string_push_byte_in_candidate_arena(&mut owner, s, b'x' as u64) };

        let desc = unsafe { &*(s as *const VowVec) };
        assert_eq!(desc.len, 1);
        assert_eq!(unsafe { *desc.ptr }, b'x');
        assert_eq!(
            owner.last_alloc_start, desc.ptr,
            "matching provenance must allocate the backing in the candidate arena"
        );

        unsafe { __vow_arena_close(&mut owner) };
    }

    #[test]
    fn candidate_string_push_str_falls_back_for_foreign_owner() {
        let mut owner = empty_arena_header();
        let mut foreign_candidate = empty_arena_header();
        unsafe { __vow_arena_open(&mut owner) };
        unsafe { __vow_arena_open(&mut foreign_candidate) };

        let dest = unsafe { __vow_string_new_in_arena(&mut owner, c"a".as_ptr(), 1) };
        let src = unsafe { __vow_string_new_in_arena(&mut owner, c"bc".as_ptr(), 2) };
        let candidate_last_alloc = foreign_candidate.last_alloc_start;
        unsafe { __vow_string_push_str_in_candidate_arena(&mut foreign_candidate, dest, src) };

        let desc = unsafe { &*(dest as *const VowVec) };
        assert_eq!(
            unsafe { std::slice::from_raw_parts(desc.ptr, desc.len) },
            b"abc"
        );
        assert_eq!(
            foreign_candidate.last_alloc_start, candidate_last_alloc,
            "foreign provenance must not allocate in the candidate arena"
        );

        unsafe { __vow_arena_close(&mut foreign_candidate) };
        unsafe { __vow_arena_close(&mut owner) };
    }

    #[test]
    fn candidate_string_push_byte_ignores_foreign_prefix_bytes() {
        #[repr(C)]
        struct ForeignDescriptor {
            preceding_word: *mut VowArena,
            desc: VowVec,
        }

        let mut candidate = empty_arena_header();
        unsafe { __vow_arena_open(&mut candidate) };
        let candidate_last_alloc = candidate.last_alloc_start;

        let mut foreign = ForeignDescriptor {
            preceding_word: &mut candidate,
            desc: VowVec {
                ptr: std::ptr::dangling_mut::<u8>(),
                len: 0,
                cap: 0,
            },
        };
        unsafe {
            __vow_string_push_byte_in_candidate_arena(
                &mut candidate,
                core::ptr::addr_of_mut!(foreign.desc) as *mut u8,
                b'x' as u64,
            )
        };

        assert_eq!(foreign.desc.len, 1);
        // The descriptor went in holding the len=0 sentinel `Vec::new` uses
        // (`dangling_mut`, i.e. address 1 — non-null, but not an allocation),
        // which is what a regression would leave behind. Reject the sentinel
        // and then the null case before reading, so only a pointer proven to
        // be a real backing reaches the read.
        assert_ne!(
            foreign.desc.ptr,
            std::ptr::dangling_mut::<u8>(),
            "push must have installed a real backing over the len=0 sentinel"
        );
        let data = NonNull::new(foreign.desc.ptr).expect("a real backing is never null");
        assert_eq!(unsafe { data.read() }, b'x');
        assert_eq!(
            candidate.last_alloc_start, candidate_last_alloc,
            "a foreign descriptor must fall back even when preceding bytes match the candidate"
        );

        unsafe { __vow_arena_close(&mut candidate) };
    }

    #[test]
    fn string_push_str_self_append_oversized_no_uaf() {
        // Regression for the self-append UAF scenario flagged on PR #392:
        // `__vow_string_push_str_in_arena` used to capture `vs.ptr` from
        // the source descriptor before `__vow_vec_reserve_in_arena`. If
        // `src == dest` and the reserve grew the backing out of an
        // oversized chunk, `arena_grow_backing`'s chunk-free helper would
        // libc::free the old chunk, leaving the captured `vs.ptr`
        // dangling for the subsequent copy_nonoverlapping.
        let mut a = empty_arena_header();
        unsafe { __vow_arena_open(&mut a) };

        // Build a String whose backing is large enough that the chunk
        // total exceeds even the size-only classifier's old threshold:
        // 5000 bytes → oversized_chunk_total = 5016 > normal_chunk_total
        // = 4112. This ensures the old chunk is freed regardless of
        // which classifier (path-flag or size-based) is in place, so the
        // test still exercises the self-append UAF guard if a future
        // change touches either piece.
        let payload = vec![b'a'; 5000];
        let s = unsafe {
            __vow_string_new_in_arena(&mut a, payload.as_ptr() as *const c_char, payload.len())
        };
        // Sanity: the backing is in a path-oversized chunk.
        let header_before = unsafe { &*(s as *const VowVec) };
        assert!(
            vow_vec_capacity(header_before) > 2048,
            "test setup: must be oversized"
        );

        // Self-append: src == dest. With the fix, the post-reserve copy
        // reads from the new backing's prefix (where the old contents
        // were copied by `arena_grow_backing`) rather than from the
        // freed old chunk.
        unsafe { __vow_string_push_str_in_arena(&mut a, s, s as *const u8) };

        let header_after = unsafe { &*(s as *const VowVec) };
        assert_eq!(header_after.len, 10000, "len doubles on self-append");
        let bytes = unsafe { std::slice::from_raw_parts(header_after.ptr, header_after.len) };
        assert!(
            bytes.iter().all(|&b| b == b'a'),
            "all 10000 bytes must be 'a' — any other value indicates UAF read"
        );

        unsafe { __vow_arena_close(&mut a) };
    }

    #[test]
    fn string_matches_literal_at_uses_pointer_and_byte_len() {
        let mut bytes = b"za\0bq".to_vec();
        let s = VowVec {
            ptr: bytes.as_mut_ptr(),
            len: bytes.len(),
            cap: bytes.len(),
        };
        let literal = b"a\0b";
        let empty = b"";
        let s_ptr = &s as *const VowVec as *const u8;

        assert_eq!(
            unsafe {
                __vow_string_matches_literal_at(s_ptr, 1, literal.as_ptr(), literal.len() as u64)
            },
            1
        );
        assert_eq!(
            unsafe {
                __vow_string_matches_literal_at(s_ptr, 2, literal.as_ptr(), literal.len() as u64)
            },
            0
        );
        assert_eq!(
            unsafe { __vow_string_matches_literal_at(s_ptr, u64::MAX, literal.as_ptr(), 3) },
            0
        );
        assert_eq!(
            unsafe { __vow_string_matches_literal_at(s_ptr, 5, empty.as_ptr(), 0) },
            1
        );
        assert_eq!(
            unsafe { __vow_string_matches_literal_at(s_ptr, 6, empty.as_ptr(), 0) },
            0
        );
    }

    #[test]
    fn string_offsets_are_unsigned_and_clamp_high() {
        let mut a = empty_arena_header();
        unsafe { __vow_arena_open(&mut a) };
        let hello = unsafe { __vow_string_new_in_arena(&mut a, c"hello".as_ptr(), 5) };
        assert_eq!(unsafe { __vow_string_byte_at(hello, 0) }, b'h' as i64);
        assert_eq!(unsafe { __vow_string_byte_at(hello, 4) }, b'o' as i64);
        assert_eq!(unsafe { __vow_string_byte_at(hello, 5) }, -1);
        assert_eq!(unsafe { __vow_string_byte_at(hello, u64::MAX) }, -1);

        let read = |p: *mut u8| unsafe {
            let v = &*(p as *const VowVec);
            std::slice::from_raw_parts(v.ptr, v.len).to_vec()
        };
        let tail = unsafe { __vow_string_substr_in_arena(&mut a, hello, u64::MAX, 3) };
        assert_eq!(read(tail), b"");
        let all = unsafe { __vow_string_substr_in_arena(&mut a, hello, 1, u64::MAX) };
        assert_eq!(read(all), b"ello");
        let none = unsafe { __vow_string_substring_in_arena(&mut a, hello, u64::MAX, 3) };
        assert_eq!(read(none), b"");
        let rev = unsafe { __vow_string_substring_in_arena(&mut a, hello, 3, 1) };
        assert_eq!(read(rev), b"");
        let rest = unsafe { __vow_string_substring_in_arena(&mut a, hello, 2, u64::MAX) };
        assert_eq!(read(rest), b"llo");
    }

    #[test]
    fn string_clone_into_arena_copies_bytes() {
        // Phase 4 / S5 return materialization: clones a String descriptor's
        // backing into the supplied arena, returning a fresh, mutable
        // descriptor (cap == len, not VOW_CAP_RODATA).
        let mut a = empty_arena_header();
        unsafe { __vow_arena_open(&mut a) };
        // Source is a rodata-backed descriptor — exercises the spec §5.1
        // ".rodata literal returned on a FreshInCaller path" case.
        let bytes: &[u8] = b"hello";
        let source = VowVec {
            ptr: bytes.as_ptr() as *mut u8,
            len: bytes.len(),
            cap: VOW_CAP_RODATA,
        };
        let cloned =
            unsafe { __vow_string_clone_into_arena(&mut a, &source as *const VowVec as *const u8) };
        let cv = unsafe { &*(cloned as *const VowVec) };
        assert_eq!(cv.len, 5);
        assert_eq!(
            vow_vec_capacity(cv),
            5,
            "clone must not inherit VOW_CAP_RODATA"
        );
        let cloned_bytes = unsafe { std::slice::from_raw_parts(cv.ptr, cv.len) };
        assert_eq!(cloned_bytes, b"hello");
        // The clone's backing must live in the arena, not in .rodata.
        // `chunk_end` is an absolute address (`base + total`), not a size
        // offset, so the upper bound is just `chunk_end` directly.
        let cv_data = cv.ptr as usize;
        let arena_start = a.first_chunk.cast::<u8>() as usize;
        let arena_end = a.chunk_end;
        assert!(
            cv_data >= arena_start && cv_data < arena_end,
            "cloned data must live inside the arena chunk"
        );
        unsafe { __vow_arena_close(&mut a) };
    }

    #[test]
    fn string_pin_to_root_deep_copies_bytes() {
        let bytes: &[u8] = b"rooted";
        let source = VowVec {
            ptr: bytes.as_ptr() as *mut u8,
            len: bytes.len(),
            cap: VOW_CAP_RODATA,
        };
        let pinned = unsafe { __vow_string_pin_to_root(&source as *const VowVec as *const u8) };
        let pv = unsafe { &*(pinned as *const VowVec) };
        assert_eq!(pv.len, 6);
        assert_eq!(
            vow_vec_capacity(pv),
            6,
            "pinning must return a mutable root copy"
        );
        assert_ne!(pv.ptr, bytes.as_ptr() as *mut u8);
        let pinned_bytes = unsafe { std::slice::from_raw_parts(pv.ptr, pv.len) };
        assert_eq!(pinned_bytes, b"rooted");
    }

    #[test]
    fn stdin_read_line_scratch_reuses_capacity_for_many_lines() {
        let line_len = 4096;
        let line_count = 512;
        let mut input = Vec::with_capacity((line_len + 1) * line_count);
        for _ in 0..line_count {
            input.extend(std::iter::repeat_n(b'x', line_len));
            input.push(b'\n');
        }

        let mut reader = std::io::Cursor::new(input);
        let mut scratch = StdinLineScratch::new();
        for _ in 0..line_count {
            let ptr = read_stdin_line_into_scratch(&mut reader, &mut scratch);
            let line = unsafe { &*(ptr as *const VowVec) };
            assert_eq!(line.len, line_len + 1);
            assert_eq!(line.cap, VOW_CAP_RODATA);
        }

        assert!(
            scratch.bytes.capacity() <= 2 * (line_len + 1),
            "scratch capacity should track max line size, not total input"
        );
    }

    #[test]
    fn stdin_read_line_scratch_descriptor_is_thread_local() {
        let barrier = std::sync::Arc::new(std::sync::Barrier::new(3));
        let (tx, rx) = std::sync::mpsc::channel();
        let mut handles = Vec::new();

        for byte in *b"ab" {
            let barrier = std::sync::Arc::clone(&barrier);
            let tx = tx.clone();
            handles.push(std::thread::spawn(move || {
                let input = [byte, b'\n'];
                let mut reader = std::io::Cursor::new(input.as_slice());
                let ptr = STDIN_LINE_SCRATCH.with(|cell| {
                    let mut scratch = cell.borrow_mut();
                    read_stdin_line_into_scratch(&mut reader, &mut scratch) as usize
                });
                tx.send(ptr).unwrap();
                barrier.wait();
            }));
        }
        drop(tx);

        let first = rx.recv().unwrap();
        let second = rx.recv().unwrap();
        assert_ne!(
            first, second,
            "stdin scratch descriptor should be per-thread"
        );
        barrier.wait();

        for handle in handles {
            handle.join().unwrap();
        }
    }

    #[test]
    fn stdin_read_line_scratch_descriptor_is_reused_and_read_only() {
        let mut reader = std::io::Cursor::new(b"first\nsecond\n".as_slice());
        let mut scratch = StdinLineScratch::new();

        let first = read_stdin_line_into_scratch(&mut reader, &mut scratch);
        let first_desc = unsafe { &*(first as *const VowVec) };
        let first_bytes = unsafe { std::slice::from_raw_parts(first_desc.ptr, first_desc.len) };
        assert_eq!(first_bytes, b"first\n");
        assert_eq!(first_desc.cap, VOW_CAP_RODATA);

        let second = read_stdin_line_into_scratch(&mut reader, &mut scratch);
        assert_eq!(first, second, "stdin scratch descriptor should be stable");
        let second_desc = unsafe { &*(second as *const VowVec) };
        let second_bytes = unsafe { std::slice::from_raw_parts(second_desc.ptr, second_desc.len) };
        assert_eq!(second_bytes, b"second\n");
        assert_eq!(second_desc.cap, VOW_CAP_RODATA);
    }

    #[test]
    fn stdin_read_line_pin_to_root_preserves_previous_line() {
        let mut reader = std::io::Cursor::new(b"alpha\nbeta\n".as_slice());
        let mut scratch = StdinLineScratch::new();

        let first = read_stdin_line_into_scratch(&mut reader, &mut scratch);
        let pinned = unsafe { __vow_string_pin_to_root(first) };
        let _second = read_stdin_line_into_scratch(&mut reader, &mut scratch);

        let pinned_desc = unsafe { &*(pinned as *const VowVec) };
        let pinned_bytes = unsafe { std::slice::from_raw_parts(pinned_desc.ptr, pinned_desc.len) };
        assert_eq!(pinned_bytes, b"alpha\n");
    }

    #[test]
    fn string_from_raw_parts_copy_copies_bytes() {
        unsafe { ensure_root_arena() };
        let bytes: &[u8] = b"raw";
        let copied = unsafe {
            __vow_string_from_raw_parts_copy(&raw mut __vow_root_arena, bytes.as_ptr(), bytes.len())
        };
        let cv = unsafe { &*(copied as *const VowVec) };
        assert_eq!(cv.len, 3);
        assert!(vow_vec_capacity(cv) >= 3);
        assert_ne!(cv.ptr, bytes.as_ptr() as *mut u8);
        let copied_bytes = unsafe { std::slice::from_raw_parts(cv.ptr, cv.len) };
        assert_eq!(copied_bytes, b"raw");
    }

    #[test]
    fn vec_from_raw_parts_copy_val_copies_slots() {
        unsafe { ensure_root_arena() };
        let raw = [11_i64, 22_i64, 33_i64];
        let copied = unsafe {
            __vow_vec_from_raw_parts_copy_val(&raw mut __vow_root_arena, raw.as_ptr(), raw.len())
        };
        let cv = unsafe { &*(copied as *const VowVec) };
        assert_eq!(cv.len, 3);
        assert!(vow_vec_capacity(cv) >= 3);
        assert_ne!(cv.ptr, raw.as_ptr() as *mut u8);
        let copied_vals = unsafe { std::slice::from_raw_parts(cv.ptr as *const i64, cv.len) };
        assert_eq!(copied_vals, &[11, 22, 33]);
    }

    #[test]
    fn vec_pin_to_root_val_copies_slots() {
        let mut a = empty_arena_header();
        unsafe { __vow_arena_open(&mut a) };
        let raw = [7_i64, 8_i64];
        let source = unsafe { __vow_vec_from_raw_parts_copy_val(&mut a, raw.as_ptr(), raw.len()) };
        let pinned = unsafe { __vow_vec_pin_to_root_val(source) };
        unsafe { __vow_vec_push_val(source, 9) };
        let pv = unsafe { &*(pinned as *const VowVec) };
        assert_eq!(pv.len, 2);
        let pinned_vals = unsafe { std::slice::from_raw_parts(pv.ptr as *const i64, pv.len) };
        assert_eq!(pinned_vals, &[7, 8]);
        unsafe { __vow_arena_close(&mut a) };
    }

    #[test]
    fn string_clone_wrapper_copies_rodata_into_root() {
        let bytes: &[u8] = b"hello";
        let source = VowVec {
            ptr: bytes.as_ptr() as *mut u8,
            len: bytes.len(),
            cap: VOW_CAP_RODATA,
        };

        let cloned = unsafe { __vow_string_clone(&source as *const VowVec as *const u8) };
        let cv = unsafe { &*(cloned as *const VowVec) };
        assert_eq!(cv.len, 5);
        assert_eq!(vow_vec_capacity(cv), 5, "clone must be mutable, not rodata");
        assert_ne!(cv.ptr, source.ptr, "clone must copy backing bytes");
        let cloned_bytes = unsafe { std::slice::from_raw_parts(cv.ptr, cv.len) };
        assert_eq!(cloned_bytes, b"hello");
    }

    #[test]
    fn string_clone_into_arena_handles_empty() {
        let mut a = empty_arena_header();
        unsafe { __vow_arena_open(&mut a) };
        let source = VowVec {
            ptr: std::ptr::dangling_mut::<u8>(),
            len: 0,
            cap: VOW_CAP_RODATA,
        };
        let cloned =
            unsafe { __vow_string_clone_into_arena(&mut a, &source as *const VowVec as *const u8) };
        let cv = unsafe { &*(cloned as *const VowVec) };
        assert_eq!(cv.len, 0);
        assert_eq!(vow_vec_capacity(cv), 0);
        unsafe { __vow_arena_close(&mut a) };
    }

    #[test]
    fn arena_alignment_respected() {
        let mut a = empty_arena_header();
        unsafe { __vow_arena_open(&mut a) };
        for &align in &[1usize, 2, 4, 8, 16] {
            let p = unsafe { __vow_arena_alloc(&mut a, 8, align) };
            assert_eq!(p as usize % align, 0, "pointer must be {align}-aligned");
        }
        unsafe { __vow_arena_close(&mut a) };
    }

    #[test]
    fn arena_large_alignment_takes_oversized_path() {
        // Small `bytes` with large `align` must route to the oversized path,
        // otherwise alignment padding could push `cursor > chunk_end`.
        let mut a = empty_arena_header();
        unsafe { __vow_arena_open(&mut a) };
        // Force the new-chunk path: bump cursor past first chunk.
        let _ = unsafe { __vow_arena_alloc(&mut a, 64, 8) };
        let p = unsafe { __vow_arena_alloc(&mut a, 9, 4096) };
        assert_eq!(p as usize % 4096, 0, "pointer must be 4096-aligned");
        assert!(a.cursor <= a.chunk_end, "cursor must not exceed chunk_end");
        assert!((p as usize) + 9 <= a.chunk_end);
        unsafe { __vow_arena_close(&mut a) };
    }

    #[test]
    fn arena_oversized_chunks_are_sealed_against_slack_reuse() {
        // Regression for the dangling-pointer scenario identified on PR #392:
        // an oversized allocation with significant alignment slack must NOT
        // host a subsequent smaller allocation in its tail. Otherwise
        // `arena_try_free_oversized_chunk` would later release the chunk
        // (classified oversized by `total > normal_chunk_total()`) while the
        // smaller allocation is still live, dangling its pointer.
        let mut a = empty_arena_header();
        unsafe { __vow_arena_open(&mut a) };

        // Force the oversized path for the 9-byte/4096-align allocation.
        // A 4096-byte/8-align prior alloc takes the oversized path itself
        // (>2048) and seals the arena cursor at its own chunk_end, so the
        // fast path inside the follow-up `alloc(9, 4096)` cannot satisfy
        // the request from any normal-chunk slack and the new-chunk branch
        // runs.
        let _filler = unsafe { __vow_arena_alloc(&mut a, 4096, 8) };
        // Sanity: the filler's chunk is itself sealed.
        assert_eq!(
            a.cursor, a.chunk_end,
            "the filler alloc must seal its own oversized chunk"
        );

        // Oversized via alignment slack: 9 bytes @ align 4096. With the seal,
        // ~4095 bytes of slack between `start + bytes` and `chunk_end` are
        // intentionally wasted to keep the chunk single-resident.
        let big_align_ptr = unsafe { __vow_arena_alloc(&mut a, 9, 4096) };
        let oversized_chunk = a.current_chunk;
        let oversized_chunk_end = a.chunk_end;
        assert_eq!(
            a.cursor, oversized_chunk_end,
            "oversized chunk must be sealed (cursor == chunk_end)"
        );

        // A modest follow-up allocation that would have fit in the slack
        // must now spill into a new chunk.
        let small = unsafe { __vow_arena_alloc(&mut a, 64, 8) };
        assert!(!small.is_null());
        assert_ne!(
            a.current_chunk, oversized_chunk,
            "subsequent allocation must take a new chunk, not the oversized slack"
        );
        // And the new chunk must not overlap the oversized payload.
        let small_addr = small as usize;
        let big_addr = big_align_ptr as usize;
        assert!(
            small_addr < big_addr || small_addr >= oversized_chunk_end,
            "subsequent allocation must live outside the oversized chunk"
        );

        unsafe { __vow_arena_close(&mut a) };
    }

    #[test]
    fn arena_close_walks_full_chain() {
        let mut a = empty_arena_header();
        unsafe { __vow_arena_open(&mut a) };
        // Force three chunks: oversized (own chunk) + normal + oversized.
        let _ = unsafe { __vow_arena_alloc(&mut a, 4096, 8) };
        let _ = unsafe { __vow_arena_alloc(&mut a, 100, 8) };
        let _ = unsafe { __vow_arena_alloc(&mut a, 8192, 8) };
        // If close fails to walk the chain, leak detectors (ASan/Miri) will flag;
        // functional success is that close completes without UB.
        unsafe { __vow_arena_close(&mut a) };
    }

    #[test]
    fn explicit_arena_map_new_allocates_in_supplied_arena() {
        let mut a = empty_arena_header();
        unsafe { __vow_arena_open(&mut a) };

        let cursor_before = a.cursor;
        let m = unsafe { __vow_map_new_in_arena(&mut a) };
        let cursor_after = a.cursor;

        assert!(!m.is_null(), "__vow_map_new_in_arena returned null");
        assert!(
            cursor_after > cursor_before,
            "arena cursor must advance for header + initial backing"
        );

        let header = unsafe { &*(m as *const VowMap) };
        assert_eq!(header.len, 0);
        assert_eq!(header.cap, MAP_INITIAL_CAP);
        assert!(!header.ptr.is_null(), "initial backing must be allocated");

        unsafe { __vow_arena_close(&mut a) };
    }

    fn map_get_pair(arena: &mut VowArena, m: *mut u8, key: i64) -> (i64, i64) {
        let opt = unsafe { __vow_map_get_in_arena(arena, m, key) } as *const i64;
        unsafe { (*opt, *opt.add(1)) }
    }

    #[test]
    fn map_get_reports_missing_key_as_none_not_zero() {
        let mut a = empty_arena_header();
        unsafe { __vow_arena_open(&mut a) };

        let m = unsafe { __vow_map_new_in_arena(&mut a) };
        unsafe { __vow_map_insert_in_arena(&mut a, m, 5, 0) };

        assert_eq!(map_get_pair(&mut a, m, 5), (1, 0), "bound to 0 is Some(0)");
        assert_eq!(map_get_pair(&mut a, m, 6), (0, 0), "missing is None");

        unsafe { __vow_arena_close(&mut a) };
    }

    #[test]
    fn explicit_arena_map_remove_decrements_len() {
        let mut a = empty_arena_header();
        unsafe { __vow_arena_open(&mut a) };

        let m = unsafe { __vow_map_new_in_arena(&mut a) };
        unsafe { __vow_map_insert_in_arena(&mut a, m, 1, 10) };
        unsafe { __vow_map_insert_in_arena(&mut a, m, 2, 20) };
        assert_eq!(unsafe { __vow_map_len(m) }, 2);

        unsafe { __vow_map_remove_in_arena(&mut a, m, 1) };
        assert_eq!(unsafe { __vow_map_len(m) }, 1);
        assert!(!unsafe { __vow_map_contains(m, 1) });
        assert_eq!(map_get_pair(&mut a, m, 2), (1, 20));
        assert_eq!(map_get_pair(&mut a, m, 1), (0, 0));

        unsafe { __vow_arena_close(&mut a) };
    }

    #[test]
    fn explicit_arena_map_insert_grows_past_initial_cap() {
        let mut a = empty_arena_header();
        unsafe { __vow_arena_open(&mut a) };

        let m = unsafe { __vow_map_new_in_arena(&mut a) };
        // Force a copy-fallback growth: insert MAP_INITIAL_CAP entries, then
        // an intervening alloc, then push the (cap+1)th entry. The intervening
        // alloc invalidates try_extend, so growth must allocate and copy.
        let n = (MAP_INITIAL_CAP + 4) as i64;
        for i in 0..(MAP_INITIAL_CAP as i64) {
            unsafe { __vow_map_insert_in_arena(&mut a, m, i, i * 100) };
        }
        let _intervening = unsafe { __vow_arena_alloc(&mut a, 16, 8) };
        for i in (MAP_INITIAL_CAP as i64)..n {
            unsafe { __vow_map_insert_in_arena(&mut a, m, i, i * 100) };
        }

        let header = unsafe { &*(m as *const VowMap) };
        assert_eq!(header.len, n as usize);
        assert!(header.cap > MAP_INITIAL_CAP, "cap must have doubled");
        for i in 0..n {
            assert_eq!(map_get_pair(&mut a, m, i), (1, i * 100));
        }

        unsafe { __vow_arena_close(&mut a) };
    }

    #[test]
    fn explicit_arena_map_insert_round_trips_through_get() {
        let mut a = empty_arena_header();
        unsafe { __vow_arena_open(&mut a) };

        let m = unsafe { __vow_map_new_in_arena(&mut a) };
        unsafe { __vow_map_insert_in_arena(&mut a, m, 7, 70) };
        unsafe { __vow_map_insert_in_arena(&mut a, m, 3, 30) };

        assert_eq!(unsafe { __vow_map_len(m) }, 2);
        assert_eq!(map_get_pair(&mut a, m, 7), (1, 70));
        assert_eq!(map_get_pair(&mut a, m, 3), (1, 30));
        assert_eq!(map_get_pair(&mut a, m, 99), (0, 0));
        assert!(unsafe { __vow_map_contains(m, 7) });
        assert!(!unsafe { __vow_map_contains(m, 99) });

        unsafe { __vow_arena_close(&mut a) };
    }

    // -----------------------------------------------------------------------
    // Runtime trap tests. These use the subprocess pattern:
    // rodata_trap_worker reruns itself with an env var and invokes the
    // appropriate trap path; it exits(1) via the trap. Parent tests spawn
    // the worker and assert exit status + stderr.
    // -----------------------------------------------------------------------

    fn make_rodata_vec_val() -> VowVec {
        VowVec {
            // never dereferenced; trap fires first
            ptr: std::ptr::dangling_mut::<u8>(),
            len: 0,
            cap: VOW_CAP_RODATA,
        }
    }

    fn make_rodata_map() -> VowMap {
        VowMap {
            ptr: std::ptr::dangling_mut::<u8>(),
            len: 0,
            cap: VOW_CAP_RODATA,
            owner: std::ptr::null_mut(),
        }
    }

    /// Worker test: when `VOW_RODATA_TRAP_OP` is set, dispatches to the named
    /// mutation which must trap with RegionLiteralMutation. Otherwise a no-op
    /// so ordinary `cargo test` runs don't crash the test binary.
    #[test]
    fn rodata_trap_worker() {
        let Ok(op) = std::env::var("VOW_RODATA_TRAP_OP") else {
            return;
        };
        // Arena-overflow branch: exercises the size-limit guard in
        // __vow_arena_alloc without touching descriptor state.
        if op == "arena_alloc_overflow" {
            let mut arena = empty_arena_header();
            unsafe { __vow_arena_open(&mut arena) };
            let _ = unsafe { __vow_arena_alloc(&mut arena, usize::MAX, 8) };
            eprintln!("rodata_trap_worker: arena overflow did NOT trap");
            std::process::exit(42);
        }
        // Vec::reserve growth-overflow branch: reserving usize::MAX elements
        // must trap OutOfMemory via the checked growth arithmetic rather than
        // wrap new_cap to 0 and spin forever (issue #435).
        if op == "vec_reserve_overflow" {
            let mut arena = empty_arena_header();
            unsafe { __vow_arena_open(&mut arena) };
            let mut v = VowVec {
                ptr: 8 as *mut u8,
                len: 0,
                cap: 0,
            };
            unsafe {
                __vow_vec_reserve_in_arena(
                    &mut arena,
                    &mut v as *mut _ as *mut u8,
                    usize::MAX,
                    8,
                    8,
                )
            };
            eprintln!("rodata_trap_worker: vec reserve overflow did NOT trap");
            std::process::exit(42);
        }
        if op == "perf_vec_sort_use_after_free" {
            let vec = VowVec {
                ptr: std::ptr::dangling_mut(),
                len: 4,
                cap: 4,
            };
            let vec_addr = &raw const vec as usize;
            __vow_sanitize_init();
            // The guard must be dropped before the call below: it re-locks
            // SHADOW_TABLE via sanitize_on_read, so flattening this scope
            // deadlocks the worker.
            {
                let mut table = SHADOW_TABLE.lock().unwrap();
                shadow_table_get_or_init(&mut table).insert(
                    vec_addr,
                    ShadowVec {
                        generations: Vec::new(),
                        freed: true,
                    },
                );
            }
            unsafe { __vow_perf_count_vec_sort(vec_addr as *const u8) };
            eprintln!("rodata_trap_worker: perf Vec::sort UAF did NOT trap");
            std::process::exit(42);
        }
        // The arena primitives are exported C entry points in their own
        // right: generated code can call them without going through a
        // `*_in_arena` wrapper, so each must trap on a null handle rather
        // than reborrow it.
        if op == "arena_init_closed_null" {
            unsafe { __vow_arena_init_closed(std::ptr::null_mut()) };
            eprintln!("rodata_trap_worker: null arena init_closed did NOT trap");
            std::process::exit(42);
        }
        if op == "arena_open_null" {
            unsafe { __vow_arena_open(std::ptr::null_mut()) };
            eprintln!("rodata_trap_worker: null arena open did NOT trap");
            std::process::exit(42);
        }
        if op == "arena_close_null" {
            unsafe { __vow_arena_close(std::ptr::null_mut()) };
            eprintln!("rodata_trap_worker: null arena close did NOT trap");
            std::process::exit(42);
        }
        if op == "arena_alloc_null" {
            let _ = unsafe { __vow_arena_alloc(std::ptr::null_mut(), 8, 8) };
            eprintln!("rodata_trap_worker: null arena alloc did NOT trap");
            std::process::exit(42);
        }
        if op == "arena_try_extend_null" {
            let _ = unsafe { __vow_arena_try_extend(std::ptr::null_mut(), 8 as *mut u8, 8, 16) };
            eprintln!("rodata_trap_worker: null arena try_extend did NOT trap");
            std::process::exit(42);
        }
        // A null chunk base means the intrusive chain is corrupt. The
        // accessors test for it themselves so the walk that lost the
        // invariant fails closed instead of reading address 0.
        if op == "arena_next_chunk_null" {
            let _ = unsafe { next_chunk(std::ptr::null_mut()) };
            eprintln!("rodata_trap_worker: null chunk next_chunk did NOT trap");
            std::process::exit(42);
        }
        if op == "arena_set_next_chunk_null" {
            unsafe { set_next_chunk(std::ptr::null_mut(), std::ptr::null_mut()) };
            eprintln!("rodata_trap_worker: null chunk set_next_chunk did NOT trap");
            std::process::exit(42);
        }
        if op == "arena_chunk_total_null" {
            let _ = unsafe { chunk_total(std::ptr::null()) };
            eprintln!("rodata_trap_worker: null chunk chunk_total did NOT trap");
            std::process::exit(42);
        }
        if op == "arena_chunk_is_oversized_null" {
            let _ = unsafe { chunk_is_oversized(std::ptr::null()) };
            eprintln!("rodata_trap_worker: null chunk chunk_is_oversized did NOT trap");
            std::process::exit(42);
        }
        if op == "arena_set_chunk_total_null" {
            unsafe { set_chunk_total_word(std::ptr::null_mut(), 16) };
            eprintln!("rodata_trap_worker: null chunk set_chunk_total did NOT trap");
            std::process::exit(42);
        }
        if op == "Vec::new_in_arena_null" {
            let _ = unsafe { __vow_vec_new_in_arena(std::ptr::null_mut(), 8, 8) };
            eprintln!("rodata_trap_worker: null arena constructor did NOT trap");
            std::process::exit(42);
        }
        if op == "Vec::push_in_arena_null" {
            let mut v = VowVec {
                ptr: 8 as *mut u8,
                len: 0,
                cap: 0,
            };
            let elem = 0_i64;
            unsafe {
                __vow_vec_push_in_arena(
                    std::ptr::null_mut(),
                    &mut v as *mut _ as *mut u8,
                    &elem as *const _ as *const u8,
                    8,
                    8,
                )
            };
            eprintln!("rodata_trap_worker: null arena push did NOT trap");
            std::process::exit(42);
        }
        if op == "String::new_in_arena_null" {
            let _ = unsafe { __vow_string_new_in_arena(std::ptr::null_mut(), c"x".as_ptr(), 1) };
            eprintln!("rodata_trap_worker: null arena string constructor did NOT trap");
            std::process::exit(42);
        }
        if op == "String::from_cstr_in_arena_null" {
            let _ = unsafe { __vow_string_from_cstr_in_arena(std::ptr::null_mut(), c"x".as_ptr()) };
            eprintln!("rodata_trap_worker: null arena string from_cstr did NOT trap");
            std::process::exit(42);
        }
        if op == "String::clone_in_arena_null" {
            let _ = unsafe { __vow_string_clone_in_arena(std::ptr::null_mut(), std::ptr::null()) };
            eprintln!("rodata_trap_worker: null arena string clone did NOT trap");
            std::process::exit(42);
        }
        if op == "String::push_str_in_arena_null" {
            let mut dest = VowVec {
                ptr: 8 as *mut u8,
                len: 0,
                cap: 0,
            };
            let src = VowVec {
                ptr: 8 as *mut u8,
                len: 0,
                cap: 0,
            };
            unsafe {
                __vow_string_push_str_in_arena(
                    std::ptr::null_mut(),
                    &mut dest as *mut _ as *mut u8,
                    &src as *const _ as *const u8,
                )
            };
            eprintln!("rodata_trap_worker: null arena string push_str did NOT trap");
            std::process::exit(42);
        }
        if op == "String::push_byte_in_arena_null" {
            let mut s = VowVec {
                ptr: 8 as *mut u8,
                len: 0,
                cap: 0,
            };
            unsafe {
                __vow_string_push_byte_in_arena(
                    std::ptr::null_mut(),
                    &mut s as *mut _ as *mut u8,
                    b'x' as u64,
                )
            };
            eprintln!("rodata_trap_worker: null arena string push_byte did NOT trap");
            std::process::exit(42);
        }
        if op == "String::substr_in_arena_null" {
            let _ = unsafe {
                __vow_string_substr_in_arena(std::ptr::null_mut(), std::ptr::null(), 0, 0)
            };
            eprintln!("rodata_trap_worker: null arena string substr did NOT trap");
            std::process::exit(42);
        }
        if op == "String::substring_in_arena_null" {
            let _ = unsafe {
                __vow_string_substring_in_arena(std::ptr::null_mut(), std::ptr::null(), 0, 0)
            };
            eprintln!("rodata_trap_worker: null arena string substring did NOT trap");
            std::process::exit(42);
        }
        if op == "String::from_i64_in_arena_null" {
            let _ = unsafe { __vow_string_from_i64_in_arena(std::ptr::null_mut(), 1) };
            eprintln!("rodata_trap_worker: null arena string from_i64 did NOT trap");
            std::process::exit(42);
        }
        if op == "String::from_u64_in_arena_null" {
            let _ = unsafe { __vow_string_from_u64_in_arena(std::ptr::null_mut(), 1) };
            eprintln!("rodata_trap_worker: null arena string from_u64 did NOT trap");
            std::process::exit(42);
        }
        if op == "String::split_in_arena_null" {
            let _ = unsafe {
                __vow_string_split_in_arena(
                    std::ptr::null_mut(),
                    std::ptr::null(),
                    std::ptr::null(),
                )
            };
            eprintln!("rodata_trap_worker: null arena string split did NOT trap");
            std::process::exit(42);
        }
        if op == "String::trim_in_arena_null" {
            let _ = unsafe { __vow_string_trim_in_arena(std::ptr::null_mut(), std::ptr::null()) };
            eprintln!("rodata_trap_worker: null arena string trim did NOT trap");
            std::process::exit(42);
        }
        if op == "String::to_upper_in_arena_null" {
            let _ =
                unsafe { __vow_string_to_upper_in_arena(std::ptr::null_mut(), std::ptr::null()) };
            eprintln!("rodata_trap_worker: null arena string to_upper did NOT trap");
            std::process::exit(42);
        }
        if op == "String::to_lower_in_arena_null" {
            let _ =
                unsafe { __vow_string_to_lower_in_arena(std::ptr::null_mut(), std::ptr::null()) };
            eprintln!("rodata_trap_worker: null arena string to_lower did NOT trap");
            std::process::exit(42);
        }
        if op == "String::replace_in_arena_null" {
            let _ = unsafe {
                __vow_string_replace_in_arena(
                    std::ptr::null_mut(),
                    std::ptr::null(),
                    std::ptr::null(),
                    std::ptr::null(),
                )
            };
            eprintln!("rodata_trap_worker: null arena string replace did NOT trap");
            std::process::exit(42);
        }
        if op == "String::join_in_arena_null" {
            let _ = unsafe {
                __vow_string_join_in_arena(std::ptr::null_mut(), std::ptr::null(), std::ptr::null())
            };
            eprintln!("rodata_trap_worker: null arena string join did NOT trap");
            std::process::exit(42);
        }
        if op == "HashMap::new_in_arena_null" {
            let _ = unsafe { __vow_map_new_in_arena(std::ptr::null_mut()) };
            eprintln!("rodata_trap_worker: null arena map new did NOT trap");
            std::process::exit(42);
        }
        if op == "HashMap::grow_null_owner" || op == "BTreeMap::grow_null_owner" {
            let mut a = empty_arena_header();
            unsafe { __vow_arena_open(&mut a) };
            if op == "HashMap::grow_null_owner" {
                let m = unsafe { __vow_map_new_in_arena(&mut a) };
                unsafe { (*(m as *mut VowMap)).owner = std::ptr::null_mut() };
                for i in 0..=MAP_INITIAL_CAP as i64 {
                    unsafe { __vow_map_insert_in_arena(&mut a, m, i, i) };
                }
            } else {
                let m = unsafe { __vow_btreemap_new_in_arena(&mut a) };
                unsafe { (*(m as *mut VowBTreeMap)).owner = std::ptr::null_mut() };
                for i in 0..=BTREEMAP_INITIAL_CAP as i64 {
                    unsafe { __vow_btreemap_insert_in_arena(&mut a, m, i, i) };
                }
            }
            eprintln!("rodata_trap_worker: null owner map growth did NOT trap");
            std::process::exit(42);
        }
        if op == "option_cells_shadow_untracked" {
            __vow_sanitize_init();
            let mut a = empty_arena_header();
            let ap: *mut VowArena = &mut a;
            unsafe { __vow_arena_open(ap) };
            let m = unsafe { __vow_btreemap_new_in_arena(ap) };
            let hm = unsafe { __vow_map_new_in_arena(ap) };
            let cells = [
                unsafe { __vow_btreemap_insert_in_arena(ap, m, 1, 2) },
                unsafe { __vow_btreemap_get_in_arena(ap, m, 1) },
                unsafe { __vow_map_get_in_arena(ap, hm, 1) },
                unsafe { __vow_i64_to_u8_try_in_arena(ap, 3) },
            ];
            let control = unsafe { __vow_vec_new_in_arena(ap, 8, 8) };
            let mut table = SHADOW_TABLE.lock().unwrap();
            let shadows = shadow_table_get_or_init(&mut table);
            if !shadows.contains_key(&(control as usize)) {
                eprintln!("worker: sanitize shadow tracking is not active for Vecs");
                std::process::exit(44);
            }
            for cell in cells {
                if shadows.contains_key(&(cell as usize)) {
                    eprintln!("worker: Option cell {cell:p} is shadow-tracked");
                    std::process::exit(43);
                }
            }
            std::process::exit(0);
        }
        if op == "HashMap::get_in_arena_null" {
            let mut m = VowMap {
                ptr: 8 as *mut u8,
                len: 0,
                cap: 0,
                owner: std::ptr::null_mut(),
            };
            let _ = unsafe {
                __vow_map_get_in_arena(std::ptr::null_mut(), &mut m as *mut _ as *mut u8, 1)
            };
            eprintln!("rodata_trap_worker: null arena map get did NOT trap");
            std::process::exit(42);
        }
        if op == "BTreeMap::new_in_arena_null" {
            let _ = unsafe { __vow_btreemap_new_in_arena(std::ptr::null_mut()) };
            eprintln!("rodata_trap_worker: null arena btreemap new did NOT trap");
            std::process::exit(42);
        }
        if op == "BTreeMap::get_in_arena_null" || op == "BTreeMap::insert_in_arena_null" {
            let mut a = empty_arena_header();
            unsafe { __vow_arena_open(&mut a) };
            let m = unsafe { __vow_btreemap_new_in_arena(&mut a) };
            if op == "BTreeMap::get_in_arena_null" {
                let _ = unsafe { __vow_btreemap_get_in_arena(std::ptr::null_mut(), m, 1) };
            } else {
                let _ = unsafe { __vow_btreemap_insert_in_arena(std::ptr::null_mut(), m, 1, 1) };
            }
            eprintln!("rodata_trap_worker: null arena btreemap op did NOT trap");
            std::process::exit(42);
        }
        if op == "Option::parse_in_arena_null" {
            let _ = unsafe {
                __vow_string_parse_u64_opt_in_arena(std::ptr::null_mut(), std::ptr::null())
            };
            eprintln!("rodata_trap_worker: null arena parse did NOT trap");
            std::process::exit(42);
        }
        if op == "Option::try_in_arena_null" {
            let _ = unsafe { __vow_i64_to_u8_try_in_arena(std::ptr::null_mut(), 7) };
            eprintln!("rodata_trap_worker: null arena narrowing did NOT trap");
            std::process::exit(42);
        }
        if op == "HashMap::insert_in_arena_null" {
            let mut m = VowMap {
                ptr: 8 as *mut u8,
                len: 0,
                cap: 0,
                owner: std::ptr::null_mut(),
            };
            unsafe {
                __vow_map_insert_in_arena(std::ptr::null_mut(), &mut m as *mut _ as *mut u8, 1, 1)
            };
            eprintln!("rodata_trap_worker: null arena map insert did NOT trap");
            std::process::exit(42);
        }
        if op == "HashMap::remove_in_arena_null" {
            let mut m = VowMap {
                ptr: 8 as *mut u8,
                len: 0,
                cap: 0,
                owner: std::ptr::null_mut(),
            };
            unsafe {
                __vow_map_remove_in_arena(std::ptr::null_mut(), &mut m as *mut _ as *mut u8, 1)
            };
            eprintln!("rodata_trap_worker: null arena map remove did NOT trap");
            std::process::exit(42);
        }
        let mut v = make_rodata_vec_val();
        let vp = &mut v as *mut _ as *mut u8;
        let mut m = make_rodata_map();
        let mp = &mut m as *mut _ as *mut u8;
        match op.as_str() {
            "Vec::reserve" => unsafe { __vow_vec_reserve(vp, 1, 8, 8) },
            "Vec::push" => {
                let elem: i64 = 0;
                unsafe { __vow_vec_push(vp, &elem as *const _ as *const u8, 8, 8) };
            }
            "Vec::push_val" => unsafe { __vow_vec_push_val(vp, 0) },
            "Vec::pop" => unsafe { __vow_vec_pop(vp) },
            "Vec::clear" => unsafe { __vow_vec_clear(vp) },
            "Vec::truncate" => unsafe { __vow_vec_truncate(vp, 0) },
            "Vec::set" => unsafe { __vow_vec_set_val(vp, 0, 0) },
            "String::clear" => unsafe { __vow_string_clear(vp) },
            "String::push_str" => {
                let mut src = make_rodata_vec_val();
                src.cap = 0; // source must not trap; destination is the rodata one
                unsafe { __vow_string_push_str(vp, &src as *const _ as *const u8) };
            }
            "String::push_byte" => unsafe { __vow_string_push_byte(vp, 0x61) },
            "String::push_byte_in_candidate_arena" => {
                let mut a = empty_arena_header();
                unsafe { __vow_arena_open(&mut a) };
                unsafe { __vow_string_push_byte_in_candidate_arena(&mut a, vp, 0x61) };
            }
            "HashMap::insert" => unsafe { __vow_map_insert(mp, 1, 2) },
            "HashMap::remove" => unsafe { __vow_map_remove(mp, 1) },
            "HashMap::insert_in_arena" => {
                let mut a = empty_arena_header();
                unsafe { __vow_arena_open(&mut a) };
                unsafe { __vow_map_insert_in_arena(&mut a, mp, 1, 2) };
            }
            "HashMap::remove_in_arena" => {
                let mut a = empty_arena_header();
                unsafe { __vow_arena_open(&mut a) };
                unsafe { __vow_map_remove_in_arena(&mut a, mp, 1) };
            }
            other => panic!("unknown trap op: {other}"),
        }
        // Should be unreachable — each branch must trap.
        eprintln!("rodata_trap_worker: did NOT trap for op={op}");
        std::process::exit(42);
    }

    fn spawn_trap_worker(op: &str) -> (std::process::Output, String) {
        use std::io::Read;
        let exe = std::env::current_exe().expect("current_exe");
        let mut child = std::process::Command::new(exe)
            .args(["tests::rodata_trap_worker", "--exact", "--nocapture"])
            .env("VOW_RODATA_TRAP_OP", op)
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped())
            .spawn()
            .expect("spawn worker");

        // Drain stdout/stderr on threads so a wedged worker can't deadlock on
        // a full pipe buffer while we poll for its exit.
        let stdout_handle = child.stdout.take();
        let stderr_handle = child.stderr.take();
        let stdout_thread = std::thread::spawn(move || {
            let mut buf = Vec::new();
            if let Some(mut r) = stdout_handle {
                let _ = r.read_to_end(&mut buf);
            }
            buf
        });
        let stderr_thread = std::thread::spawn(move || {
            let mut buf = Vec::new();
            if let Some(mut r) = stderr_handle {
                let _ = r.read_to_end(&mut buf);
            }
            buf
        });

        // Every trap worker exits within milliseconds. A worker still alive
        // after this bound means the failure being guarded against (e.g. the
        // issue #435 Vec::reserve infinite loop) has regressed: kill it and
        // fail the test, rather than block on output() until the CI job-level
        // timeout hangs the whole suite.
        let timeout = std::time::Duration::from_secs(60);
        let start = std::time::Instant::now();
        let status = loop {
            match child.try_wait().expect("try_wait on worker") {
                Some(status) => break status,
                None => {
                    if start.elapsed() >= timeout {
                        let _ = child.kill();
                        let _ = child.wait();
                        panic!(
                            "trap worker for {op} did not exit within {timeout:?}; \
                             likely reintroduced an infinite loop (issue #435)"
                        );
                    }
                    std::thread::sleep(std::time::Duration::from_millis(10));
                }
            }
        };

        let stdout = stdout_thread.join().unwrap_or_default();
        let stderr_bytes = stderr_thread.join().unwrap_or_default();
        let stderr = String::from_utf8_lossy(&stderr_bytes).to_string();
        (
            std::process::Output {
                status,
                stdout,
                stderr: stderr_bytes,
            },
            stderr,
        )
    }

    fn assert_rodata_trap(op: &str, expected_op_in_json: &str) {
        let (out, stderr) = spawn_trap_worker(op);
        assert_eq!(
            out.status.code(),
            Some(VOW_RUNTIME_ABORT_EXIT),
            "worker for {op} should exit with the reserved runtime-abort code (#877); stderr:\n{stderr}"
        );
        assert!(
            stderr.contains(r#""error":"RegionLiteralMutation""#),
            "stderr missing RegionLiteralMutation for {op}:\n{stderr}"
        );
        assert!(
            stderr.contains(&format!(r#""operation":"{expected_op_in_json}""#)),
            "stderr missing operation={expected_op_in_json}:\n{stderr}"
        );
        assert!(
            stderr.contains(r#""origin":"rodata""#),
            "stderr missing origin=rodata:\n{stderr}"
        );
        assert!(
            stderr.contains("hint: use String::from(literal)")
                || stderr.contains("hint: use Vec::from(literal)")
                || stderr.contains("hint: construct a mutable HashMap"),
            "stderr missing hint line:\n{stderr}"
        );
        if expected_op_in_json.starts_with("String::") || expected_op_in_json.starts_with("Vec::") {
            assert!(
                stderr.contains("pin_to_root(value)"),
                "stderr missing pin_to_root hint for read-only heap value:\n{stderr}"
            );
        }
    }

    fn assert_runtime_invariant(op: &str, expected_op_in_json: &str, expected_reason: &str) {
        let (out, stderr) = spawn_trap_worker(op);
        assert_eq!(
            out.status.code(),
            Some(VOW_RUNTIME_ABORT_EXIT),
            "worker for {op} should exit with the reserved runtime-abort code (#877); stderr:\n{stderr}"
        );
        assert!(
            stderr.contains(r#""error":"RuntimeInvariantViolation""#),
            "stderr missing RuntimeInvariantViolation for {op}:\n{stderr}"
        );
        assert!(
            stderr.contains(&format!(r#""operation":"{expected_op_in_json}""#)),
            "stderr missing operation={expected_op_in_json}:\n{stderr}"
        );
        assert!(
            stderr.contains(&format!(r#""reason":"{expected_reason}""#)),
            "stderr missing reason={expected_reason}:\n{stderr}"
        );
    }

    fn assert_runtime_invariant_null_arena(op: &str, expected_op_in_json: &str) {
        assert_runtime_invariant(op, expected_op_in_json, "null arena");
    }

    #[test]
    fn perf_vec_sort_cost_adapter_checks_sanitizer_before_reading_header() {
        let (out, stderr) = spawn_trap_worker("perf_vec_sort_use_after_free");
        assert_eq!(
            out.status.code(),
            Some(VOW_RUNTIME_ABORT_EXIT),
            "cost adapter should reject a freed Vec descriptor; stderr:\n{stderr}"
        );
        assert!(
            stderr.contains(r#""error":"UseAfterFree""#),
            "stderr missing UseAfterFree for Vec::sort cost adapter:\n{stderr}"
        );
        assert!(
            stderr.contains(r#""op":"read""#),
            "stderr missing read operation for Vec::sort cost adapter:\n{stderr}"
        );
    }

    #[test]
    fn arena_alloc_rejects_overflow() {
        // Verifies the isize::MAX size-limit guard: passing bytes=usize::MAX
        // must trap OutOfMemory rather than wrap and return a garbage
        // pointer.
        let (out, stderr) = spawn_trap_worker("arena_alloc_overflow");
        assert_eq!(
            out.status.code(),
            Some(VOW_RUNTIME_ABORT_EXIT),
            "worker should exit with the reserved runtime-abort code (#877); stderr:\n{stderr}"
        );
        assert!(
            stderr.contains(r#""error":"OutOfMemory""#),
            "stderr missing OutOfMemory trap:\n{stderr}"
        );
    }

    #[test]
    fn vec_reserve_rejects_overflow() {
        // Verifies the checked growth arithmetic in vec_reserve_in_arena
        // (issue #435): reserving usize::MAX elements must trap OutOfMemory
        // within a bounded time rather than wrap new_cap to 0 and spin
        // forever. The worker runs in a subprocess so a regression that
        // reintroduced the infinite loop surfaces as a timeout, not a hang
        // of the whole suite.
        let (out, stderr) = spawn_trap_worker("vec_reserve_overflow");
        assert_eq!(
            out.status.code(),
            Some(VOW_RUNTIME_ABORT_EXIT),
            "worker should exit with the reserved runtime-abort code (#877); stderr:\n{stderr}"
        );
        assert!(
            stderr.contains(r#""error":"OutOfMemory""#),
            "stderr missing OutOfMemory trap:\n{stderr}"
        );
        assert!(
            stderr.contains(r#""operation":"Vec::reserve""#),
            "stderr missing operation=Vec::reserve:\n{stderr}"
        );
    }

    #[test]
    fn explicit_arena_vec_new_null_arena_traps() {
        assert_runtime_invariant_null_arena("Vec::new_in_arena_null", "Vec::new");
    }

    #[test]
    fn explicit_arena_vec_push_null_arena_traps() {
        assert_runtime_invariant_null_arena("Vec::push_in_arena_null", "Vec::push");
    }

    #[test]
    fn explicit_arena_string_new_null_arena_traps() {
        assert_runtime_invariant_null_arena("String::new_in_arena_null", "String::new");
    }

    #[test]
    fn explicit_arena_string_from_cstr_null_arena_traps() {
        assert_runtime_invariant_null_arena("String::from_cstr_in_arena_null", "String::from_cstr");
    }

    #[test]
    fn explicit_arena_string_clone_null_arena_traps() {
        assert_runtime_invariant_null_arena("String::clone_in_arena_null", "String::clone");
    }

    #[test]
    fn explicit_arena_string_push_str_null_arena_traps() {
        assert_runtime_invariant_null_arena("String::push_str_in_arena_null", "String::push_str");
    }

    #[test]
    fn explicit_arena_string_push_byte_null_arena_traps() {
        assert_runtime_invariant_null_arena("String::push_byte_in_arena_null", "String::push_byte");
    }

    #[test]
    fn explicit_arena_string_substr_null_arena_traps() {
        assert_runtime_invariant_null_arena("String::substr_in_arena_null", "String::substr");
    }

    #[test]
    fn explicit_arena_string_substring_null_arena_traps() {
        assert_runtime_invariant_null_arena("String::substring_in_arena_null", "String::substring");
    }

    #[test]
    fn explicit_arena_string_from_i64_null_arena_traps() {
        assert_runtime_invariant_null_arena("String::from_i64_in_arena_null", "String::from_i64");
    }

    #[test]
    fn explicit_arena_string_from_u64_null_arena_traps() {
        assert_runtime_invariant_null_arena("String::from_u64_in_arena_null", "String::from_u64");
    }

    #[test]
    fn explicit_arena_map_new_null_arena_traps() {
        assert_runtime_invariant_null_arena("HashMap::new_in_arena_null", "HashMap::new");
    }

    #[test]
    fn explicit_arena_map_insert_null_arena_traps() {
        assert_runtime_invariant_null_arena("HashMap::insert_in_arena_null", "HashMap::insert");
    }

    #[test]
    fn option_returning_builtins_trap_on_null_arena() {
        let cases = [
            ("HashMap::get_in_arena_null", "HashMap::get"),
            ("BTreeMap::new_in_arena_null", "BTreeMap::new"),
            ("BTreeMap::get_in_arena_null", "BTreeMap::get"),
            ("BTreeMap::insert_in_arena_null", "BTreeMap::insert"),
            (
                "Option::parse_in_arena_null",
                "__vow_string_parse_u64_opt_in_arena",
            ),
            ("Option::try_in_arena_null", "__vow_i64_to_u8_try_in_arena"),
        ];
        for (op, expected) in cases {
            assert_runtime_invariant_null_arena(op, expected);
        }
    }

    #[test]
    fn map_growth_traps_on_a_null_owner_arena() {
        assert_runtime_invariant("HashMap::grow_null_owner", "map growth", "null arena");
        assert_runtime_invariant("BTreeMap::grow_null_owner", "map growth", "null arena");
    }

    #[test]
    fn explicit_arena_map_remove_null_arena_traps() {
        assert_runtime_invariant_null_arena("HashMap::remove_in_arena_null", "HashMap::remove");
    }

    #[test]
    fn arena_primitive_null_arena_traps() {
        let cases = [
            ("arena_init_closed_null", "arena_init_closed"),
            ("arena_open_null", "arena_open"),
            ("arena_close_null", "arena_close"),
            ("arena_alloc_null", "arena_alloc"),
            ("arena_try_extend_null", "arena_try_extend"),
        ];
        for (op, expected) in cases {
            assert_runtime_invariant_null_arena(op, expected);
        }
    }

    #[test]
    fn chunk_header_accessor_null_chunk_traps() {
        let cases = [
            ("arena_next_chunk_null", "arena_next_chunk"),
            ("arena_set_next_chunk_null", "arena_set_next_chunk"),
            ("arena_chunk_total_null", "arena_chunk_total"),
            ("arena_chunk_is_oversized_null", "arena_chunk_is_oversized"),
            ("arena_set_chunk_total_null", "arena_set_chunk_total"),
        ];
        for (op, expected) in cases {
            assert_runtime_invariant(op, expected, "null chunk");
        }
    }

    #[test]
    fn explicit_arena_string_fresh_helper_null_arena_traps() {
        let cases = [
            ("String::split_in_arena_null", "String::split"),
            ("String::trim_in_arena_null", "String::trim"),
            ("String::to_upper_in_arena_null", "String::to_upper"),
            ("String::to_lower_in_arena_null", "String::to_lower"),
            ("String::replace_in_arena_null", "String::replace"),
            ("String::join_in_arena_null", "String::join"),
        ];
        for (op, expected) in cases {
            assert_runtime_invariant_null_arena(op, expected);
        }
    }

    #[test]
    fn rodata_vec_reserve_traps() {
        assert_rodata_trap("Vec::reserve", "Vec::reserve");
    }
    /// Acceptance test 4 from issue #198: `VOW_CAP_RODATA` mutation via
    /// `Vec::push` on a literal-backed descriptor traps with
    /// `RegionLiteralMutation` before the allocation logic is reached
    /// (spec §6.1, §7.3).
    #[test]
    fn rodata_vec_push_traps() {
        assert_rodata_trap("Vec::push", "Vec::push");
    }
    #[test]
    fn rodata_vec_push_val_traps() {
        assert_rodata_trap("Vec::push_val", "Vec::push_val");
    }
    #[test]
    fn rodata_vec_pop_traps() {
        assert_rodata_trap("Vec::pop", "Vec::pop");
    }
    #[test]
    fn rodata_vec_clear_traps() {
        assert_rodata_trap("Vec::clear", "Vec::clear");
    }
    #[test]
    fn rodata_vec_truncate_traps() {
        assert_rodata_trap("Vec::truncate", "Vec::truncate");
    }
    #[test]
    fn rodata_vec_set_traps() {
        assert_rodata_trap("Vec::set", "Vec::set");
    }
    #[test]
    fn rodata_string_clear_traps() {
        assert_rodata_trap("String::clear", "String::clear");
    }
    #[test]
    fn rodata_string_push_str_traps() {
        assert_rodata_trap("String::push_str", "String::push_str");
    }
    #[test]
    fn rodata_string_push_byte_traps() {
        assert_rodata_trap("String::push_byte", "String::push_byte");
    }
    #[test]
    fn rodata_string_candidate_push_byte_traps_without_reading_owner_prefix() {
        assert_rodata_trap("String::push_byte_in_candidate_arena", "String::push_byte");
    }
    #[test]
    fn rodata_map_insert_traps() {
        assert_rodata_trap("HashMap::insert", "HashMap::insert");
    }
    #[test]
    fn rodata_map_insert_in_arena_traps() {
        assert_rodata_trap("HashMap::insert_in_arena", "HashMap::insert");
    }
    #[test]
    fn rodata_map_remove_traps() {
        assert_rodata_trap("HashMap::remove", "HashMap::remove");
    }
    #[test]
    fn rodata_map_remove_in_arena_traps() {
        assert_rodata_trap("HashMap::remove_in_arena", "HashMap::remove");
    }

    #[test]
    fn rodata_lazy_empty_still_works() {
        // cap == 0 (lazy-empty) must NOT be mistaken for VOW_CAP_RODATA.
        let v = __vow_vec_new_val();
        unsafe { __vow_vec_push_val(v, 42) };
        let vec = unsafe { &*(v as *const VowVec) };
        assert_eq!(vec.len, 1);
        assert!(
            vow_vec_capacity(vec) >= 1,
            "lazy-allocated, cap should be populated"
        );
    }

    fn make_test_string(s: &str) -> *mut u8 {
        unsafe { __vow_string_new(s.as_ptr() as *const c_char, s.len()) }
    }

    #[test]
    fn parse_f64_bits_null_pointer_returns_zero() {
        let bits = unsafe { __vow_parse_f64_bits(std::ptr::null()) };
        assert_eq!(bits, 0);
    }

    #[test]
    fn parse_f64_bits_round_trips_known_values() {
        for text in ["1.5", "0.0", "3.14159", "100.25"] {
            let s = make_test_string(text);
            let bits = unsafe { __vow_parse_f64_bits(s) };
            let expected: f64 = text.parse().unwrap();
            assert_eq!(bits, expected.to_bits(), "parsing {text}");
        }
    }

    #[test]
    fn format_f64_bits_round_trips_known_values() {
        for value in [1.5f64, 0.0, 7.123456, 100.25] {
            let ptr = unsafe { __vow_format_f64_bits(value.to_bits()) };
            let v = unsafe { &*(ptr as *const VowVec) };
            let bytes = unsafe { std::slice::from_raw_parts(v.ptr, v.len) };
            assert_eq!(std::str::from_utf8(bytes).unwrap(), value.to_string());
        }
    }

    #[test]
    fn parse_then_format_f64_bits_round_trips_text() {
        let s = make_test_string("2.0");
        let bits = unsafe { __vow_parse_f64_bits(s) };
        let ptr = unsafe { __vow_format_f64_bits(bits) };
        let v = unsafe { &*(ptr as *const VowVec) };
        let bytes = unsafe { std::slice::from_raw_parts(v.ptr, v.len) };
        assert_eq!(std::str::from_utf8(bytes).unwrap(), "2");
    }
}
