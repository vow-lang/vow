#![allow(clippy::missing_safety_doc)]

use std::cell::RefCell;
use std::collections::HashMap;
use std::ffi::{CStr, c_char};
use std::io::Write as _;
use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, AtomicI64, AtomicU64, AtomicUsize, Ordering};

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

const TAG_I32: u8 = 0;
const TAG_I64: u8 = 1;
const TAG_F32: u8 = 2;
const TAG_F64: u8 = 3;
const TAG_BOOL: u8 = 4;
const TAG_U64: u8 = 5;
const TAG_U8: u8 = 6;

/// Reserved process exit status for any runtime abort — a contract
/// violation, checked-arithmetic overflow, unwrap-on-None, index-out-of-bounds,
/// region-literal mutation, runtime-invariant violation, sanitizer trap, stack
/// overflow, or out-of-memory. A runtime abort is an environment/soundness
/// failure, never an application result, so it must terminate with a status
/// that cannot be confused with an application's own `return N` from `main`
/// (issue #877). 134 = 128 + SIGABRT, the conventional "aborted" status; it is
/// what the stack-overflow handler and `__vow_malloc` failure already use, so
/// this unifies every runtime abort on one reserved code. The structured JSON
/// envelope written to stderr still identifies which abort occurred.
const VOW_RUNTIME_ABORT_EXIT: i32 = 134;

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
        TAG_U8 => format!("{}", payload as u8),
        _ => format!("0x{payload:x}"),
    }
}

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
    std::process::exit(VOW_RUNTIME_ABORT_EXIT);
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn __vow_print_str(s: *const u8) {
    sanitize_on_read(s as usize, 0);
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
                if counters.is_empty() {
                    return;
                }
                let mut entries: Vec<_> = counters
                    .iter()
                    .map(|(name, count)| (*name, *count))
                    .collect();
                entries.sort_by_key(|k| std::cmp::Reverse(k.1));
                let total: u64 = entries.iter().map(|item| item.1).sum();
                let _ = writeln!(std::io::stderr(), "\n--- vow profile report ---");
                let _ = writeln!(
                    std::io::stderr(),
                    "{:<40} {:>12} {:>7}",
                    "function",
                    "calls",
                    "%"
                );
                let _ = writeln!(std::io::stderr(), "{}", "-".repeat(61));
                let limit = entries.len().min(20);
                for (name, count) in &entries[..limit] {
                    let pct = (*count as f64 / total as f64) * 100.0;
                    let _ = writeln!(
                        std::io::stderr(),
                        "{:<40} {:>12} {:>6.1}%",
                        name,
                        count,
                        pct
                    );
                }
                if entries.len() > limit {
                    let _ = writeln!(
                        std::io::stderr(),
                        "  ... and {} more functions",
                        entries.len() - limit
                    );
                }
                let _ = writeln!(std::io::stderr(), "{}", "-".repeat(61));
                let _ = writeln!(
                    std::io::stderr(),
                    "total calls: {total}, unique functions: {}",
                    entries.len()
                );
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
    let _ = MEMORY_ALLOC_COUNT.fetch_update(Ordering::Relaxed, Ordering::Relaxed, |count| {
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
        unsafe { *(base.add(CHUNK_TOTAL_OFFSET) as *mut usize) = total | flag };
    }
    base
}

// Read the total-size word written by `alloc_chunk`, masking off the
// oversized-flag bit.
unsafe fn chunk_total(base: *const u8) -> usize {
    unsafe { *(base.add(CHUNK_TOTAL_OFFSET) as *const usize) & !CHUNK_OVERSIZED_FLAG }
}

// True iff the chunk was allocated via __vow_arena_alloc's oversized path,
// regardless of its `total` size. This is the predicate
// `arena_try_free_oversized_chunk` consults to decide whether a chunk is
// single-resident and safe to free.
unsafe fn chunk_is_oversized(base: *const u8) -> bool {
    unsafe { *(base.add(CHUNK_TOTAL_OFFSET) as *const usize) & CHUNK_OVERSIZED_FLAG != 0 }
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
// was initialized by `alloc_chunk`. The C ESBMC mirror in
// `vow-runtime/verify/arena.c` performs the identical `*(void**)chunk` access
// under the same invariant, so the model check exercises this exact reasoning.
#[inline]
unsafe fn next_chunk(chunk: *mut u8) -> *mut u8 {
    unsafe { *(chunk as *mut *mut u8) }
}

#[inline]
unsafe fn set_next_chunk(chunk: *mut u8, next: *mut u8) {
    unsafe { *(chunk as *mut *mut u8) = next };
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

#[unsafe(no_mangle)]
pub unsafe extern "C" fn __vow_arena_init_closed(a: *mut VowArena) {
    let arena = unsafe { &mut *a };
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
    let arena = unsafe { &mut *a };
    if !arena.first_chunk.is_null() {
        return;
    }

    let total = normal_chunk_total();
    let base = unsafe { alloc_chunk(total, false) };
    if base.is_null() {
        oom_trap("arena_open");
    }
    let arena = unsafe { &mut *a };
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
    let arena = unsafe { &mut *a };
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
    let arena = unsafe { &mut *a };
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
    let arena = unsafe { &mut *a };
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
    let arena = unsafe { &mut *a };
    let mut prev: *mut u8 = core::ptr::null_mut();
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
            if prev.is_null() {
                arena.first_chunk = next;
            } else {
                unsafe { set_next_chunk(prev, next) };
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
        prev = chunk;
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
    let _guard = ROOT_ARENA_LOCK.lock().unwrap();
    unsafe { ensure_root_arena_locked() };
}

unsafe fn ensure_root_arena_locked() {
    if !ROOT_ARENA_INITIALIZED.load(Ordering::SeqCst) {
        unsafe { __vow_arena_open(&raw mut __vow_root_arena) };
        ROOT_ARENA_INITIALIZED.store(true, Ordering::SeqCst);
    }
}

unsafe fn root_arena_alloc(bytes: usize, align: usize) -> *mut u8 {
    let _guard = ROOT_ARENA_LOCK.lock().unwrap();
    unsafe { ensure_root_arena_locked() };
    unsafe { __vow_arena_alloc(&raw mut __vow_root_arena, bytes, align) }
}

unsafe fn root_arena_alloc_zeroed(bytes: usize, align: usize) -> *mut u8 {
    let ptr = unsafe { root_arena_alloc(bytes, align) };
    unsafe { std::ptr::write_bytes(ptr, 0, bytes) };
    ptr
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

unsafe fn root_arena_grow_backing(
    ptr: *mut u8,
    old_size: usize,
    new_size: usize,
    align: usize,
) -> *mut u8 {
    let _guard = ROOT_ARENA_LOCK.lock().unwrap();
    unsafe { ensure_root_arena_locked() };
    unsafe { arena_grow_backing(&raw mut __vow_root_arena, ptr, old_size, new_size, align) }
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
    let header_ptr = unsafe { __vow_arena_alloc(arena, 24, 8) } as *mut VowVec;
    // Lazy allocation: don't allocate buffer until first push.
    // Use a dangling aligned pointer so from_raw_parts with len=0 is safe.
    unsafe {
        (*header_ptr).ptr = align as *mut u8;
        (*header_ptr).len = 0;
        (*header_ptr).cap = 0;
    }
    sanitize_on_vec_new(header_ptr as usize);
    header_ptr as *mut u8
}

#[unsafe(no_mangle)]
pub extern "C" fn __vow_vec_new(elem_size: usize, align: usize) -> *mut u8 {
    let _guard = ROOT_ARENA_LOCK.lock().unwrap();
    unsafe { ensure_root_arena_locked() };
    unsafe { __vow_vec_new_in_arena(&raw mut __vow_root_arena, elem_size, align) }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn __vow_vec_new_val_in_arena(arena: *mut VowArena) -> *mut u8 {
    unsafe { __vow_vec_new_in_arena(arena, 8, 8) }
}

#[unsafe(no_mangle)]
pub extern "C" fn __vow_vec_new_val() -> *mut u8 {
    let _guard = ROOT_ARENA_LOCK.lock().unwrap();
    unsafe { ensure_root_arena_locked() };
    unsafe { __vow_vec_new_val_in_arena(&raw mut __vow_root_arena) }
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
    v.cap = len;
    vec
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn __vow_vec_pin_to_root_val(source: *const u8) -> *mut u8 {
    if source.is_null() {
        return __vow_vec_new_val();
    }
    sanitize_on_read(source as usize, 0);
    let src = unsafe { &*(source as *const VowVec) };
    let _guard = ROOT_ARENA_LOCK.lock().unwrap();
    unsafe { ensure_root_arena_locked() };
    unsafe {
        __vow_vec_from_raw_parts_copy_val(&raw mut __vow_root_arena, src.ptr as *const i64, src.len)
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
    if required <= v.cap {
        return;
    }
    let mut new_cap = if v.cap == 0 { VEC_INITIAL_CAP } else { v.cap };
    while new_cap < required {
        new_cap = match new_cap.checked_mul(2) {
            Some(c) => c,
            None => oom_trap("Vec::reserve"),
        };
    }
    let old_size = match v.cap.checked_mul(elem_size) {
        Some(s) => s,
        None => oom_trap("Vec::reserve"),
    };
    let new_size = match new_cap.checked_mul(elem_size) {
        Some(s) => s,
        None => oom_trap("Vec::reserve"),
    };
    let new_ptr = unsafe { arena_grow_backing(arena, v.ptr, old_size, new_size, elem_align) };
    v.ptr = new_ptr;
    v.cap = new_cap;
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
    let _guard = ROOT_ARENA_LOCK.lock().unwrap();
    unsafe { ensure_root_arena_locked() };
    unsafe {
        vec_reserve_in_arena_no_null_check(
            &raw mut __vow_root_arena,
            vec,
            additional,
            elem_size,
            elem_align,
        )
    };
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
    let _guard = ROOT_ARENA_LOCK.lock().unwrap();
    unsafe { ensure_root_arena_locked() };
    unsafe { __vow_vec_push_in_arena(&raw mut __vow_root_arena, vec, elem, elem_size, elem_align) };
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
    let _guard = ROOT_ARENA_LOCK.lock().unwrap();
    unsafe { ensure_root_arena_locked() };
    unsafe { __vow_vec_push_val_in_arena(&raw mut __vow_root_arena, vec, value) };
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
    let _guard = ROOT_ARENA_LOCK.lock().unwrap();
    unsafe { ensure_root_arena_locked() };
    unsafe { __vow_string_new_in_arena(&raw mut __vow_root_arena, ptr, len) }
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
    let _guard = ROOT_ARENA_LOCK.lock().unwrap();
    unsafe { ensure_root_arena_locked() };
    unsafe { __vow_string_from_cstr_in_arena(&raw mut __vow_root_arena, ptr) }
}

/// Deep-copy `source` (a `VowString` / `Vec<u8>` descriptor) into `arena`,
/// returning a freshly-allocated descriptor whose backing also lives in
/// `arena`. Used by Phase 4 / S5 return materialization (spec §5.1) to
/// satisfy the `FreshInCaller` representation promise when the source path
/// is a `.rodata` literal or a parameter alias whose backing is not in
/// `target_region`.
///
/// The new descriptor has `cap = len`; growth is up to the caller. The
/// source's `cap` is irrelevant — `VOW_CAP_RODATA` (read-only literal) is
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
    let header = unsafe { __vow_arena_alloc(arena, 24, 8) } as *mut VowVec;
    if source.is_null() {
        unsafe {
            (*header).ptr = std::ptr::dangling_mut::<u8>(); // len=0
            (*header).len = 0;
            (*header).cap = 0;
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
        (*header).cap = len;
    }
    header as *mut u8
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn __vow_string_clone(source: *const u8) -> *mut u8 {
    let _guard = ROOT_ARENA_LOCK.lock().unwrap();
    unsafe { ensure_root_arena_locked() };
    unsafe { __vow_string_clone_into_arena(&raw mut __vow_root_arena, source) }
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
    let _guard = ROOT_ARENA_LOCK.lock().unwrap();
    unsafe { ensure_root_arena_locked() };
    unsafe { __vow_string_clone_into_arena(&raw mut __vow_root_arena, source) }
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
    let header = unsafe { __vow_arena_alloc(arena, 24, 8) } as *mut VowVec;
    if len == 0 || ptr.is_null() {
        unsafe {
            (*header).ptr = std::ptr::dangling_mut::<u8>();
            (*header).len = 0;
            (*header).cap = 0;
        }
        return header as *mut u8;
    }
    let data_ptr = unsafe { __vow_arena_alloc(arena, len, 1) };
    unsafe { std::ptr::copy_nonoverlapping(ptr, data_ptr, len) };
    unsafe {
        (*header).ptr = data_ptr;
        (*header).len = len;
        (*header).cap = len;
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
    pos: i64,
    literal_ptr: *const u8,
    literal_len: i64,
) -> i64 {
    if s.is_null() || literal_ptr.is_null() || pos < 0 || literal_len < 0 {
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

#[unsafe(no_mangle)]
pub unsafe extern "C" fn __vow_string_push_str(dest: *mut u8, src: *const u8) {
    let _guard = ROOT_ARENA_LOCK.lock().unwrap();
    unsafe { ensure_root_arena_locked() };
    unsafe { __vow_string_push_str_in_arena(&raw mut __vow_root_arena, dest, src) };
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
    let _guard = ROOT_ARENA_LOCK.lock().unwrap();
    unsafe { ensure_root_arena_locked() };
    unsafe { __vow_string_from_i64_in_arena(&raw mut __vow_root_arena, v) }
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
pub unsafe extern "C" fn __vow_string_byte_at(s: *const u8, idx: i64) -> i64 {
    sanitize_on_read(s as usize, 0);
    let v = unsafe { &*(s as *const VowVec) };
    if idx < 0 || idx as usize >= v.len {
        return -1;
    }
    let bytes = unsafe { std::slice::from_raw_parts(v.ptr, v.len) };
    bytes[idx as usize] as i64
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn __vow_string_push_byte_in_arena(
    arena: *mut VowArena,
    s: *mut u8,
    byte: i64,
) {
    if arena.is_null() {
        null_arena_trap("String::push_byte");
    }
    // Sanitize once here, then delegate to the no-sanitize inner helper with
    // a type-specific operation name. This keeps both orderings correct:
    // sanitizer runs before any dereference (UAF detected first), and the
    // shadow table records a single generation for the one appended byte.
    sanitize_on_push(s as usize);
    let b = byte as u8;
    unsafe { vec_push_no_sanitize_in_arena(arena, s, &b as *const u8, 1, 1, "String::push_byte") };
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn __vow_string_push_byte(s: *mut u8, byte: i64) {
    let _guard = ROOT_ARENA_LOCK.lock().unwrap();
    unsafe { ensure_root_arena_locked() };
    unsafe { __vow_string_push_byte_in_arena(&raw mut __vow_root_arena, s, byte) };
}

// ---------------------------------------------------------------------------
// String utility builtins
// ---------------------------------------------------------------------------

#[unsafe(no_mangle)]
pub unsafe extern "C" fn __vow_string_substr_in_arena(
    arena: *mut VowArena,
    s: *const u8,
    start: i64,
    len: i64,
) -> *mut u8 {
    if arena.is_null() {
        null_arena_trap("String::substr");
    }
    if s.is_null() {
        return unsafe { __vow_string_new_in_arena(arena, std::ptr::null(), 0) };
    }
    sanitize_on_read(s as usize, 0);
    let v = unsafe { &*(s as *const VowVec) };
    let slen = v.len as i64;
    let clamped_start = start.clamp(0, slen) as usize;
    let clamped_len = len.clamp(0, slen - clamped_start as i64) as usize;
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
pub unsafe extern "C" fn __vow_string_substr(s: *const u8, start: i64, len: i64) -> *mut u8 {
    let _guard = ROOT_ARENA_LOCK.lock().unwrap();
    unsafe { ensure_root_arena_locked() };
    unsafe { __vow_string_substr_in_arena(&raw mut __vow_root_arena, s, start, len) }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn __vow_string_substring_in_arena(
    arena: *mut VowArena,
    s: *const u8,
    start: i64,
    end: i64,
) -> *mut u8 {
    if arena.is_null() {
        null_arena_trap("String::substring");
    }
    if s.is_null() {
        return unsafe { __vow_string_new_in_arena(arena, std::ptr::null(), 0) };
    }
    sanitize_on_read(s as usize, 0);
    let v = unsafe { &*(s as *const VowVec) };
    let slen = v.len as i64;
    let clamped_start = start.clamp(0, slen) as usize;
    let clamped_end = end.clamp(clamped_start as i64, slen) as usize;
    let bytes = unsafe { std::slice::from_raw_parts(v.ptr, v.len) };
    let len = clamped_end - clamped_start;
    unsafe {
        __vow_string_new_in_arena(arena, bytes[clamped_start..].as_ptr() as *const c_char, len)
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn __vow_string_substring(s: *const u8, start: i64, end: i64) -> *mut u8 {
    let _guard = ROOT_ARENA_LOCK.lock().unwrap();
    unsafe { ensure_root_arena_locked() };
    unsafe { __vow_string_substring_in_arena(&raw mut __vow_root_arena, s, start, end) }
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
    let _guard = ROOT_ARENA_LOCK.lock().unwrap();
    unsafe { ensure_root_arena_locked() };
    unsafe { __vow_string_split_in_arena(&raw mut __vow_root_arena, haystack, separator) }
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
    let _guard = ROOT_ARENA_LOCK.lock().unwrap();
    unsafe { ensure_root_arena_locked() };
    unsafe { __vow_string_trim_in_arena(&raw mut __vow_root_arena, s) }
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
    let _guard = ROOT_ARENA_LOCK.lock().unwrap();
    unsafe { ensure_root_arena_locked() };
    unsafe { __vow_string_to_upper_in_arena(&raw mut __vow_root_arena, s) }
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
    let _guard = ROOT_ARENA_LOCK.lock().unwrap();
    unsafe { ensure_root_arena_locked() };
    unsafe { __vow_string_to_lower_in_arena(&raw mut __vow_root_arena, s) }
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
    let _guard = ROOT_ARENA_LOCK.lock().unwrap();
    unsafe { ensure_root_arena_locked() };
    unsafe { __vow_string_replace_in_arena(&raw mut __vow_root_arena, s, from, to) }
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
    let _guard = ROOT_ARENA_LOCK.lock().unwrap();
    unsafe { ensure_root_arena_locked() };
    unsafe { __vow_string_join_in_arena(&raw mut __vow_root_arena, vec_ptr, sep) }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn __vow_string_parse_i64_opt(s: *const u8) -> *mut u8 {
    let ptr = __vow_vec_new(8, 8) as *mut i64;
    if s.is_null() {
        unsafe { *ptr = 0 };
        return ptr as *mut u8;
    }
    sanitize_on_read(s as usize, 0);
    let v = unsafe { &*(s as *const VowVec) };
    let bytes = unsafe { std::slice::from_raw_parts(v.ptr, v.len) };
    match std::str::from_utf8(bytes) {
        Ok(text) => match text.trim().parse::<i64>() {
            Ok(val) => {
                unsafe { *ptr = 1 };
                unsafe { *ptr.add(1) = val };
            }
            Err(_) => {
                unsafe { *ptr = 0 };
            }
        },
        Err(_) => {
            unsafe { *ptr = 0 };
        }
    }
    ptr as *mut u8
}

unsafe fn alloc_option_u8(value: Option<u8>) -> *mut u8 {
    let ptr = __vow_vec_new(8, 8) as *mut i64;
    match value {
        Some(value) => unsafe {
            *ptr = 1;
            *ptr.add(1) = i64::from(value);
        },
        None => unsafe {
            *ptr = 0;
            *ptr.add(1) = 0;
        },
    }
    ptr as *mut u8
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn __vow_string_parse_u8_opt(s: *const u8) -> *mut u8 {
    if s.is_null() {
        return unsafe { alloc_option_u8(None) };
    }
    sanitize_on_read(s as usize, 0);
    let v = unsafe { &*(s as *const VowVec) };
    let bytes = unsafe { std::slice::from_raw_parts(v.ptr, v.len) };
    let value = std::str::from_utf8(bytes)
        .ok()
        .and_then(|text| text.trim().parse::<u8>().ok());
    unsafe { alloc_option_u8(value) }
}

macro_rules! define_signed_to_u8 {
    ($try_name:ident, $wrap_name:ident, $sat_name:ident, $ty:ty) => {
        #[unsafe(no_mangle)]
        pub unsafe extern "C" fn $try_name(value: $ty) -> *mut u8 {
            unsafe { alloc_option_u8(u8::try_from(value).ok()) }
        }

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
    ($try_name:ident, $wrap_name:ident, $sat_name:ident, $ty:ty) => {
        #[unsafe(no_mangle)]
        pub unsafe extern "C" fn $try_name(value: $ty) -> *mut u8 {
            unsafe { alloc_option_u8(u8::try_from(value).ok()) }
        }

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
    __vow_i16_to_u8_wrap,
    __vow_i16_to_u8_sat,
    i16
);
define_signed_to_u8!(
    __vow_i32_to_u8_try,
    __vow_i32_to_u8_wrap,
    __vow_i32_to_u8_sat,
    i32
);
define_signed_to_u8!(
    __vow_i64_to_u8_try,
    __vow_i64_to_u8_wrap,
    __vow_i64_to_u8_sat,
    i64
);
define_signed_to_u8!(
    __vow_i128_to_u8_try,
    __vow_i128_to_u8_wrap,
    __vow_i128_to_u8_sat,
    i128
);
define_unsigned_to_u8!(
    __vow_u16_to_u8_try,
    __vow_u16_to_u8_wrap,
    __vow_u16_to_u8_sat,
    u16
);
define_unsigned_to_u8!(
    __vow_u32_to_u8_try,
    __vow_u32_to_u8_wrap,
    __vow_u32_to_u8_sat,
    u32
);
define_unsigned_to_u8!(
    __vow_u64_to_u8_try,
    __vow_u64_to_u8_wrap,
    __vow_u64_to_u8_sat,
    u64
);
define_unsigned_to_u8!(
    __vow_u128_to_u8_try,
    __vow_u128_to_u8_wrap,
    __vow_u128_to_u8_sat,
    u128
);

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

#[unsafe(no_mangle)]
pub unsafe extern "C" fn __vow_string_parse_u64_opt(s: *const u8) -> *mut u8 {
    let ptr = __vow_vec_new(8, 8) as *mut i64;
    if s.is_null() {
        unsafe { *ptr = 0 };
        return ptr as *mut u8;
    }
    sanitize_on_read(s as usize, 0);
    let v = unsafe { &*(s as *const VowVec) };
    let bytes = unsafe { std::slice::from_raw_parts(v.ptr, v.len) };
    match std::str::from_utf8(bytes) {
        Ok(text) => match text.trim().parse::<u64>() {
            Ok(val) => {
                unsafe { *ptr = 1 };
                unsafe { *ptr.add(1) = val as i64 };
            }
            Err(_) => {
                unsafe { *ptr = 0 };
            }
        },
        Err(_) => {
            unsafe { *ptr = 0 };
        }
    }
    ptr as *mut u8
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn __vow_parse_i64(s: *const u8) -> i64 {
    if s.is_null() {
        return 0;
    }
    sanitize_on_read(s as usize, 0);
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
    sanitize_on_read(vec as usize, 0);
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
pub unsafe extern "C" fn __vow_hex_encode(vec: *const u8) -> *mut u8 {
    if vec.is_null() {
        return __vow_vec_new(1, 1);
    }
    sanitize_on_read(vec as usize, 0);
    let v = unsafe { &*(vec as *const VowVec) };
    let vals = unsafe { std::slice::from_raw_parts(v.ptr as *const i64, v.len) };
    let mut hex = String::new();
    for &val in vals {
        hex.push_str(&format!("{:02x}", (val & 0xff) as u8));
    }
    unsafe { __vow_string_new(hex.as_ptr() as *const c_char, hex.len()) }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn __vow_hex_decode(s: *const u8) -> *mut u8 {
    let result = __vow_vec_new_val();
    if s.is_null() {
        return result;
    }
    sanitize_on_read(s as usize, 0);
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
    sanitize_on_read(path_ptr as usize, 0);
    let v = unsafe { &*(path_ptr as *const VowVec) };
    let bytes = unsafe { std::slice::from_raw_parts(v.ptr, v.len) };
    let path = match std::str::from_utf8(bytes) {
        Ok(s) => s,
        Err(_) => return __vow_vec_new(1, 1),
    };
    match std::fs::read(path) {
        Ok(bytes) => unsafe { __vow_string_new(bytes.as_ptr() as *const c_char, bytes.len()) },
        Err(_) => __vow_vec_new(1, 1),
    }
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

#[unsafe(no_mangle)]
pub extern "C" fn __vow_fs_read_line(handle: i64) -> *mut u8 {
    use std::io::BufRead;

    let mut map_guard = FILE_READ_MAP.lock().unwrap();
    let Some(map) = map_guard.as_mut() else {
        return unsafe { __vow_string_new(std::ptr::null(), 0) };
    };
    let Some(state) = map.get_mut(&handle) else {
        return unsafe { __vow_string_new(std::ptr::null(), 0) };
    };
    state.line_buf.clear();
    // The process-global handle table lock is intentionally held while reading;
    // docs/spec/grammar.md documents the concurrency tradeoff for this API.
    match state.reader.read_until(b'\n', &mut state.line_buf) {
        Ok(0) => {
            state.status = 1;
            unsafe { __vow_string_new(std::ptr::null(), 0) }
        }
        Ok(_) => {
            state.status = 0;
            unsafe {
                __vow_string_new(
                    state.line_buf.as_ptr() as *const c_char,
                    state.line_buf.len(),
                )
            }
        }
        Err(_) => {
            state.status = -1;
            unsafe { __vow_string_new(std::ptr::null(), 0) }
        }
    }
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

#[unsafe(no_mangle)]
pub unsafe extern "C" fn __vow_fs_listdir(path_ptr: *const u8) -> *mut u8 {
    let result_vec = __vow_vec_new_val();
    if path_ptr.is_null() {
        return result_vec;
    }
    sanitize_on_read(path_ptr as usize, 0);
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
    let mut names: Vec<String> = entries
        .flatten()
        .map(|e| e.file_name().to_string_lossy().into_owned())
        .collect();
    names.sort();
    for name_str in &names {
        let str_vec =
            unsafe { __vow_string_new(name_str.as_ptr() as *const c_char, name_str.len()) } as i64;
        unsafe { __vow_vec_push_val(result_vec, str_vec) };
    }
    result_vec
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

#[unsafe(no_mangle)]
pub extern "C" fn __vow_stdin_read() -> *mut u8 {
    use std::io::Read;
    let mut buf = Vec::new();
    let _ = std::io::stdin().read_to_end(&mut buf);
    unsafe { __vow_string_new(buf.as_ptr() as *const c_char, buf.len()) }
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
pub extern "C" fn __vow_args() -> *mut u8 {
    let result_vec = __vow_vec_new(8, 8);
    for arg in std::env::args() {
        let str_vec = unsafe { __vow_string_new(arg.as_ptr() as *const c_char, arg.len()) } as i64;
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
pub extern "C" fn __vow_process_get_stdout() -> *mut u8 {
    LAST_STDOUT.with(|cell| {
        let bytes = cell.borrow();
        unsafe { __vow_string_new(bytes.as_ptr() as *const c_char, bytes.len()) }
    })
}

#[unsafe(no_mangle)]
pub extern "C" fn __vow_process_get_stderr() -> *mut u8 {
    LAST_STDERR.with(|cell| {
        let bytes = cell.borrow();
        unsafe { __vow_string_new(bytes.as_ptr() as *const c_char, bytes.len()) }
    })
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

#[unsafe(no_mangle)]
pub extern "C" fn __vow_process_stdout_for(handle: i64) -> *mut u8 {
    let guard = PROCESS_MAP.lock().unwrap();
    if let Some(Some(ProcessState::Completed { stdout, .. })) =
        guard.as_ref().map(|m| m.get(&handle))
    {
        unsafe { __vow_string_new(stdout.as_ptr() as *const c_char, stdout.len()) }
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
        unsafe { __vow_string_new(stderr.as_ptr() as *const c_char, stderr.len()) }
    } else {
        unsafe { __vow_string_new(std::ptr::null(), 0) }
    }
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
}

const MAP_ENTRY_BYTES: usize = 16;
const MAP_INITIAL_CAP: usize = 8;

#[unsafe(no_mangle)]
pub unsafe extern "C" fn __vow_map_new_in_arena(arena: *mut VowArena) -> *mut u8 {
    if arena.is_null() {
        null_arena_trap("HashMap::new");
    }
    let header_ptr = unsafe { __vow_arena_alloc(arena, 24, 8) } as *mut VowMap;
    let buf_size = MAP_INITIAL_CAP * MAP_ENTRY_BYTES;
    let buf_ptr = unsafe { __vow_arena_alloc(arena, buf_size, 8) };
    unsafe { std::ptr::write_bytes(buf_ptr, 0, buf_size) };
    unsafe {
        (*header_ptr).ptr = buf_ptr;
        (*header_ptr).len = 0;
        (*header_ptr).cap = MAP_INITIAL_CAP;
    }
    header_ptr as *mut u8
}

#[unsafe(no_mangle)]
pub extern "C" fn __vow_map_new() -> *mut u8 {
    let _guard = ROOT_ARENA_LOCK.lock().unwrap();
    unsafe { ensure_root_arena_locked() };
    unsafe { __vow_map_new_in_arena(&raw mut __vow_root_arena) }
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
        let new_ptr = unsafe { arena_grow_backing(arena, m.ptr, old_size, new_size, 8) };
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
    let _guard = ROOT_ARENA_LOCK.lock().unwrap();
    unsafe { ensure_root_arena_locked() };
    unsafe { __vow_map_insert_in_arena(&raw mut __vow_root_arena, map, key, val) };
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
// writes. Lives in the root arena (no explicit free), matching HashMap.
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
}

const BTREEMAP_INITIAL_CAP: usize = 8;
const BTREEMAP_ENTRY_BYTES: usize = 8;

#[unsafe(no_mangle)]
pub extern "C" fn __vow_btreemap_new() -> *mut u8 {
    let header_ptr = unsafe { root_arena_alloc_zeroed(std::mem::size_of::<VowBTreeMap>(), 8) }
        as *mut VowBTreeMap;
    let buf_size = BTREEMAP_INITIAL_CAP * BTREEMAP_ENTRY_BYTES;
    let keys_buf = unsafe { root_arena_alloc_zeroed(buf_size, 8) };
    let vals_buf = unsafe { root_arena_alloc_zeroed(buf_size, 8) };
    unsafe {
        (*header_ptr).keys_ptr = keys_buf;
        (*header_ptr).entries_len = 0;
        (*header_ptr).keys_cap = BTREEMAP_INITIAL_CAP;
        (*header_ptr).vals_ptr = vals_buf;
        (*header_ptr).vals_cap = BTREEMAP_INITIAL_CAP;
    }
    header_ptr as *mut u8
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

// Allocate an Option<i64> using the same layout as `__vow_string_parse_i64_opt`:
// a `*mut i64` pointing at [tag, payload] (tag=1 Some, tag=0 None).
unsafe fn alloc_option_i64(tag: i64, payload: i64) -> *mut u8 {
    let ptr = __vow_vec_new(8, 8) as *mut i64;
    unsafe {
        *ptr = tag;
        *ptr.add(1) = payload;
    }
    ptr as *mut u8
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn __vow_btreemap_insert(map: *mut u8, key: i64, val: i64) -> *mut u8 {
    let m = unsafe { &mut *(map as *mut VowBTreeMap) };
    let keys = unsafe { std::slice::from_raw_parts(m.keys_ptr as *const i64, m.entries_len) };
    match btreemap_search(keys, key) {
        Ok(idx) => {
            let vals =
                unsafe { std::slice::from_raw_parts_mut(m.vals_ptr as *mut i64, m.entries_len) };
            let prev = vals[idx];
            vals[idx] = val;
            unsafe { alloc_option_i64(1, prev) }
        }
        Err(idx) => {
            if m.entries_len == m.keys_cap {
                let old_size = m.keys_cap * BTREEMAP_ENTRY_BYTES;
                let new_cap = m.keys_cap * 2;
                let new_size = new_cap * BTREEMAP_ENTRY_BYTES;
                m.keys_ptr = unsafe { root_arena_grow_backing(m.keys_ptr, old_size, new_size, 8) };
                m.vals_ptr = unsafe { root_arena_grow_backing(m.vals_ptr, old_size, new_size, 8) };
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
            unsafe { alloc_option_i64(0, 0) }
        }
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn __vow_btreemap_get(map: *const u8, key: i64) -> *mut u8 {
    let m = unsafe { &*(map as *const VowBTreeMap) };
    let keys = unsafe { std::slice::from_raw_parts(m.keys_ptr as *const i64, m.entries_len) };
    match btreemap_search(keys, key) {
        Ok(idx) => {
            let vals =
                unsafe { std::slice::from_raw_parts(m.vals_ptr as *const i64, m.entries_len) };
            unsafe { alloc_option_i64(1, vals[idx]) }
        }
        Err(_) => unsafe { alloc_option_i64(0, 0) },
    }
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
        assert_eq!(vec.cap, 0, "empty Vec should have cap=0 (lazy)");
    }

    #[test]
    fn vec_first_push_allocates() {
        let v = __vow_vec_new_val();
        unsafe { __vow_vec_push_val(v, 42) };
        let vec = unsafe { &*(v as *const VowVec) };
        assert_eq!(vec.len, 1);
        assert!(vec.cap >= 1, "cap should be allocated after first push");
        assert_eq!(unsafe { __vow_vec_get_val(v, 0) }, 42);
    }

    #[test]
    fn string_new_empty_lazy() {
        let s = __vow_vec_new(1, 1);
        let vec = unsafe { &*(s as *const VowVec) };
        assert_eq!(vec.len, 0);
        assert_eq!(vec.cap, 0, "empty String should have cap=0 (lazy)");
    }

    #[test]
    fn string_from_empty_lazy() {
        let s = unsafe { __vow_string_new(std::ptr::null(), 0) };
        let vec = unsafe { &*(s as *const VowVec) };
        assert_eq!(vec.len, 0);
        assert_eq!(vec.cap, 0, "String::from(\"\") should have cap=0 (lazy)");
    }

    #[test]
    fn string_from_nonempty_allocates() {
        let data = b"hello";
        let s = unsafe { __vow_string_new(data.as_ptr() as *const c_char, 5) };
        let vec = unsafe { &*(s as *const VowVec) };
        assert_eq!(vec.len, 5);
        assert!(vec.cap >= 5);
    }

    #[test]
    fn vec_multiple_push_after_lazy() {
        let v = __vow_vec_new_val();
        for i in 0..20 {
            unsafe { __vow_vec_push_val(v, i) };
        }
        let vec = unsafe { &*(v as *const VowVec) };
        assert_eq!(vec.len, 20);
        assert!(vec.cap >= 20);
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
        assert_eq!(header.cap, 0);

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
        assert!(header.cap >= 12);

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
        unsafe { __vow_string_push_byte_in_arena(&mut a, hello, b'w' as i64) };

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

        unsafe { __vow_arena_close(&mut a) };
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
        assert!(header_before.cap > 2048, "test setup: must be oversized");

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
                __vow_string_matches_literal_at(s_ptr, 1, literal.as_ptr(), literal.len() as i64)
            },
            1
        );
        assert_eq!(
            unsafe {
                __vow_string_matches_literal_at(s_ptr, 2, literal.as_ptr(), literal.len() as i64)
            },
            0
        );
        assert_eq!(
            unsafe { __vow_string_matches_literal_at(s_ptr, -1, literal.as_ptr(), 3) },
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
        assert_eq!(cv.cap, 5, "clone must not inherit VOW_CAP_RODATA");
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
        assert_eq!(pv.cap, 6, "pinning must return a mutable root copy");
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

        for byte in [b'a', b'b'] {
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
        assert!(cv.cap >= 3);
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
        assert!(cv.cap >= 3);
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
        assert_eq!(cv.cap, 5, "clone must be mutable, not rodata");
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
        assert_eq!(cv.cap, 0);
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
        assert_eq!(unsafe { __vow_map_get(m, 2) }, 20);

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
            assert_eq!(unsafe { __vow_map_get(m, i) }, i * 100);
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
        assert_eq!(unsafe { __vow_map_get(m, 7) }, 70);
        assert_eq!(unsafe { __vow_map_get(m, 3) }, 30);
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
                    b'x' as i64,
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
        if op == "HashMap::insert_in_arena_null" {
            let mut m = VowMap {
                ptr: 8 as *mut u8,
                len: 0,
                cap: 0,
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

    fn assert_runtime_invariant_null_arena(op: &str, expected_op_in_json: &str) {
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
            stderr.contains(r#""reason":"null arena""#),
            "stderr missing null arena reason:\n{stderr}"
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
    fn explicit_arena_map_new_null_arena_traps() {
        assert_runtime_invariant_null_arena("HashMap::new_in_arena_null", "HashMap::new");
    }

    #[test]
    fn explicit_arena_map_insert_null_arena_traps() {
        assert_runtime_invariant_null_arena("HashMap::insert_in_arena_null", "HashMap::insert");
    }

    #[test]
    fn explicit_arena_map_remove_null_arena_traps() {
        assert_runtime_invariant_null_arena("HashMap::remove_in_arena_null", "HashMap::remove");
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
        assert!(vec.cap >= 1, "lazy-allocated, cap should be populated");
    }
}
