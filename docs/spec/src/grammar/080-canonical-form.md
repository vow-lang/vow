
#### System

| Function         | Signature                                  | Effects    |
|------------------|--------------------------------------------|------------|
| `num_cpus`       | `fn() -> i64`                              | `[io]`     |
| `proc_sample`    | `fn() -> String`                           | `[io]`     |
| `gzip_write_file` | `fn(path: String, data: String) -> i64`   | `[io]`     |
| `memory_root_arena_bytes` | `fn() -> u64`                    | `[io]`     |
| `memory_peak_bytes` | `fn() -> u64`                           | `[io]`     |
| `memory_alloc_count_since_start` | `fn() -> u64`              | `[io]`     |

`num_cpus()` returns the number of available logical CPUs (from `std::thread::available_parallelism`), or `1` if the query fails. Used to size worker pools (e.g. the default `--verify-jobs` value).

`proc_sample()` samples the calling process and its child processes and returns one `group|rss_kb|cpu_pct` line per group, separated by newlines (all integers); it returns the empty string where the sample cannot be read (non-Linux hosts). `gzip_write_file(path, data)` writes `data` gzip-compressed to `path` and returns `0` on success or `-1` when either argument is not a valid path/string or the file cannot be written. Both exist for the `--perfetto` tracer.

`memory_root_arena_bytes()` returns the current bytes retained by root-region
arena chunks. It is a gauge, not a monotone counter: adding a root chunk raises
it, while reclaiming an abandoned single-resident oversized root chunk during
backing growth lowers it. `memory_peak_bytes()` returns the peak live bytes
retained by all open arena chunks since process start.
`memory_alloc_count_since_start()` returns the number of successful Vow arena
allocation requests since process start. Peak bytes and allocation count are
monotone non-decreasing and saturate at `u64::MAX` rather than wrapping. These
queries do not allocate; they are effectful because they observe runtime process
state.

#### Encoding

| Function         | Signature                                  | Effects    |
|------------------|--------------------------------------------|------------|
| `hex_encode`     | `fn(data: Vec<u8>) -> String`              | `[]`       |
| `hex_decode`     | `fn(s: String) -> Vec<u8>`                 | `[]`       |

#### Hashing

| Function         | Signature                                  | Effects    |
|------------------|--------------------------------------------|------------|
| `hash_u64`       | `fn(x: u64) -> u64`                        | `[]`       |
| `hash_str`       | `fn(s: String) -> u64`                     | `[]`       |

Both are pure and deterministic across runs, platforms and compilers. `hash_u64` is the SplitMix64 output function applied to state `x` (`z = x + 0x9E3779B97F4A7C15; z = (z ^ (z >> 30)) * 0xBF58476D1CE4E5B9; z = (z ^ (z >> 27)) * 0x94D049BB133111EB; z ^ (z >> 31)`, all arithmetic wrapping), so `hash_u64(0) == 0xE220A8397B1DCDAF`; it is a bijection on `u64`, which is also the function `HashMap` uses to place its keys. `hash_str` is 64-bit FNV-1a over the string's bytes (offset basis `0xCBF29CE484222325`, prime `0x100000001B3`), so `hash_str("") == 0xCBF29CE484222325` and `hash_str("a") == 0xAF63DC4C8601EC8C`. Combine hashes in Vow, e.g. `hash_u64(a ^ hash_u64(b))`.

A hash is **not** an identity: distinct strings can collide. To key a `HashMap` by `String`, use `hash_str(s)` as the `u64` key and keep a side table (for example a `Vec<String>` indexed by interned id) that confirms the original string on every hit, resolving collisions explicitly. The native verifier (`--backend native`) encodes `hash_u64` exactly, as the closed 64-bit term above, so a property of `hash_u64(x)` is proven or refuted for the runtime's value. The ESBMC backend, and `hash_str` under both backends (a fold over every byte of a string of unbounded length, with no closed form), have no model of the result: a function that calls one is reported as skipped rather than proven.

#### Input

| Function         | Signature                                  | Effects    |
|------------------|--------------------------------------------|------------|
| `args`           | `fn() -> Vec<String>`                      | `[read]`   |
| `stdin_read`     | `fn() -> String`                           | `[read]`   |
| `stdin_read_line`| `fn() -> String`                           | `[read]`   |
| `stdin_ready`    | `fn() -> bool`                             | `[read]`   |

#### Process Management

| Function              | Signature                                        | Effects |
|-----------------------|--------------------------------------------------|---------|
| `process_exit`        | `fn(code: i64) -> !`                             | `[io]`  |
| `process_run`         | `fn(cmd: String, args: Vec<String>) -> i64`      | `[io]`  |
| `process_get_stdout`  | `fn() -> String`                                 | `[io]`  |
| `process_get_stderr`  | `fn() -> String`                                 | `[io]`  |
| `process_start`       | `fn(cmd: String, args: Vec<String>) -> i64`      | `[io]`  |
| `process_start_capped` | `fn(cmd: String, args: Vec<String>, mem_kb: i64) -> i64` | `[io]`  |
| `process_wait`        | `fn(pid: i64) -> i64`                            | `[io]`  |
| `process_wait_timeout`| `fn(pid: i64, timeout_ms: i64) -> i64`           | `[io]`  |
| `process_poll_wait`   | `fn(pid: i64, timeout_ms: i64) -> i64`           | `[io]`  |
| `process_kill`        | `fn(pid: i64) -> i64`                             | `[io]`  |
| `process_stdout_for`  | `fn(pid: i64) -> String`                         | `[io]`  |
| `process_stderr_for`  | `fn(pid: i64) -> String`                         | `[io]`  |
| `process_start_piped` | `fn(cmd: String, args: Vec<String>) -> i64`      | `[io]`  |
| `process_write_stdin` | `fn(pid: i64, data: String) -> i64`              | `[io]`  |
| `process_close_stdin` | `fn(pid: i64) -> i64`                            | `[io]`  |
| `process_read_line`   | `fn(pid: i64, timeout_ms: i64) -> String`        | `[io]`  |
| `process_read_status` | `fn(pid: i64) -> i64`                            | `[io]`  |

**`args` semantics:** `args()` returns all process arguments including the program name at index 0 (matching C `argv` and Rust `std::env::args()` conventions). For `./my_program foo bar`, `args()` returns `["./my_program", "foo", "bar"]`. Use `args[1]` onward for user-supplied arguments. The Vec is empty only if the OS provides no arguments (unusual). Returns an empty String element if an argument is empty (`""`). Non-UTF-8 arguments are included as-is (byte content preserved).

**`fs_read` semantics:** `fs_read(path)` opens the file at `path`, reads its entire contents, and returns a String. Returns `""` (empty String) on any error (file not found, permission denied, I/O error, non-UTF-8 path). Does not block on regular files. An empty result is ambiguous on its own, because an empty file also reads as `""`; call `fs_read_status()` immediately after `fs_read` to tell the two apart.

**`fs_read_status` semantics:** `fs_read_status()` reports the outcome of the calling thread's most recent `fs_read`: `0` ok (including an empty file), `1` not found, `2` permission denied, `3` any other I/O error (a directory, a read error), `4` invalid path (null, non-UTF-8, or containing a NUL byte), and `-1` if the thread has not called `fs_read` yet. The status is thread-local, so concurrent threads do not race; read it on the same thread, immediately after the `fs_read` it describes, because the next `fs_read` overwrites it. Other `fs_*` builtins do not touch it.

**`getenv` semantics:** `getenv(name)` returns the value of environment variable `name`, or `""` when the variable is unset, its name is empty or contains `=` or a NUL byte, or its name or value is not valid UTF-8. An unset variable and a variable set to the empty string are not distinguished.

**`path_lookup` semantics:** `path_lookup(name)` resolves an executable the way `command -v` does, minus shell builtins and aliases. A `name` containing `/` is tested directly and returned unchanged when it is a regular file with an execute bit; otherwise each entry of `$PATH` is searched in order (an empty entry means the current directory) and the first `<entry>/<name>` that is an executable regular file is returned. Returns `""` when nothing matches, `name` is empty, or `$PATH` is unset and `name` contains no `/`.

**`mktemp_dir` semantics:** `mktemp_dir(prefix)` atomically creates a new directory `<tmp>/<prefix>.XXXXXXXXXX` with mode `0700` and returns its path, where `<tmp>` is `$TMPDIR` (trailing `/` stripped) or `/tmp` when `$TMPDIR` is unset or empty. Returns `""` when `prefix` is empty or contains `/` or a NUL byte, or when creation fails. The caller owns the directory and must remove it (`fs_remove` its files, then `fs_remove_dir`; or `fs_remove_dir_all`).

**Streaming file input:** `fs_open(path)` opens a file for incremental reading and returns a positive handle, or `-1` on path/open error. `fs_read_line(handle)` reads one line from the current cursor and returns it as a String, including the trailing newline when present. It returns `""` at EOF, for an invalid handle, or after a read error. A blank line is returned as `"\n"`, so newline-delimited callers can distinguish a real blank line from EOF by content. After `fs_read_line(handle)` returns `""`, call `fs_status(handle)` to distinguish EOF from error: `0` means the handle is open with no EOF/error state, `1` means EOF, and `-1` means invalid handle or read error. `fs_status(handle)` reports the result of the most recent `fs_read_line(handle)` call on that open handle; read it immediately after a `""` return because later reads may update it. `fs_close(handle)` releases the handle and returns `0` on success or `-1` for an invalid/already-closed handle. Long-running programs must close handles they no longer need. All streaming handle operations use the `[read]` effect, including `fs_close`, because closing a read handle releases read-stream state and does not mutate filesystem contents. The current runtime stores streaming handles in one process-global table, and `fs_read_line` holds that table lock while it reads the next line. This keeps the API simple for single-stream file processing, but it is not intended for latency-sensitive concurrent reads from multiple slow handles.

**`fs_remove_dir` vs `fs_remove_dir_all`:** `fs_remove_dir(path)` is `rmdir`: it removes `path` only when it is an empty directory and fails (non-zero) when the directory is non-empty, is not a directory, or does not exist, so it can never delete files by accident. `fs_remove_dir_all(path)` recursively removes `path` and everything beneath it and is irreversible; it fails (non-zero) when `path` does not exist, and a symlink at the top level is unlinked rather than followed. Prefer `fs_remove_dir` unless a whole tree must go.

**Filesystem return values:** `fs_write`, `fs_mkdir`, `fs_remove`, `fs_remove_dir`, `fs_remove_dir_all`, and `fs_rename` return `i64`: 0 on success, non-zero on failure. `fs_open`, `fs_status`, and `fs_close` use the streaming status codes above. `fs_exists`, `fs_is_dir`, and `fs_is_symlink` are predicates: they return 1 for true, 0 for false. Errors (null pointer, invalid UTF-8) also return 0, so callers cannot distinguish "false" from "error". `fs_is_symlink` uses `lstat`-equivalent semantics: a symlink reports 1 even when its target is a regular file or directory.

**`string_starts_with` / `string_ends_with` / `string_matches_literal_at` return values:** Return `i64`: 1 if true, 0 if false.

**`string_matches_literal_at` literal operand:** The third argument must be written as a string literal at the call site. The compiler lowers that literal to static bytes plus an explicit byte length, so no temporary `String` allocation is created and embedded NUL bytes are preserved. Passing a variable or computed `String` as the third argument is a type-check error (`StaticLiteralRequired`). Use `string_starts_with`, `string_ends_with`, or `String` methods when the needle must be dynamic.

**`process_run` vs `process_start`:** `process_run(cmd, args)` runs a subprocess synchronously and returns its exit code. After it returns, `process_get_stdout()` and `process_get_stderr()` retrieve the captured output of the most recent `process_run` call. `process_start(cmd, args)` launches a subprocess asynchronously and returns a process ID. Use `process_wait(pid)` to wait for completion and get the exit code, and `process_stdout_for(pid)` / `process_stderr_for(pid)` to retrieve output.

**`process_wait_timeout`:** `process_wait_timeout(pid, timeout_ms)` polls a process started with `process_start` until it exits or the timeout (in milliseconds) elapses. Returns the exit code on completion, `-1` on error, or `-2` on timeout. After a timeout, the process is still running; use `process_kill(pid)` to terminate it.

**`process_start_capped`:** `process_start_capped(cmd, args, mem_kb)` starts a child like `process_start` (same ID, same `process_wait*`, `process_poll_wait`, `process_stdout_for` and `process_kill` behaviour) with two differences. The child leads its own process group, and `process_kill` and a `process_wait_timeout` expiry SIGKILL the whole group, so grandchildren die with it; after a normal exit the group is swept the same way. On Linux the child also runs with an address-space limit of `mem_kb` KiB (`mem_kb <= 0` means no limit; other platforms ignore it). A child killed by a signal reports `-1000 - signal` instead of `-1`, so a signal death can be told from a spawn error. Returns `-1` if the command cannot be started.

**`process_poll_wait`:** `process_poll_wait(pid, timeout_ms)` waits up to `timeout_ms` milliseconds for a process started with `process_start` to exit, without killing it on timeout. Returns the exit code on completion, `-1` on an unknown process ID, or a large negative sentinel if the process is still running after the timeout (left alive, not killed). Unlike `process_wait_timeout`, callers can re-poll and impose their own watchdog deadline instead of the process being killed automatically.

**Interactive child processes:** `process_start_piped(cmd, args)` starts a child like `process_start` but with stdin, stdout and stderr all piped, and returns a process ID (`-1` if the command cannot be started). The ID works with `process_wait`, `process_wait_timeout`, `process_poll_wait`, `process_kill`, `process_stdout_for` and `process_stderr_for`. `process_write_stdin(pid, data)` writes all of `data` to the child's stdin and flushes it; it returns `0`, or `-1` for an unknown or non-piped ID, a closed stdin, or a child that stopped reading (a broken pipe never terminates the program). `process_close_stdin(pid)` closes the child's stdin so it sees end-of-file; it returns `0` (also when already closed) or `-1` for an unknown or non-piped ID. `process_read_line(pid, timeout_ms)` returns the child's next stdout line including its trailing `\n` (a final line without a newline is returned as-is), or `""` when no line is available. `timeout_ms` bounds the whole call: a negative value blocks until a line or end-of-file, `0` only polls. Bytes read before a timeout are kept for the next call. `process_read_status(pid)` reports the outcome of the last `process_read_line` on that ID: `0` a line was read, `1` end-of-file (stdout closed and fully consumed), `2` timed out with the child still running, `-1` unknown or non-piped ID. `process_wait` closes the child's stdin first so a child waiting for end-of-file can finish; `process_wait_timeout` and `process_poll_wait` leave stdin open, so call `process_close_stdin` first for a child that only exits at end-of-file. Once `process_wait`, `process_wait_timeout` or `process_poll_wait` has finished the child, or `process_kill` has run, the ID is no longer piped: `process_read_line` returns `""` with status `-1` and `process_write_stdin` and `process_close_stdin` return `-1`. Whatever stdout the program never read is then available only as a whole through `process_stdout_for(pid)` (`process_kill` discards it), so read the child's last lines before polling it to completion. While the program only calls `process_read_line`, a child that writes faster than the program reads blocks on a full pipe instead of growing the program's memory, so a program that writes a large payload to the child's stdin without reading its output can deadlock against a chatty child exactly as with a raw pipe. `process_wait`, `process_wait_timeout` and `process_poll_wait` instead collect all remaining stdout in memory, and stderr is always collected in full until the ID is waited on or killed. Every piped ID must eventually be passed to `process_wait` or `process_kill`.

**`process_kill`:** `process_kill(pid)` sends a kill signal to a running process and waits for it to exit. Returns 0 on success, -1 on error. No-op (returns 0) if the process has already completed.

**`stdin_read` vs `stdin_read_line`:** `stdin_read()` reads the entire stdin stream into a single String (unbounded memory). `stdin_read_line()` reads one line at a time, including the trailing newline. Returns `""` (empty string) at EOF. The returned String is runtime scratch storage valid until the next `stdin_read_line()` call. Process each line before reading the next one for bounded memory; use `pin_to_root(line)` before the next read when a line must be stored, returned, passed to a function that may store it, mutated, or otherwise retained. The direct scratch line is read-only. The scratch buffer keeps the largest line capacity seen so far, so memory is bounded by maximum line length rather than total input, but one very large line can retain that capacity for the process lifetime.

```vow
let lines: Vec<String> = Vec::new();
let mut line: String = stdin_read_line();
while str_len(line) > 0 {
    // Without pin_to_root, lines.push(line) would store the scratch alias, not a copy.
    lines.push(pin_to_root(line));
    line = stdin_read_line();
}
```

```vow
let mut line: String = stdin_read_line();
while str_len(line) > 0 {
    // process line (has trailing \n)
    line = stdin_read_line();
}
```

**`stdin_ready`:** `stdin_ready()` returns `true` if `stdin_read_line()` would return immediately without blocking, `false` otherwise. Uses a non-blocking poll with zero timeout. Use this in computation loops that must remain responsive to external input:

```vow
while !stdin_ready() && depth < max_depth {
    // continue searching
    depth = depth + 1;
}
if stdin_ready() {
    let cmd: String = stdin_read_line();
    // handle command
}
```

## Canonical Form

The canonical printer normalizes source: `parse → print → parse` is idempotent. Effects are sorted alphabetically, indentation uses 4 spaces, trailing expressions omit semicolons.
