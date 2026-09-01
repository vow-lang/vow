use std::io::{BufReader, Read, Write};
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use vow_verify::{ArithOverflowSite, CalleePrecondition, Counterexample, SolverConfig};

use crate::frontend::DependencyManifest;

// Counterexamples are serialized after source-name mapping. Version the record
// header whenever that mapping or the disk schema changes, so an old failure
// cannot re-enter the compiler with stale or ambiguously decoded payloads.
const VERIFY_CACHE_FAILURE_HEADER: &str = "FAILED v3";

// Bump this whenever generated object files are no longer ABI-compatible with
// existing cached artifacts. Static string literals, full-width block arena
// stack slots, UTF-8-preserving string lexing, contextual narrow-integer
// reductions, narrow Option parameter payloads, width-preserving unary
// negation, aggregate match-payload tags (which also change FieldGet indices
// for the same source), the fail-closed 128-bit aggregate-field guard, and
// element-width narrowing at Vec index reads all make pre-cutover objects
// unsafe to reuse.
const COMPILE_CACHE_ABI_VERSION: &str =
    "static-string-arena-slot-utf8-lexer-narrow-unary-match-aggregate-wide-guard-index-v7";

pub struct CompileCache {
    dir: PathBuf,
}

impl CompileCache {
    pub fn new() -> Option<Self> {
        let dir = if let Ok(d) = std::env::var("VOW_CACHE_DIR") {
            PathBuf::from(d)
        } else if let Ok(home) = std::env::var("HOME") {
            PathBuf::from(home).join(".cache").join("vow")
        } else {
            return None;
        };
        std::fs::create_dir_all(&dir).ok()?;
        Some(Self { dir })
    }

    pub fn cache_key(deps: &DependencyManifest, mode: &str, trace: &str) -> Option<String> {
        Self::cache_key_with_abi_seed(deps, mode, trace, COMPILE_CACHE_ABI_VERSION)
    }

    // Fail closed on any per-dep canonicalize / open / read error: returning None
    // skips both lookup and store, so partial dep sets can never collide with a
    // previously-cached object built from the full set.
    fn cache_key_with_abi_seed(
        deps: &DependencyManifest,
        mode: &str,
        trace: &str,
        abi_seed: &str,
    ) -> Option<String> {
        // FNV-1a for stable on-disk keys across toolchain upgrades (DefaultHasher is unspecified across releases).
        let mut entries: Vec<(String, u64)> = Vec::with_capacity(deps.paths().len());
        for p in deps.paths() {
            let canon = p.canonicalize().ok()?;
            let f = std::fs::File::open(&canon).ok()?;
            let content_hash = fnv1a_hash_reader(BufReader::new(f)).ok()?;
            entries.push((canon.to_string_lossy().to_string(), content_hash));
        }
        entries.sort_by(|a, b| a.0.cmp(&b.0));
        // Length-prefix paths so an embedded `:` can't create path/hash boundary ambiguity.
        let mut combined = String::new();
        combined.push_str("__abi=");
        combined.push_str(abi_seed);
        combined.push('\n');
        for (path, content_hash) in &entries {
            combined.push_str(&format!(
                "__dep={}:{path}:{content_hash:016x}\n",
                path.len()
            ));
        }
        combined.push_str("__mode=");
        combined.push_str(mode);
        combined.push('\n');
        combined.push_str("__trace=");
        combined.push_str(trace);
        combined.push('\n');
        let hash = fnv1a_hash(combined.as_bytes());
        Some(format!("{hash:016x}"))
    }

    pub fn lookup(&self, key: &str) -> Option<PathBuf> {
        let cached = self.dir.join(format!("{key}.o"));
        if cached.exists() { Some(cached) } else { None }
    }

    pub fn store(&self, key: &str, obj: &Path) -> PathBuf {
        let cached = self.dir.join(format!("{key}.o"));
        let _ = std::fs::copy(obj, &cached);
        cached
    }
}

// ---------------------------------------------------------------------------
// Verification cache
// ---------------------------------------------------------------------------

fn fnv1a_hash(data: &[u8]) -> u64 {
    let mut hash: u64 = 0xcbf29ce484222325;
    for &byte in data {
        hash ^= byte as u64;
        hash = hash.wrapping_mul(0x100000001b3);
    }
    hash
}

// Streaming FNV-1a so dependency-content hashing stays bounded-memory regardless of file size. Caller must pass a blocking reader (e.g. `BufReader<File>`); a non-blocking `Read` returning 0 to mean "try again" would be misinterpreted as EOF.
fn fnv1a_hash_reader<R: Read>(mut r: R) -> std::io::Result<u64> {
    let mut hash: u64 = 0xcbf29ce484222325;
    let mut buf = [0u8; 8192];
    loop {
        let n = r.read(&mut buf)?;
        if n == 0 {
            break;
        }
        for &byte in &buf[..n] {
            hash ^= byte as u64;
            hash = hash.wrapping_mul(0x100000001b3);
        }
    }
    Ok(hash)
}

// Security: cached PROVEN results from disk are never trusted, so the cached
// type only carries failure data. A forged on-disk entry must not be able to
// bypass ESBMC, so successful verifications are never cached. The type is
// module-private: callers speak in `Counterexample` through `lookup_failure` /
// `store_failure`, which makes "only failures are cached" unrepresentable
// outside this module rather than a caller-side discipline.
#[derive(Debug)]
struct CachedFailure {
    vow_id: Option<u32>,
    callee_precondition: Option<CalleePrecondition>,
    /// The `arith:` site is persisted explicitly rather than re-derived from
    /// tool output on replay, keeping cache semantics independent of ESBMC text.
    arith_overflow: Option<ArithOverflowSite>,
    description: String,
    values: Vec<(String, String)>,
    block_visits: Vec<u32>,
    raw_output: String,
}

// Keep the disk schema independent of verifier crate traits. JSON makes every
// string boundary unambiguous: ESBMC aggregate values contain commas and raw
// traces contain newlines, both of which corrupted the old delimiter format.
#[derive(Deserialize, Serialize)]
struct CachedFailureRecord {
    vow_id: Option<u32>,
    callee_precondition_func_id: Option<u32>,
    callee_precondition_vow_id: Option<u32>,
    arith_cause: Option<String>,
    arith_func: Option<u32>,
    arith_start: Option<u32>,
    arith_len: Option<u32>,
    description: String,
    values: Vec<(String, String)>,
    block_visits: Vec<u32>,
    raw_output: String,
}

impl From<&CachedFailure> for CachedFailureRecord {
    fn from(failure: &CachedFailure) -> Self {
        Self {
            vow_id: failure.vow_id,
            callee_precondition_func_id: failure
                .callee_precondition
                .map(|precondition| precondition.func_id),
            callee_precondition_vow_id: failure
                .callee_precondition
                .map(|precondition| precondition.vow_id),
            arith_cause: failure
                .arith_overflow
                .map(|site| site.abort.label().to_string()),
            arith_func: failure.arith_overflow.map(|site| site.func_id),
            arith_start: failure.arith_overflow.map(|site| site.span_start),
            arith_len: failure.arith_overflow.map(|site| site.span_len),
            description: failure.description.clone(),
            values: failure.values.clone(),
            block_visits: failure.block_visits.clone(),
            raw_output: failure.raw_output.clone(),
        }
    }
}

impl From<CachedFailureRecord> for CachedFailure {
    fn from(record: CachedFailureRecord) -> Self {
        let callee_precondition = record
            .callee_precondition_func_id
            .zip(record.callee_precondition_vow_id)
            .map(|(func_id, vow_id)| CalleePrecondition { func_id, vow_id });
        let arith_overflow = record
            .arith_cause
            .as_deref()
            .and_then(vow_verify::ArithAbort::from_label)
            .zip(record.arith_func)
            .zip(record.arith_start.zip(record.arith_len))
            .map(
                |((abort, func_id), (span_start, span_len))| ArithOverflowSite {
                    abort,
                    func_id,
                    span_start,
                    span_len,
                },
            );
        Self {
            vow_id: record.vow_id,
            callee_precondition,
            arith_overflow,
            description: record.description,
            values: record.values,
            block_visits: record.block_visits,
            raw_output: record.raw_output,
        }
    }
}

impl CachedFailure {
    fn to_counterexample(&self) -> Counterexample {
        Counterexample {
            description: self.description.clone(),
            vow_id: self.vow_id,
            callee_precondition: self.callee_precondition,
            arith_overflow: self.arith_overflow,
            values: self.values.clone(),
            block_visits: self.block_visits.clone(),
            raw_output: self.raw_output.clone(),
        }
    }

    fn from_counterexample(ce: &Counterexample) -> Self {
        CachedFailure {
            vow_id: ce.vow_id,
            callee_precondition: ce.callee_precondition,
            arith_overflow: ce.arith_overflow,
            description: ce.description.clone(),
            values: ce.values.clone(),
            block_visits: ce.block_visits.clone(),
            raw_output: ce.raw_output.clone(),
        }
    }
}

pub struct VerifyCache {
    dir: PathBuf,
}

impl VerifyCache {
    pub fn new() -> Option<Self> {
        let dir = if let Ok(d) = std::env::var("VOW_CACHE_DIR") {
            PathBuf::from(d).join("verify")
        } else if let Ok(home) = std::env::var("HOME") {
            PathBuf::from(home)
                .join(".cache")
                .join("vow")
                .join("verify")
        } else {
            return None;
        };
        std::fs::create_dir_all(&dir).ok()?;
        Some(Self { dir })
    }

    /// Look up a cached counterexample for `c_source` under `config`.
    ///
    /// The key is derived from the resolved solver/encoding/memlimit — the same
    /// dimensions that can change a verdict — so a run that reproduces those
    /// reproduces the hit. Only FAILED entries are ever returned: a forged
    /// PROVEN file on disk is discarded here, never bypassing ESBMC.
    pub fn lookup_failure(
        &self,
        c_source: &str,
        max_k_step: u32,
        config: &SolverConfig,
    ) -> Option<Counterexample> {
        let key = Self::config_key(c_source, max_k_step, config);
        self.lookup(&key).map(|cached| cached.to_counterexample())
    }

    /// Cache a counterexample for `c_source` under `config`.
    ///
    /// Taking a `Counterexample` (never a verdict) is what enforces "never cache
    /// PROVEN" as a type property rather than a caller obligation: there is no
    /// way to hand a successful result to this method.
    pub fn store_failure(
        &self,
        c_source: &str,
        max_k_step: u32,
        config: &SolverConfig,
        ce: &Counterexample,
    ) {
        let key = Self::config_key(c_source, max_k_step, config);
        self.store(&key, &CachedFailure::from_counterexample(ce));
    }

    // Project a `SolverConfig` onto the key components. `timeout_secs` is
    // deliberately omitted: a found counterexample is independent of how long
    // the solver was allowed to run, and the auto-mode fallback stores under a
    // config whose timeout differs from the lookup config's (Some(30) vs None) —
    // including it would turn every fallback store into a permanent miss.
    // solver/encoding go through `*_str()` (which resolves Auto), so a pre-
    // fallback `func_config` and the post-fallback `resolved_config` hash alike.
    fn config_key(c_source: &str, max_k_step: u32, config: &SolverConfig) -> String {
        Self::cache_key(
            c_source,
            max_k_step,
            config.solver_str(),
            config.encoding_str(),
            config.memlimit_mb,
        )
    }

    fn cache_key(
        c_source: &str,
        max_k_step: u32,
        solver: &str,
        encoding: &str,
        memlimit_mb: Option<u32>,
    ) -> String {
        let memlimit = match memlimit_mb {
            Some(mb) => format!("{mb}m"),
            None => "none".to_string(),
        };
        let combined = format!(
            "{c_source}\n__max_k_step={max_k_step}\n__solver={solver}\n__encoding={encoding}\n__memlimit={memlimit}"
        );
        let hash = fnv1a_hash(combined.as_bytes());
        format!("{hash:016x}")
    }

    fn lookup(&self, key: &str) -> Option<CachedFailure> {
        let path = self.dir.join(format!("{key}.vr"));
        let content = std::fs::read_to_string(path).ok()?;
        parse_cached_result(&content)
    }

    fn store(&self, key: &str, result: &CachedFailure) {
        let path = self.dir.join(format!("{key}.vr"));
        let Some(content) = serialize_cached_result(result) else {
            return;
        };
        let mut f = match std::fs::File::create(&path) {
            Ok(f) => f,
            Err(_) => return,
        };
        let _ = f.write_all(content.as_bytes());
    }
}

fn serialize_cached_result(result: &CachedFailure) -> Option<String> {
    let payload = serde_json::to_string(&CachedFailureRecord::from(result)).ok()?;
    Some(format!("{VERIFY_CACHE_FAILURE_HEADER}\n{payload}\n"))
}

fn parse_cached_result(content: &str) -> Option<CachedFailure> {
    let mut lines = content.lines();
    let status = lines.next()?;
    // Security: only honor cached FAILED entries. A forged "PROVEN" file must
    // never bypass ESBMC, so PROVEN content is discarded here.
    if status != VERIFY_CACHE_FAILURE_HEADER {
        return None;
    }
    let payload = lines.next()?;
    if lines.next().is_some() {
        return None;
    }
    serde_json::from_str::<CachedFailureRecord>(payload)
        .ok()
        .map(Into::into)
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;
    use vow_verify::{Encoding, Solver, SolverConfig};

    fn cache_in(dir: &TempDir) -> VerifyCache {
        VerifyCache {
            dir: dir.path().to_path_buf(),
        }
    }

    fn sample_ce() -> Counterexample {
        Counterexample {
            arith_overflow: Some(ArithOverflowSite {
                abort: vow_verify::ArithAbort::Mul,
                func_id: 4,
                span_start: 92,
                span_len: 6,
            }),
            description: "x overflows".to_string(),
            vow_id: Some(5),
            callee_precondition: Some(CalleePrecondition {
                func_id: 2,
                vow_id: 1,
            }),
            values: vec![
                ("x".to_string(), "9223372036854775807".to_string()),
                ("y".to_string(), "1".to_string()),
            ],
            block_visits: vec![0, 2, 5],
            raw_output: "ESBMC raw trace".to_string(),
        }
    }

    // #585: the `arith:` site must survive the cache round-trip. It used to be
    // re-derived from the cached `raw_output`, which silently does not work:
    // `raw=` is serialized as the whole ESBMC output but parsed one line at a
    // time, so a replayed counterexample keeps only its first line and the
    // `Violated property:` block is gone. The symptom was a fixture that
    // verified under `--no-cache` and reported VerifyFailed on a cache hit.
    #[test]
    fn arith_site_survives_the_cache_round_trip() {
        let dir = TempDir::new().expect("temp dir");
        let cache = cache_in(&dir);
        let ce = sample_ce();
        let cfg = cfg(Solver::Boolector, Encoding::Bv, None, None);
        cache.store_failure("int main(){}", 10, &cfg, &ce);

        let back = cache
            .lookup_failure("int main(){}", 10, &cfg)
            .expect("cached failure");
        assert_eq!(
            back.arith_overflow, ce.arith_overflow,
            "the arith site must round-trip exactly"
        );
    }

    // A counterexample with no arith site round-trips as None, not as a
    // half-parsed site pointing at offset 0.
    #[test]
    fn absent_arith_site_round_trips_as_none() {
        let dir = TempDir::new().expect("temp dir");
        let cache = cache_in(&dir);
        let mut ce = sample_ce();
        ce.arith_overflow = None;
        let cfg = cfg(Solver::Boolector, Encoding::Bv, None, None);
        cache.store_failure("int main(){}", 10, &cfg, &ce);

        let back = cache
            .lookup_failure("int main(){}", 10, &cfg)
            .expect("cached failure");
        assert_eq!(back.arith_overflow, None);
    }

    fn cfg(
        solver: Solver,
        encoding: Encoding,
        timeout_secs: Option<u32>,
        memlimit_mb: Option<u32>,
    ) -> SolverConfig {
        SolverConfig {
            solver,
            encoding,
            timeout_secs,
            memlimit_mb,
        }
    }

    const C_SRC: &str = "int f(void) { return 0; }";

    #[test]
    fn verify_cache_failure_roundtrips_all_counterexample_fields() {
        let dir = TempDir::new().unwrap();
        let vc = cache_in(&dir);
        let ce = sample_ce();
        let config = cfg(Solver::Boolector, Encoding::Bv, None, Some(4096));

        vc.store_failure(C_SRC, 10, &config, &ce);
        let got = vc
            .lookup_failure(C_SRC, 10, &config)
            .expect("a stored failure must be found under the same key");

        assert_eq!(got.description, ce.description);
        assert_eq!(got.vow_id, ce.vow_id);
        assert_eq!(got.callee_precondition, ce.callee_precondition);
        assert_eq!(got.values, ce.values);
        assert_eq!(got.block_visits, ce.block_visits);
        assert_eq!(got.raw_output, ce.raw_output);
    }

    #[test]
    fn verify_cache_failure_preserves_aggregate_values_and_multiline_output() {
        let dir = TempDir::new().unwrap();
        let vc = cache_in(&dir);
        let mut ce = sample_ce();
        ce.values
            .push(("v".to_string(), "{ .len=2, .data={ 1, 2, 0 } }".to_string()));
        ce.raw_output = "Counterexample:\nv = { .len=2, .data={ 1, 2, 0 } }\nFAILED".to_string();
        let config = cfg(Solver::Boolector, Encoding::Bv, None, Some(4096));

        vc.store_failure(C_SRC, 10, &config, &ce);
        let got = vc
            .lookup_failure(C_SRC, 10, &config)
            .expect("a stored aggregate counterexample must remain readable");

        assert_eq!(got.values, ce.values);
        assert_eq!(got.raw_output, ce.raw_output);
    }

    // The verify driver looks up under the pre-fallback `func_config` (encoding
    // may be Auto) but stores under the post-fallback `resolved_config`
    // (encoding Bv). Because the key derives from `encoding_str()`, which
    // resolves Auto -> Bv, the lookup must still hit. Pins that the two-config
    // call pattern in verify_one_function is a guaranteed hit, not a silent miss.
    #[test]
    fn verify_cache_lookup_hits_across_auto_and_bv_encoding() {
        let dir = TempDir::new().unwrap();
        let vc = cache_in(&dir);
        let ce = sample_ce();
        let stored = cfg(Solver::Boolector, Encoding::Bv, None, Some(4096));
        let looked_up = cfg(Solver::Boolector, Encoding::Auto, None, Some(4096));

        vc.store_failure(C_SRC, 10, &stored, &ce);
        assert!(
            vc.lookup_failure(C_SRC, 10, &looked_up).is_some(),
            "Auto encoding must resolve to Bv and hit the stored entry"
        );
    }

    // Auto-mode fallback stores under a config whose timeout differs from the
    // lookup config's (Some(30) vs None). timeout_secs must stay out of the key,
    // or every fallback store would be a permanent miss.
    #[test]
    fn verify_cache_key_excludes_timeout_secs() {
        let dir = TempDir::new().unwrap();
        let vc = cache_in(&dir);
        let ce = sample_ce();
        let stored = cfg(Solver::Boolector, Encoding::Bv, Some(30), Some(4096));
        let looked_up = cfg(Solver::Boolector, Encoding::Bv, None, Some(4096));

        vc.store_failure(C_SRC, 10, &stored, &ce);
        assert!(
            vc.lookup_failure(C_SRC, 10, &looked_up).is_some(),
            "differing timeout_secs must not change the key"
        );
    }

    #[test]
    fn verify_cache_lookup_misses_on_distinct_key_components() {
        let dir = TempDir::new().unwrap();
        let vc = cache_in(&dir);
        let base = cfg(Solver::Boolector, Encoding::Bv, None, Some(4096));
        vc.store_failure(C_SRC, 10, &base, &sample_ce());

        // Different unwind bound -> miss.
        assert!(vc.lookup_failure(C_SRC, 20, &base).is_none());
        // Different C source -> miss.
        assert!(
            vc.lookup_failure("int g(void) { return 1; }", 10, &base)
                .is_none()
        );
        // Different memory limit -> miss.
        assert!(
            vc.lookup_failure(
                C_SRC,
                10,
                &cfg(Solver::Boolector, Encoding::Bv, None, Some(1024))
            )
            .is_none()
        );
        // Never-stored key -> miss.
        let empty_dir = TempDir::new().unwrap();
        assert!(
            cache_in(&empty_dir)
                .lookup_failure(C_SRC, 10, &base)
                .is_none()
        );
    }

    #[test]
    fn verify_cache_proven_disk_entry_is_discarded() {
        // Security: a forged "PROVEN" cache file must never be honored.
        assert!(parse_cached_result("PROVEN\n").is_none());
        assert!(parse_cached_result("PROVEN").is_none());
    }

    #[test]
    fn verify_cache_unknown_status_is_discarded() {
        assert!(parse_cached_result("BOGUS\n").is_none());
        assert!(parse_cached_result("").is_none());
    }

    #[test]
    fn verify_cache_legacy_failure_is_discarded() {
        assert!(parse_cached_result("FAILED\nvow_id=1\n").is_none());
        assert!(parse_cached_result("FAILED v2\nvow_id=1\n").is_none());
    }

    #[test]
    fn verify_cache_roundtrip_failed() {
        let result = CachedFailure {
            vow_id: Some(3),
            callee_precondition: Some(vow_verify::CalleePrecondition {
                func_id: 7,
                vow_id: 3,
            }),
            arith_overflow: Some(ArithOverflowSite {
                abort: vow_verify::ArithAbort::DivOverflow,
                func_id: 2,
                span_start: 7,
                span_len: 3,
            }),
            description: "test failure".to_string(),
            values: vec![("x".to_string(), "42".to_string())],
            block_visits: vec![0, 1, 3],
            raw_output: "raw".to_string(),
        };
        let serialized = serialize_cached_result(&result).unwrap();
        let parsed = parse_cached_result(&serialized).unwrap();
        assert_eq!(parsed.vow_id, Some(3));
        assert_eq!(
            parsed.callee_precondition,
            Some(vow_verify::CalleePrecondition {
                func_id: 7,
                vow_id: 3,
            })
        );
        assert_eq!(parsed.description, "test failure");
        assert_eq!(parsed.values.len(), 1);
        assert_eq!(parsed.block_visits, vec![0, 1, 3]);
    }

    #[test]
    fn fnv1a_deterministic() {
        let h1 = fnv1a_hash(b"hello world");
        let h2 = fnv1a_hash(b"hello world");
        assert_eq!(h1, h2);
        let h3 = fnv1a_hash(b"different");
        assert_ne!(h1, h3);
    }

    #[test]
    fn cache_key_includes_max_k_step() {
        let k1 = VerifyCache::cache_key("int f() { return 0; }", 10, "boolector", "bv", Some(4096));
        let k2 = VerifyCache::cache_key("int f() { return 0; }", 20, "boolector", "bv", Some(4096));
        assert_ne!(k1, k2);
    }

    #[test]
    fn cache_key_includes_solver_encoding() {
        let k1 = VerifyCache::cache_key("int f() { return 0; }", 10, "boolector", "bv", Some(4096));
        let k2 = VerifyCache::cache_key("int f() { return 0; }", 10, "z3", "ir", Some(4096));
        assert_ne!(k1, k2);
    }

    #[test]
    fn cache_key_includes_esbmc_memlimit() {
        let k1 = VerifyCache::cache_key("int f() { return 0; }", 10, "boolector", "bv", Some(1024));
        let k2 = VerifyCache::cache_key("int f() { return 0; }", 10, "boolector", "bv", Some(4096));
        assert_ne!(k1, k2);
    }

    #[test]
    fn cache_key_distinguishes_uncapped_esbmc_runs() {
        let capped =
            VerifyCache::cache_key("int f() { return 0; }", 10, "boolector", "bv", Some(4096));
        let uncapped = VerifyCache::cache_key("int f() { return 0; }", 10, "boolector", "bv", None);
        assert_ne!(capped, uncapped);
    }

    #[test]
    fn compile_cache_key_ignores_dependency_order() {
        let dir = TempDir::new().unwrap();
        let a = dir.path().join("a.vow");
        let b = dir.path().join("b.vow");
        std::fs::write(&a, "module A").unwrap();
        std::fs::write(&b, "module B").unwrap();

        let deps_ab = DependencyManifest::from_paths(vec![a.clone(), b.clone()]);
        let deps_ba = DependencyManifest::from_paths(vec![b, a]);

        let k1 = CompileCache::cache_key(&deps_ab, "Release", "Off").unwrap();
        let k2 = CompileCache::cache_key(&deps_ba, "Release", "Off").unwrap();

        assert_eq!(k1, k2);
    }

    #[test]
    fn compile_cache_key_changes_when_dependency_set_changes() {
        let dir = TempDir::new().unwrap();
        let a = dir.path().join("a.vow");
        let b = dir.path().join("b.vow");
        std::fs::write(&a, "module A").unwrap();
        std::fs::write(&b, "module B").unwrap();

        let deps_a = DependencyManifest::from_paths(vec![a.clone()]);
        let deps_ab = DependencyManifest::from_paths(vec![a, b]);

        let k1 = CompileCache::cache_key(&deps_a, "Release", "Off").unwrap();
        let k2 = CompileCache::cache_key(&deps_ab, "Release", "Off").unwrap();

        assert_ne!(k1, k2);
    }

    #[test]
    fn compile_cache_key_includes_codegen_abi_seed() {
        let dir = TempDir::new().unwrap();
        let a = dir.path().join("a.vow");
        std::fs::write(&a, "module A").unwrap();

        let deps = DependencyManifest::from_paths(vec![a]);

        let old_key =
            CompileCache::cache_key_with_abi_seed(&deps, "Release", "Off", "old-abi").unwrap();
        let new_key =
            CompileCache::cache_key_with_abi_seed(&deps, "Release", "Off", "new-abi").unwrap();

        assert_ne!(old_key, new_key);
    }

    #[test]
    fn compile_cache_key_invalidates_pre_aggregate_pattern_objects() {
        let dir = TempDir::new().unwrap();
        let a = dir.path().join("a.vow");
        std::fs::write(&a, "module A").unwrap();

        let deps = DependencyManifest::from_paths(vec![a]);
        let legacy_key = CompileCache::cache_key_with_abi_seed(
            &deps,
            "Release",
            "Off",
            "static-string-arena-slot-utf8-lexer-v1",
        )
        .unwrap();
        let current_key = CompileCache::cache_key(&deps, "Release", "Off").unwrap();

        assert_ne!(legacy_key, current_key);
    }

    #[test]
    fn compile_cache_key_invalidates_pre_wide_aggregate_guard_objects() {
        let dir = TempDir::new().unwrap();
        let a = dir.path().join("a.vow");
        std::fs::write(&a, "module A").unwrap();

        let deps = DependencyManifest::from_paths(vec![a]);
        let legacy_key = CompileCache::cache_key_with_abi_seed(
            &deps,
            "Release",
            "Off",
            "static-string-arena-slot-utf8-lexer-narrow-unary-match-aggregate-v5",
        )
        .unwrap();
        let current_key = CompileCache::cache_key(&deps, "Release", "Off").unwrap();

        assert_ne!(legacy_key, current_key);
    }

    #[test]
    fn compile_cache_key_invalidates_pre_narrow_vec_index_objects() {
        let dir = TempDir::new().unwrap();
        let a = dir.path().join("a.vow");
        std::fs::write(&a, "module A").unwrap();

        let deps = DependencyManifest::from_paths(vec![a]);
        let legacy_key = CompileCache::cache_key_with_abi_seed(
            &deps,
            "Release",
            "Off",
            "static-string-arena-slot-utf8-lexer-narrow-unary-match-aggregate-wide-guard-v6",
        )
        .unwrap();
        let current_key = CompileCache::cache_key(&deps, "Release", "Off").unwrap();

        assert_ne!(legacy_key, current_key);
    }

    #[test]
    fn compile_cache_key_changes_when_dependency_content_changes() {
        let dir = TempDir::new().unwrap();
        let a = dir.path().join("a.vow");
        std::fs::write(&a, "module A").unwrap();
        let deps = DependencyManifest::from_paths(vec![a.clone()]);

        let k1 = CompileCache::cache_key(&deps, "Release", "Off").unwrap();
        std::fs::write(&a, "module A updated").unwrap();
        let k2 = CompileCache::cache_key(&deps, "Release", "Off").unwrap();

        assert_ne!(k1, k2);
    }

    #[test]
    fn compile_cache_key_returns_none_when_dependency_unreadable() {
        // Fail-closed: any per-dep canonicalize / open / read error must cause
        // key generation to return None so the cache is not consulted with an
        // incomplete dep set (which could collide with a previously-cached key).
        let dir = TempDir::new().unwrap();
        let missing = dir.path().join("does_not_exist.vow");
        let deps = DependencyManifest::from_paths(vec![missing]);
        assert!(CompileCache::cache_key(&deps, "Release", "Off").is_none());
    }
}
