use std::io::{BufReader, Read, Write};
use std::path::{Path, PathBuf};

use vow_verify::Counterexample;

use crate::frontend::DependencyManifest;

// Bump this whenever generated object files are no longer ABI-compatible with
// existing cached artifacts. Phase 7 adds FFI wrapper stdlib intrinsics and
// runtime helper imports, so pre-cutover objects must not be reused.
const COMPILE_CACHE_ABI_VERSION: &str = "arena-phase7-ffi-wrapper-v1";

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

// Streaming FNV-1a so dependency-content hashing stays bounded-memory regardless of file size.
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
// bypass ESBMC, so successful verifications are never cached.
#[derive(Debug)]
pub struct CachedFailure {
    pub vow_id: Option<u32>,
    pub description: String,
    pub values: Vec<(String, String)>,
    pub block_visits: Vec<u32>,
    pub raw_output: String,
}

impl CachedFailure {
    pub fn to_counterexample(&self) -> Counterexample {
        Counterexample {
            description: self.description.clone(),
            vow_id: self.vow_id,
            values: self.values.clone(),
            block_visits: self.block_visits.clone(),
            raw_output: self.raw_output.clone(),
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

    pub fn cache_key(c_source: &str, max_k_step: u32, solver: &str, encoding: &str) -> String {
        let combined = format!(
            "{c_source}\n__max_k_step={max_k_step}\n__solver={solver}\n__encoding={encoding}"
        );
        let hash = fnv1a_hash(combined.as_bytes());
        format!("{hash:016x}")
    }

    pub fn lookup(&self, key: &str) -> Option<CachedFailure> {
        let path = self.dir.join(format!("{key}.vr"));
        let content = std::fs::read_to_string(path).ok()?;
        parse_cached_result(&content)
    }

    pub fn store(&self, key: &str, result: &CachedFailure) {
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
    let mut s = String::from("FAILED\n");
    match result.vow_id {
        Some(id) => s.push_str(&format!("vow_id={id}\n")),
        None => s.push_str("vow_id=\n"),
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
    if status != "FAILED" {
        return None;
    }
    let mut vow_id: Option<u32> = None;
    let mut description = String::new();
    let mut values = Vec::new();
    let mut block_visits = Vec::new();
    let mut raw_output = String::new();

    for line in lines {
        if let Some(rest) = line.strip_prefix("vow_id=") {
            vow_id = rest.parse().ok();
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

    Some(CachedFailure {
        vow_id,
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
    fn verify_cache_roundtrip_failed() {
        let result = CachedFailure {
            vow_id: Some(3),
            description: "test failure".to_string(),
            values: vec![("x".to_string(), "42".to_string())],
            block_visits: vec![0, 1, 3],
            raw_output: "raw".to_string(),
        };
        let serialized = serialize_cached_result(&result);
        let parsed = parse_cached_result(&serialized).unwrap();
        assert_eq!(parsed.vow_id, Some(3));
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
        let k1 = VerifyCache::cache_key("int f() { return 0; }", 10, "boolector", "bv");
        let k2 = VerifyCache::cache_key("int f() { return 0; }", 20, "boolector", "bv");
        assert_ne!(k1, k2);
    }

    #[test]
    fn cache_key_includes_solver_encoding() {
        let k1 = VerifyCache::cache_key("int f() { return 0; }", 10, "boolector", "bv");
        let k2 = VerifyCache::cache_key("int f() { return 0; }", 10, "z3", "ir");
        assert_ne!(k1, k2);
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
