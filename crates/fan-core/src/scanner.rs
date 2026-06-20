use crate::types::RawFileInfo;
use std::fs;
use std::path::Path;
use std::time::UNIX_EPOCH;
use walkdir::WalkDir;

pub struct Scanner {
    include_dirs: Vec<String>,
    exclude_patterns: Vec<String>,
    source_server: String,
}

impl Scanner {
    pub fn new(include: Vec<String>, exclude: Vec<String>, source_server: String) -> Self {
        Self { include_dirs: include, exclude_patterns: exclude, source_server }
    }

    pub fn scan(&self) -> impl Iterator<Item = RawFileInfo> + '_ {
        self.include_dirs.iter().flat_map(move |dir| {
            WalkDir::new(dir)
                .follow_links(false)
                .into_iter()
                .filter_entry(move |e| !self.is_excluded(e.path()))
                .filter_map(|entry| {
                    let entry = entry.ok()?;
                    if !entry.file_type().is_file() { return None; }
                    Some(self.collect_info(entry.path()))
                })
        })
    }

    fn collect_info(&self, path: &Path) -> RawFileInfo {
        let meta = fs::metadata(path).ok();
        let size = meta.as_ref().map(|m| m.len()).unwrap_or(0);
        let mtime = meta.and_then(|m| m.modified().ok())
            .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
            .map(|d| d.as_secs() as i64)
            .unwrap_or(0);
        let magic = read_magic(path);
        let mime = mime_guess::from_path(path).first_or_octet_stream().to_string();
        RawFileInfo {
            path: path.to_path_buf(),
            source_server: self.source_server.clone(),
            size,
            mtime_secs: mtime,
            hash_sha256: None,
            magic_bytes: magic,
            mime_type: mime,
        }
    }

    fn is_excluded(&self, path: &Path) -> bool {
        let path_str = path.to_string_lossy();
        let file_name = path.file_name().map(|n| n.to_string_lossy());
        self.exclude_patterns.iter().any(|pat| {
            if pat.starts_with("*.") {
                path_str.ends_with(&pat[1..])
            } else if pat == ".*" {
                file_name.as_ref().map(|n| n.starts_with('.')).unwrap_or(false)
            } else {
                path_str.starts_with(pat.as_str())
            }
        })
    }

    pub fn scan_single(&self, path: &Path) -> Option<RawFileInfo> {
        if path.is_file() { Some(self.collect_info(path)) } else { None }
    }
}

fn read_magic(path: &Path) -> Vec<u8> {
    fs::File::open(path)
        .ok()
        .and_then(|mut f| {
            use std::io::Read;
            let mut buf = vec![0u8; 512];
            let n = f.read(&mut buf).ok()?;
            buf.truncate(n);
            Some(buf)
        })
        .unwrap_or_default()
}
