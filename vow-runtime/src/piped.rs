//! Child processes with a piped stdin and an incrementally readable stdout.
//!
//! A piped child lives in two tables: its `std::process::Child` is in
//! `PROCESS_MAP` (so `process_wait`, `process_kill`, ... keep working on the
//! same handle) and its pipes are here. A reader thread forwards stdout in
//! bounded chunks over a bounded channel, so a chatty child blocks on a full
//! pipe exactly as it would with a raw pipe instead of growing runtime memory;
//! a second thread drains stderr. Lifecycle hooks in `lib.rs` pump the channel
//! before waiting so an unread child can always finish, then call [`release`]
//! to join the threads and hand the captured output back.

use super::{
    NEXT_PROCESS_HANDLE, PROCESS_MAP, ProcessState, VowArena, VowVec, alloc_bytes_string,
    decode_process_command, process_map_init, sanitize_on_read, spawn_drain, with_root_arena,
};
use std::collections::HashMap;
use std::io::{Read, Write};
use std::process::{ChildStdin, Command, Stdio};
use std::sync::atomic::Ordering;
use std::sync::mpsc::{Receiver, RecvTimeoutError, TryRecvError, sync_channel};
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

const CHUNK_BYTES: usize = 64 * 1024;
const CHANNEL_CHUNKS: usize = 16;

const READ_LINE: i64 = 0;
const READ_EOF: i64 = 1;
const READ_TIMEOUT: i64 = 2;
const READ_UNKNOWN: i64 = -1;

/// Stdout forwarder and stderr collector of one piped child.
type Threads = (JoinHandle<()>, JoinHandle<Vec<u8>>);

struct StdoutReader {
    rx: Receiver<Vec<u8>>,
    pending: Vec<u8>,
    head: usize,
    eof: bool,
    status: i64,
}

impl StdoutReader {
    fn buffered(&self) -> &[u8] {
        &self.pending[self.head..]
    }

    /// Appends `chunk`, first dropping the consumed prefix once it is at
    /// least half the buffer, so consuming a line never shifts the rest.
    fn push(&mut self, chunk: &[u8]) {
        if self.head > 0 && self.head * 2 >= self.pending.len() {
            self.pending.drain(..self.head);
            self.head = 0;
        }
        self.pending.extend_from_slice(chunk);
    }

    fn take(&mut self, len: usize) -> Vec<u8> {
        let taken = self.pending[self.head..self.head + len].to_vec();
        self.head += len;
        if self.head == self.pending.len() {
            self.pending.clear();
            self.head = 0;
        }
        taken
    }

    fn take_all(&mut self) -> Vec<u8> {
        let mut all = std::mem::take(&mut self.pending);
        all.drain(..self.head);
        self.head = 0;
        all
    }
}

pub(crate) struct PipedChild {
    stdin: Mutex<Option<ChildStdin>>,
    stdout: Mutex<StdoutReader>,
    threads: Mutex<Option<Threads>>,
}

static PIPED_MAP: Mutex<Option<HashMap<i64, Arc<PipedChild>>>> = Mutex::new(None);

/// The piped state of `handle`, if it was started with `process_start_piped`
/// and has not been released.
pub(crate) fn lookup(handle: i64) -> Option<Arc<PipedChild>> {
    PIPED_MAP.lock().unwrap().as_ref()?.get(&handle).cloned()
}

/// Makes writes to `stdin` fail with `EPIPE` instead of raising SIGPIPE. Darwin
/// directs that signal at the process, so blocking it on the writing thread (see
/// `write_shielded`) does not stop another thread from taking it; the per-fd
/// flag does, and leaves the process-wide disposition alone.
#[cfg(target_vendor = "apple")]
fn suppress_sigpipe(stdin: &ChildStdin) {
    use std::os::fd::AsRawFd;
    // Not exported by the libc crate; value from <sys/fcntl.h>.
    const F_SETNOSIGPIPE: libc::c_int = 73;
    unsafe { libc::fcntl(stdin.as_raw_fd(), F_SETNOSIGPIPE, 1) };
}

#[cfg(not(target_vendor = "apple"))]
fn suppress_sigpipe(_stdin: &ChildStdin) {}

/// Writes `data` with SIGPIPE blocked on this thread, so a child that stopped
/// reading yields `EPIPE` instead of killing the program. The process-wide
/// disposition is left alone: ignoring it globally would also keep a program
/// alive after its own stdout consumer (`| head`) has gone away.
fn write_shielded(stdin: &mut ChildStdin, data: &[u8]) -> bool {
    let mut sigpipe: libc::sigset_t = unsafe { std::mem::zeroed() };
    let mut previous: libc::sigset_t = unsafe { std::mem::zeroed() };
    unsafe {
        libc::sigemptyset(&mut sigpipe);
        libc::sigaddset(&mut sigpipe, libc::SIGPIPE);
        libc::pthread_sigmask(libc::SIG_BLOCK, &sigpipe, &mut previous);
    }
    let written = stdin.write_all(data).and_then(|()| stdin.flush()).is_ok();
    unsafe {
        if libc::sigismember(&previous, libc::SIGPIPE) != 1 {
            let mut pending: libc::sigset_t = std::mem::zeroed();
            libc::sigpending(&mut pending);
            if libc::sigismember(&pending, libc::SIGPIPE) == 1 {
                let mut signal = 0;
                libc::sigwait(&sigpipe, &mut signal);
            }
        }
        libc::pthread_sigmask(libc::SIG_SETMASK, &previous, std::ptr::null_mut());
    }
    written
}

fn spawn_stdout_reader(
    mut pipe: impl Read + Send + 'static,
) -> (Receiver<Vec<u8>>, JoinHandle<()>) {
    let (tx, rx) = sync_channel::<Vec<u8>>(CHANNEL_CHUNKS);
    let thread = std::thread::spawn(move || {
        let mut buf = vec![0u8; CHUNK_BYTES];
        loop {
            match pipe.read(&mut buf) {
                Ok(0) | Err(_) => break,
                Ok(n) => {
                    if tx.send(buf[..n].to_vec()).is_err() {
                        break;
                    }
                }
            }
        }
    });
    (rx, thread)
}

/// Spawns `cmd` with all three standard streams piped and registers it under a
/// fresh process handle; `-1` if the command cannot be started.
pub(crate) fn start(cmd: &str, args: &[String]) -> i64 {
    let mut child = match Command::new(cmd)
        .args(args)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
    {
        Ok(child) => child,
        Err(_) => return -1,
    };
    let (Some(stdin), Some(stdout), Some(stderr)) =
        (child.stdin.take(), child.stdout.take(), child.stderr.take())
    else {
        let _ = child.kill();
        let _ = child.wait();
        return -1;
    };
    suppress_sigpipe(&stdin);
    let (rx, stdout_thread) = spawn_stdout_reader(stdout);
    let stderr_thread = spawn_drain(Some(stderr));
    let piped = Arc::new(PipedChild {
        stdin: Mutex::new(Some(stdin)),
        stdout: Mutex::new(StdoutReader {
            rx,
            pending: Vec::new(),
            head: 0,
            eof: false,
            status: READ_LINE,
        }),
        threads: Mutex::new(Some((stdout_thread, stderr_thread))),
    });
    let handle = NEXT_PROCESS_HANDLE.fetch_add(1, Ordering::Relaxed);
    PIPED_MAP
        .lock()
        .unwrap()
        .get_or_insert_with(HashMap::new)
        .insert(handle, piped);
    let mut guard = PROCESS_MAP.lock().unwrap();
    process_map_init(&mut guard).insert(handle, ProcessState::Running(child));
    handle
}

impl PipedChild {
    pub(crate) fn close_stdin(&self) {
        self.stdin.lock().unwrap().take();
    }

    fn write_stdin(&self, data: &[u8]) -> i64 {
        let mut guard = self.stdin.lock().unwrap();
        let Some(stdin) = guard.as_mut() else {
            return -1;
        };
        if write_shielded(stdin, data) { 0 } else { -1 }
    }

    /// Moves every chunk the reader thread has already produced into the
    /// pending buffer without blocking.
    pub(crate) fn pump(&self) {
        let mut r = self.stdout.lock().unwrap();
        loop {
            match r.rx.try_recv() {
                Ok(chunk) => r.push(&chunk),
                Err(TryRecvError::Empty) => return,
                Err(TryRecvError::Disconnected) => {
                    r.eof = true;
                    return;
                }
            }
        }
    }

    /// Blocks until the child's stdout closes, buffering whatever it writes.
    pub(crate) fn drain_to_eof(&self) {
        let mut r = self.stdout.lock().unwrap();
        while let Ok(chunk) = r.rx.recv() {
            r.push(&chunk);
        }
        r.eof = true;
    }

    /// Next stdout line including its `\n`; a final unterminated line is
    /// returned as-is. `timeout_ms < 0` blocks, otherwise it bounds the whole
    /// call. Empty on timeout or EOF; `read_status` says which.
    fn read_line(&self, timeout_ms: i64) -> Vec<u8> {
        let deadline =
            (timeout_ms >= 0).then(|| Instant::now() + Duration::from_millis(timeout_ms as u64));
        let mut r = self.stdout.lock().unwrap();
        let mut scanned = 0;
        loop {
            if let Some(off) = r.buffered()[scanned..].iter().position(|&b| b == b'\n') {
                r.status = READ_LINE;
                return r.take(scanned + off + 1);
            }
            scanned = r.buffered().len();
            if r.eof {
                if scanned > 0 {
                    r.status = READ_LINE;
                    return r.take(scanned);
                }
                r.status = READ_EOF;
                return Vec::new();
            }
            let received = match deadline {
                None => r.rx.recv().map_err(|_| RecvTimeoutError::Disconnected),
                Some(d) => {
                    r.rx.recv_timeout(d.saturating_duration_since(Instant::now()))
                }
            };
            match received {
                Ok(chunk) => r.push(&chunk),
                Err(RecvTimeoutError::Timeout) => {
                    r.status = READ_TIMEOUT;
                    return Vec::new();
                }
                Err(RecvTimeoutError::Disconnected) => r.eof = true,
            }
        }
    }
}

/// Retires a finished (or killed) piped child: closes stdin, collects every
/// byte of stdout it never read plus all of stderr, and joins the threads.
pub(crate) fn release(handle: i64) -> Option<(Vec<u8>, Vec<u8>)> {
    let piped = PIPED_MAP.lock().unwrap().as_mut()?.remove(&handle)?;
    piped.close_stdin();
    piped.drain_to_eof();
    let stdout = piped.stdout.lock().unwrap().take_all();
    let stderr = match piped.threads.lock().unwrap().take() {
        Some((out, err)) => {
            let _ = out.join();
            err.join().unwrap_or_default()
        }
        None => Vec::new(),
    };
    Some((stdout, stderr))
}

/// Starts a child with piped stdin/stdout/stderr. Returns a process handle
/// usable with every `process_*` builtin, or -1 on error.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn __vow_process_start_piped(cmd_ptr: i64, args_ptr: i64) -> i64 {
    match decode_process_command(cmd_ptr, args_ptr) {
        Some((cmd, args)) => start(&cmd, &args),
        None => -1,
    }
}

/// Writes all of `data` to the child's stdin and flushes. 0 on success, -1 for
/// an unknown or non-piped handle, a closed stdin, or a child that stopped
/// reading.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn __vow_process_write_stdin(handle: i64, data_ptr: i64) -> i64 {
    sanitize_on_read(data_ptr as usize, 0);
    let data = unsafe { &*(data_ptr as *const VowVec) };
    let bytes = unsafe { std::slice::from_raw_parts(data.ptr, data.len) };
    match lookup(handle) {
        Some(piped) => piped.write_stdin(bytes),
        None => -1,
    }
}

/// Closes the child's stdin so it sees EOF. 0 on success (idempotent), -1 for
/// an unknown or non-piped handle.
#[unsafe(no_mangle)]
pub extern "C" fn __vow_process_close_stdin(handle: i64) -> i64 {
    match lookup(handle) {
        Some(piped) => {
            piped.close_stdin();
            0
        }
        None => -1,
    }
}

fn read_process_line(handle: i64, timeout_ms: i64) -> Vec<u8> {
    lookup(handle).map_or_else(Vec::new, |piped| piped.read_line(timeout_ms))
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn __vow_process_read_line_in_arena(
    arena: *mut VowArena,
    handle: i64,
    timeout_ms: i64,
) -> *mut u8 {
    let line = read_process_line(handle, timeout_ms);
    unsafe { alloc_bytes_string(arena, &line) }
}

#[unsafe(no_mangle)]
pub extern "C" fn __vow_process_read_line(handle: i64, timeout_ms: i64) -> *mut u8 {
    let line = read_process_line(handle, timeout_ms);
    unsafe { with_root_arena(|arena| alloc_bytes_string(arena, &line)) }
}

/// Outcome of the last `process_read_line` on `handle`: 0 line read, 1 EOF,
/// 2 timed out, -1 unknown or non-piped handle.
#[unsafe(no_mangle)]
pub extern "C" fn __vow_process_read_status(handle: i64) -> i64 {
    match lookup(handle) {
        Some(piped) => piped.stdout.lock().unwrap().status,
        None => READ_UNKNOWN,
    }
}

#[cfg(test)]
mod tests {
    use super::super::{
        __vow_process_kill, __vow_process_poll_wait, __vow_process_stderr_for,
        __vow_process_stdout_for, __vow_process_wait, __vow_process_wait_timeout, __vow_string_new,
        __vow_vec_new, __vow_vec_push, VOW_PROC_STILL_RUNNING,
    };
    use super::*;
    use std::ffi::c_char;

    fn vow_string(s: &str) -> i64 {
        unsafe { __vow_string_new(s.as_ptr() as *const c_char, s.len()) as i64 }
    }

    fn vow_text(p: *mut u8) -> String {
        let v = unsafe { &*(p as *const VowVec) };
        let bytes = unsafe { std::slice::from_raw_parts(v.ptr, v.len) };
        String::from_utf8(bytes.to_vec()).unwrap()
    }

    fn start_sh(script: &str) -> i64 {
        let args = __vow_vec_new(8, 8);
        for a in ["-c", script] {
            let s = vow_string(a);
            unsafe { __vow_vec_push(args, &s as *const i64 as *const u8, 8, 8) };
        }
        unsafe { __vow_process_start_piped(vow_string("sh"), args as i64) }
    }

    fn write(handle: i64, data: &str) -> i64 {
        unsafe { __vow_process_write_stdin(handle, vow_string(data)) }
    }

    fn read(handle: i64, timeout_ms: i64) -> (String, i64) {
        let line = vow_text(__vow_process_read_line(handle, timeout_ms));
        (line, __vow_process_read_status(handle))
    }

    const ECHO_LOOP: &str = r#"while IFS= read -r l; do echo "got:$l"; done"#;

    #[test]
    fn echoes_line_by_line_before_stdin_closes() {
        let h = start_sh(ECHO_LOOP);
        assert!(h > 0);
        assert_eq!(write(h, "a\n"), 0);
        assert_eq!(read(h, 5000), ("got:a\n".to_string(), READ_LINE));
        assert_eq!(write(h, "b\n"), 0);
        assert_eq!(read(h, 5000), ("got:b\n".to_string(), READ_LINE));
        assert_eq!(__vow_process_close_stdin(h), 0);
        assert_eq!(__vow_process_close_stdin(h), 0);
        assert_eq!(read(h, 5000), (String::new(), READ_EOF));
        assert_eq!(__vow_process_wait(h), 0);
        assert!(lookup(h).is_none());
    }

    #[test]
    fn timeout_keeps_partial_line_for_the_next_call() {
        let h = start_sh("printf par; sleep 0.4; printf 'tial\\n'");
        assert_eq!(read(h, 50), (String::new(), READ_TIMEOUT));
        assert_eq!(read(h, 5000), ("partial\n".to_string(), READ_LINE));
        assert_eq!(__vow_process_wait(h), 0);
    }

    #[test]
    fn zero_timeout_polls_and_silent_child_stays_alive() {
        let h = start_sh("cat >/dev/null");
        assert_eq!(read(h, 0), (String::new(), READ_TIMEOUT));
        assert_eq!(read(h, 30), (String::new(), READ_TIMEOUT));
        assert_eq!(__vow_process_poll_wait(h, 10), VOW_PROC_STILL_RUNNING);
        assert_eq!(__vow_process_kill(h), 0);
    }

    #[test]
    fn unterminated_tail_is_returned_then_eof() {
        let h = start_sh("printf x");
        assert_eq!(read(h, 5000), ("x".to_string(), READ_LINE));
        assert_eq!(read(h, 5000), (String::new(), READ_EOF));
        assert_eq!(__vow_process_wait(h), 0);
    }

    #[test]
    fn several_lines_in_one_chunk_are_split() {
        let h = start_sh("printf 'one\\ntwo\\n\\nthree'");
        assert_eq!(read(h, 5000).0, "one\n");
        assert_eq!(read(h, 5000).0, "two\n");
        assert_eq!(read(h, 5000).0, "\n");
        assert_eq!(read(h, 5000).0, "three");
        assert_eq!(read(h, 5000), (String::new(), READ_EOF));
        assert_eq!(__vow_process_wait(h), 0);
    }

    #[test]
    fn many_lines_across_chunks_arrive_in_order() {
        let h = start_sh("seq 1 60000");
        for expected in 1..=60000 {
            assert_eq!(read(h, 20_000), (format!("{expected}\n"), READ_LINE));
        }
        assert_eq!(read(h, 20_000), (String::new(), READ_EOF));
        assert_eq!(__vow_process_wait(h), 0);
    }

    #[test]
    fn error_paths_return_minus_one() {
        assert_eq!(write(-5, "x"), -1);
        assert_eq!(__vow_process_close_stdin(-5), -1);
        assert_eq!(read(-5, 10), (String::new(), -1));
        assert_eq!(__vow_process_read_status(-5), -1);
        let bad = unsafe { __vow_process_start_piped(vow_string("/no/such/binary"), vow_vec()) };
        assert_eq!(bad, -1);

        let h = start_sh("cat >/dev/null");
        assert_eq!(__vow_process_close_stdin(h), 0);
        assert_eq!(write(h, "late\n"), -1, "write after close");
        assert_eq!(__vow_process_wait(h), 0);
        assert_eq!(write(h, "x"), -1, "write after wait");
    }

    fn vow_vec() -> i64 {
        __vow_vec_new(8, 8) as i64
    }

    #[test]
    fn write_to_an_exited_child_fails_instead_of_raising_sigpipe() {
        let h = start_sh("exit 0");
        assert_eq!(read(h, 5000), (String::new(), READ_EOF));
        assert!(
            lookup(h).is_some(),
            "the handle must still be live so the write reaches the pipe"
        );
        let big = "x".repeat(1 << 20);
        let inherited = unsafe { libc::signal(libc::SIGPIPE, libc::SIG_DFL) };
        let rc = write(h, &big);
        unsafe { libc::signal(libc::SIGPIPE, inherited) };
        assert_eq!(rc, -1);
        assert_eq!(__vow_process_wait(h), 0);
    }

    #[test]
    fn kill_unblocks_a_reader_and_releases_the_handle() {
        let h = start_sh("cat >/dev/null");
        let reader = std::thread::spawn(move || read(h, -1));
        std::thread::sleep(Duration::from_millis(100));
        assert_eq!(__vow_process_kill(h), 0);
        let (line, status) = reader.join().unwrap();
        assert_eq!(line, "");
        assert!(status == READ_EOF || status == READ_UNKNOWN, "{status}");
        assert!(lookup(h).is_none());
    }

    #[test]
    fn wait_keeps_unread_stdout_and_stderr_and_closes_stdin() {
        let h = start_sh("echo out1; echo out2; echo err 1>&2; cat >/dev/null");
        assert_eq!(read(h, 5000).0, "out1\n");
        assert_eq!(__vow_process_wait(h), 0, "wait closes stdin so cat ends");
        assert_eq!(vow_text(__vow_process_stdout_for(h)), "out2\n");
        assert_eq!(vow_text(__vow_process_stderr_for(h)), "err\n");
        assert!(lookup(h).is_none());
    }

    const BIG: usize = 2_000_000;

    #[test]
    fn wait_drains_a_chatty_child_nobody_read() {
        let h = start_sh("head -c 2000000 /dev/zero");
        assert_eq!(__vow_process_wait(h), 0);
        assert_eq!(vow_text(__vow_process_stdout_for(h)).len(), BIG);
        assert!(lookup(h).is_none());
    }

    #[test]
    fn poll_wait_drains_a_chatty_child_nobody_read() {
        let h = start_sh("head -c 2000000 /dev/zero");
        let mut code = VOW_PROC_STILL_RUNNING;
        for _ in 0..1000 {
            code = __vow_process_poll_wait(h, 20);
            if code != VOW_PROC_STILL_RUNNING {
                break;
            }
        }
        assert_eq!(code, 0);
        assert_eq!(vow_text(__vow_process_stdout_for(h)).len(), BIG);
        assert!(lookup(h).is_none());
    }

    #[test]
    fn wait_timeout_drains_and_reports_the_output() {
        let h = start_sh("head -c 2000000 /dev/zero; echo err 1>&2");
        assert_eq!(__vow_process_wait_timeout(h, 20_000), 0);
        assert_eq!(vow_text(__vow_process_stdout_for(h)).len(), BIG);
        assert_eq!(vow_text(__vow_process_stderr_for(h)), "err\n");
        assert!(lookup(h).is_none());
    }

    #[test]
    fn wait_timeout_kills_a_silent_child_and_releases() {
        let h = start_sh("cat >/dev/null");
        assert_eq!(__vow_process_wait_timeout(h, 50), -2);
        assert!(lookup(h).is_none());
    }

    #[test]
    fn read_line_allocates_in_the_requested_arena() {
        use super::super::__vow_arena_open;
        let h = start_sh("echo hi");
        let mut a = VowArena {
            first_chunk: core::ptr::null_mut(),
            current_chunk: core::ptr::null_mut(),
            cursor: 0,
            chunk_end: 0,
            last_alloc_start: core::ptr::null_mut(),
            last_alloc_size: 0,
            retained_bytes: 0,
        };
        let ap: *mut VowArena = &mut a;
        unsafe { __vow_arena_open(ap) };
        let before = unsafe { (*ap).cursor };
        let s = unsafe { __vow_process_read_line_in_arena(ap, h, 5000) };
        assert_eq!(vow_text(s), "hi\n");
        assert!(unsafe { (*ap).cursor } > before);
        assert_eq!(__vow_process_wait(h), 0);
    }
}
