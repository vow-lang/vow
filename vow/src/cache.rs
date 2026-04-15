use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};
use std::io::Write;
use std::path::{Path, PathBuf};

use vow_verify::Counterexample;

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

    pub fn cache_key(source_files: &[PathBuf], mode: &str, trace: &str) -> String {
        let mut hasher = DefaultHasher::new();
        let mut entries: Vec<(String, u64)> = source_files
            .iter()
            .filter_map(|p| {
                let canon = p.canonicalize().ok()?;
                let mtime = std::fs::metadata(&canon)
                    .ok()?
                    .modified()
                    .ok()?
                    .duration_since(std::time::UNIX_EPOCH)
                    .ok()?
                    .as_secs();
                Some((canon.to_string_lossy().to_string(), mtime))
            })
            .collect();
        entries.sort_by(|a, b| a.0.cmp(&b.0));
        for (path, mtime) in &entries {
            path.hash(&mut hasher);
            mtime.hash(&mut hasher);
        }
        mode.hash(&mut hasher);
        trace.hash(&mut hasher);
        format!("{:016x}", hasher.finish())
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

#[derive(Debug)]
pub enum CachedVerifyResult {
    Proven,
    Failed {
        vow_id: Option<u32>,
        description: String,
        values: Vec<(String, String)>,
        block_visits: Vec<u32>,
        raw_output: String,
    },
}

impl CachedVerifyResult {
    pub fn to_counterexample(&self) -> Option<Counterexample> {
        match self {
            CachedVerifyResult::Proven => None,
            CachedVerifyResult::Failed {
                vow_id,
                description,
                values,
                block_visits,
                raw_output,
            } => Some(Counterexample {
                description: description.clone(),
                vow_id: *vow_id,
                values: values.clone(),
                block_visits: block_visits.clone(),
                raw_output: raw_output.clone(),
            }),
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

    pub fn cache_key(c_source: &str, max_k_step: u32) -> String {
        let combined = format!("{c_source}\n__max_k_step={max_k_step}");
        let hash = fnv1a_hash(combined.as_bytes());
        format!("{hash:016x}")
    }

    pub fn lookup(&self, key: &str) -> Option<CachedVerifyResult> {
        let path = self.dir.join(format!("{key}.vr"));
        let content = std::fs::read_to_string(path).ok()?;
        parse_cached_result(&content)
    }

    pub fn store(&self, key: &str, result: &CachedVerifyResult) {
        let path = self.dir.join(format!("{key}.vr"));
        let content = serialize_cached_result(result);
        let mut f = match std::fs::File::create(&path) {
            Ok(f) => f,
            Err(_) => return,
        };
        let _ = f.write_all(content.as_bytes());
    }
}

fn serialize_cached_result(result: &CachedVerifyResult) -> String {
    match result {
        CachedVerifyResult::Proven => "PROVEN\n".to_string(),
        CachedVerifyResult::Failed {
            vow_id,
            description,
            values,
            block_visits,
            raw_output,
        } => {
            let mut s = String::from("FAILED\n");
            match vow_id {
                Some(id) => s.push_str(&format!("vow_id={id}\n")),
                None => s.push_str("vow_id=\n"),
            }
            s.push_str(&format!("description={description}\n"));
            let vals: Vec<String> = values.iter().map(|(k, v)| format!("{k}:{v}")).collect();
            s.push_str(&format!("values={}\n", vals.join(",")));
            let blks: Vec<String> = block_visits.iter().map(|b| b.to_string()).collect();
            s.push_str(&format!("blocks={}\n", blks.join(",")));
            s.push_str("raw=");
            s.push_str(raw_output);
            s.push('\n');
            s
        }
    }
}

fn parse_cached_result(content: &str) -> Option<CachedVerifyResult> {
    let mut lines = content.lines();
    let status = lines.next()?;
    match status {
        "PROVEN" => Some(CachedVerifyResult::Proven),
        "FAILED" => {
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

            Some(CachedVerifyResult::Failed {
                vow_id,
                description,
                values,
                block_visits,
                raw_output,
            })
        }
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn verify_cache_roundtrip_proven() {
        let result = CachedVerifyResult::Proven;
        let serialized = serialize_cached_result(&result);
        let parsed = parse_cached_result(&serialized).unwrap();
        assert!(matches!(parsed, CachedVerifyResult::Proven));
    }

    #[test]
    fn verify_cache_roundtrip_failed() {
        let result = CachedVerifyResult::Failed {
            vow_id: Some(3),
            description: "test failure".to_string(),
            values: vec![("x".to_string(), "42".to_string())],
            block_visits: vec![0, 1, 3],
            raw_output: "raw".to_string(),
        };
        let serialized = serialize_cached_result(&result);
        let parsed = parse_cached_result(&serialized).unwrap();
        match parsed {
            CachedVerifyResult::Failed {
                vow_id,
                description,
                values,
                block_visits,
                ..
            } => {
                assert_eq!(vow_id, Some(3));
                assert_eq!(description, "test failure");
                assert_eq!(values.len(), 1);
                assert_eq!(block_visits, vec![0, 1, 3]);
            }
            _ => panic!("expected Failed"),
        }
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
        let k1 = VerifyCache::cache_key("int f() { return 0; }", 10);
        let k2 = VerifyCache::cache_key("int f() { return 0; }", 20);
        assert_ne!(k1, k2);
    }
}
