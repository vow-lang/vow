use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};
use std::path::{Path, PathBuf};

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
