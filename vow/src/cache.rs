use std::io::{BufReader, Read, Write};
use std::path::{Path, PathBuf};

use vow_verify::{ArithOverflowSite, CalleePrecondition, Counterexample, SolverConfig};

use crate::frontend::DependencyManifest;

// Counterexample values are serialized after source-name mapping. Version the
// record header whenever that public name mapping changes so an old failure
// cannot re-enter the compiler with stale agent-facing keys.
const VERIFY_CACHE_FAILURE_HEADER: &str = "FAILED v2";

// Bump this whenever generated object files are no longer ABI-compatible with
// existing cached artifacts. Static string literals, full-width block arena
// stack slots, UTF-8-preserving string lexing, contextual narrow-integer
// reductions, narrow Option parameter payloads, width-preserving unary
// negation, and aggregate match-payload tags (which also change FieldGet
// indices for the same source) all change generated object contents, so
// pre-cutover objects must not be reused.
const COMPILE_CACHE_ABI_VERSION: &str =
    "static-string-arena-slot-utf8-lexer-narrow-unary-match-aggregate-v5";

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
    /// The `arith:` site, persisted as its own fields rather than re-derived
    /// from `raw_output` on replay: `raw=` is written as the ESBMC output
    /// verbatim but read back a line at a time, so a cached `raw_output` keeps
    /// only its first line and the `Violated property:` block is gone (#585).
    arith_overflow: Option<ArithOverflowSite>,
    description: String,
    values: Vec<(String, String)>,
    block_visits: Vec<u32>,
    raw_output: String,
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
        let content = serialize_cached_result(result);
        let mut f = match std::fs::File::create(&path) {
            Ok(f) => f,
            Err(_) => return,
        };
        let _ = f.write_all(content.as_bytes());
    }
}

fn serialize_cached_result(result: &CachedFailure) -> String {
    let mut s = format!("{VERIFY_CACHE_FAILURE_HEADER}\n");
    match result.vow_id {
        Some(id) => s.push_str(&format!("vow_id={id}\n")),
        None => s.push_str("vow_id=\n"),
    }
    match result.callee_precondition {
        Some(pre) => {
            s.push_str(&format!("callee_precondition_func_id={}\n", pre.func_id));
            s.push_str(&format!("callee_precondition_vow_id={}\n", pre.vow_id));
        }
        None => {
            s.push_str("callee_precondition_func_id=\n");
            s.push_str("callee_precondition_vow_id=\n");
        }
    }
    match result.arith_overflow {
        Some(site) => {
            s.push_str(&format!("arith_cause={}\n", site.abort.label()));
            s.push_str(&format!("arith_func={}\n", site.func_id));
            s.push_str(&format!("arith_start={}\n", site.span_start));
            s.push_str(&format!("arith_len={}\n", site.span_len));
        }
        None => {
            s.push_str("arith_cause=\n");
            s.push_str("arith_func=\n");
            s.push_str("arith_start=\n");
            s.push_str("arith_len=\n");
        }
    }
    s.push_str(&format!("description={}\n", result.description));
    let vals: Vec<String> = result
        .values
        .iter()
        .map(|(k, v)| format!("{k}:{v}"))
        .collect();
    s.push_str(&format!("values={}\n", vals.join(",")));
    let blks: Vec<String> = result.block_visits.iter().map(|b| b.to_string()).collect();
    s.push_str(&format!("blocks={}\n", blks.join(",")));
    s.push_str("raw=");
    s.push_str(&result.raw_output);
    s.push('\n');
    s
}

fn parse_cached_result(content: &str) -> Option<CachedFailure> {
    let mut lines = content.lines();
    let status = lines.next()?;
    // Security: only honor cached FAILED entries. A forged "PROVEN" file must
    // never bypass ESBMC, so PROVEN content is discarded here.
    if status != VERIFY_CACHE_FAILURE_HEADER {
        return None;
    }
    let mut vow_id: Option<u32> = None;
    let mut callee_precondition_func_id: Option<u32> = None;
    let mut callee_precondition_vow_id: Option<u32> = None;
    let mut description = String::new();
    let mut values = Vec::new();
    let mut block_visits = Vec::new();
    let mut raw_output = String::new();
    let mut arith_cause: Option<vow_verify::ArithAbort> = None;
    let mut arith_func: Option<u32> = None;
    let mut arith_start: Option<u32> = None;
    let mut arith_len: Option<u32> = None;

    for line in lines {
        if let Some(rest) = line.strip_prefix("vow_id=") {
            vow_id = rest.parse().ok();
        } else if let Some(rest) = line.strip_prefix("callee_precondition_func_id=") {
            callee_precondition_func_id = rest.parse().ok();
        } else if let Some(rest) = line.strip_prefix("callee_precondition_vow_id=") {
            callee_precondition_vow_id = rest.parse().ok();
        } else if let Some(rest) = line.strip_prefix("arith_cause=") {
            arith_cause = vow_verify::ArithAbort::from_label(rest);
        } else if let Some(rest) = line.strip_prefix("arith_func=") {
            arith_func = rest.parse().ok();
        } else if let Some(rest) = line.strip_prefix("arith_start=") {
            arith_start = rest.parse().ok();
        } else if let Some(rest) = line.strip_prefix("arith_len=") {
            arith_len = rest.parse().ok();
        } else if let Some(rest) = line.strip_prefix("description=") {
            description = rest.to_string();
        } else if let Some(rest) = line.strip_prefix("values=") {
            if !rest.is_empty() {
                for pair in rest.split(',') {
                    if let Some((k, v)) = pair.split_once(':') {
                        values.push((k.to_string(), v.to_string()));
                    }
                }
            }
        } else if let Some(rest) = line.strip_prefix("blocks=") {
            if !rest.is_empty() {
                for b in rest.split(',') {
                    if let Ok(n) = b.parse() {
                        block_visits.push(n);
                    }
                }
            }
        } else if let Some(rest) = line.strip_prefix("raw=") {
            raw_output = rest.to_string();
        }
    }

    let callee_precondition = callee_precondition_func_id
        .zip(callee_precondition_vow_id)
        .map(|(func_id, vow_id)| CalleePrecondition { func_id, vow_id });
    // Every part or none: a half-written site would mislocate the diagnostic, so
    // an incomplete entry reports no site at all.
    let arith_overflow = arith_cause
        .zip(arith_func)
        .zip(arith_start.zip(arith_len))
        .map(
            |((abort, func_id), (span_start, span_len))| ArithOverflowSite {
                abort,
                func_id,
                span_start,
                span_len,
            },
        );

    Some(CachedFailure {
        vow_id,
        callee_precondition,
        arith_overflow,
        description,
        values,
        block_visits,
        raw_output,
    })
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
        let serialized = serialize_cached_result(&result);
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
