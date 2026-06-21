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

/// RemoteScanner discovers files on a remote server via SSH.
/// Uses `ssh <host> find <root> -type f -printf` for file listing
/// and `ssh <host> head -c 512 <path> | base64` for magic bytes.
pub struct RemoteScanner {
    pub server_name: String,
    pub ssh_host: String,
    pub scan_roots: Vec<String>,
}

#[derive(Debug)]
pub struct RemoteFileEntry {
    pub path: String,
    pub size: u64,
    pub mtime_secs: i64,
}

impl RemoteScanner {
    pub fn new(server_name: String, ssh_host: String, scan_roots: Vec<String>) -> Self {
        Self { server_name, ssh_host, scan_roots }
    }

    /// Discover files across all scan roots: use cache if available, otherwise find.
    pub fn discover_files(&self) -> Result<Vec<RemoteFileEntry>, String> {
        let mut all_files = Vec::new();
        for root in &self.scan_roots {
            let entries = self.discover_one_root(root)?;
            all_files.extend(entries);
        }
        Ok(all_files)
    }

    /// Discover files for a single root directory.
    fn discover_one_root(&self, scan_root: &str) -> Result<Vec<RemoteFileEntry>, String> {
        let cache_name = scan_root.trim_matches('/').replace('/', "_");
        let cache_path = format!("$HOME/.fan-cache/{}_files.txt", cache_name);

        // Step 1: Try to read the cache file
        let mut files = Vec::new();
        let read_cache_cmd = format!(
            "mkdir -p $HOME/.fan-cache && cat {} 2>/dev/null",
            cache_path
        );
        let cache_output = ssh_exec(&self.ssh_host, &read_cache_cmd)?;
        let mut cache_count = 0usize;

        if !cache_output.is_empty() {
            // Parse cache entries
            for line in cache_output.lines() {
                let line = line.trim();
                if line.is_empty() { continue; }
                let parts: Vec<&str> = line.splitn(3, '\t').collect();
                if parts.len() != 3 { continue; }
                let size: u64 = parts[1].parse().unwrap_or(0);
                let mtime_float: f64 = parts[2].parse().unwrap_or(0.0);
                files.push(RemoteFileEntry {
                    path: parts[0].to_string(),
                    size,
                    mtime_secs: mtime_float as i64,
                });
            }
            cache_count = files.len();
        }

        if cache_count > 0 {
            // Step 2: Incremental — find files newer than the cache file
            let incr_cmd = format!(
                "find {} -type f -newer {} -printf '%p\\t%s\\t%T@\\n' 2>/dev/null || true",
                shell_escape(scan_root),
                cache_path
            );
            if let Ok(incr_output) = ssh_exec(&self.ssh_host, &incr_cmd) {
                let mut new_count = 0usize;
                for line in incr_output.lines() {
                    let line = line.trim();
                    if line.is_empty() { continue; }
                    let parts: Vec<&str> = line.splitn(3, '\t').collect();
                    if parts.len() != 3 { continue; }
                    let size: u64 = parts[1].parse().unwrap_or(0);
                    let mtime_float: f64 = parts[2].parse().unwrap_or(0.0);
                    files.push(RemoteFileEntry {
                        path: parts[0].to_string(),
                        size,
                        mtime_secs: mtime_float as i64,
                    });
                    new_count += 1;
                }
                eprintln!("  cache: {} files, incremental: {} new", cache_count, new_count);
            }

            // Step 3: Rebuild cache in background (async refresh for next time)
            let rebuild_cmd = format!(
                "find {} -type f -printf '%p\\t%s\\t%T@\\n' 2>/dev/null > {}.tmp && mv {}.tmp {}",
                shell_escape(scan_root),
                cache_path, cache_path, cache_path
            );
            // Fire-and-forget: don't wait for rebuild
            let _ = ssh_exec_bg(&self.ssh_host, &rebuild_cmd);
        } else {
            // Step 4: No cache — build it and return results
            eprintln!("  building file cache (first run)...");
            let build_cmd = format!(
                "mkdir -p $HOME/.fan-cache && find {} -type f -printf '%p\\t%s\\t%T@\\n' 2>/dev/null | tee {}.tmp",
                shell_escape(scan_root),
                cache_path
            );
            let output = ssh_exec(&self.ssh_host, &build_cmd)?;
            let _ = ssh_exec(&self.ssh_host,
                &format!("mv {}.tmp {}", cache_path, cache_path));

            for line in output.lines() {
                let line = line.trim();
                if line.is_empty() { continue; }
                let parts: Vec<&str> = line.splitn(3, '\t').collect();
                if parts.len() != 3 { continue; }
                let size: u64 = parts[1].parse().unwrap_or(0);
                let mtime_float: f64 = parts[2].parse().unwrap_or(0.0);
                files.push(RemoteFileEntry {
                    path: parts[0].to_string(),
                    size,
                    mtime_secs: mtime_float as i64,
                });
            }
            eprintln!("  cache built: {} files", files.len());
        }

        Ok(files)
    }

    /// Fetch magic bytes (first 512 bytes) from a remote file via SSH.
    pub fn fetch_magic_bytes(&self, remote_path: &str) -> Vec<u8> {
        let cmd = format!(
            "head -c 512 {} | base64 2>/dev/null",
            shell_escape(remote_path)
        );
        match ssh_exec(&self.ssh_host, &cmd) {
            Ok(output) => {
                let trimmed = output.trim();
                if trimmed.is_empty() {
                    return Vec::new();
                }
                let single_line = trimmed.replace('\n', "").replace('\r', "");
                base64_decode(&single_line).unwrap_or_default()
            }
            Err(_) => Vec::new(),
        }
    }

    /// Scan: discover + optionally fetch magic bytes, yield RawFileInfo.
    pub fn scan(&self, fetch_magic: bool) -> Result<Vec<RawFileInfo>, String> {
        let entries = self.discover_files()?;
        let mut results = Vec::with_capacity(entries.len());
        for entry in &entries {
            let magic_bytes = if fetch_magic {
                self.fetch_magic_bytes(&entry.path)
            } else {
                Vec::new()
            };
            let mime = mime_guess::from_path(&entry.path)
                .first_or_octet_stream()
                .to_string();
            results.push(RawFileInfo {
                path: std::path::PathBuf::from(&entry.path),
                source_server: self.server_name.clone(),
                size: entry.size,
                mtime_secs: entry.mtime_secs,
                hash_sha256: None,
                magic_bytes,
                mime_type: mime,
            });
        }
        Ok(results)
    }
}

/// Execute an SSH command in background (fire-and-forget).
fn ssh_exec_bg(host: &str, cmd: &str) -> Result<(), String> {
    std::process::Command::new("ssh")
        .args(["-o", "ConnectTimeout=10", "-o", "BatchMode=yes", host, cmd])
        .spawn()
        .map_err(|e| format!("ssh bg failed: {}", e))?;
    Ok(())
}

/// Execute a command via SSH, returning stdout on success.
fn ssh_exec(host: &str, cmd: &str) -> Result<String, String> {
    let output = std::process::Command::new("ssh")
        .args(["-o", "ConnectTimeout=10", "-o", "BatchMode=yes", host, cmd])
        .output()
        .map_err(|e| format!("ssh failed to start: {}", e))?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(format!("ssh exited with {}: {}", output.status, stderr.trim()));
    }
    Ok(String::from_utf8_lossy(&output.stdout).to_string())
}

/// Basic shell escaping for single-quoted strings.
fn shell_escape(s: &str) -> String {
    format!("'{}'", s.replace('\'', "'\\''"))
}

/// Minimal base64 decode using only std.
fn base64_decode(input: &str) -> Result<Vec<u8>, String> {
    const CHARS: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut buf = Vec::with_capacity(input.len() * 3 / 4);
    let mut accum: u32 = 0;
    let mut bits: u32 = 0;
    for b in input.bytes() {
        if b == b'=' { break; }
        if b == b'\n' || b == b'\r' || b == b' ' { continue; }
        let idx = CHARS.iter().position(|&c| c == b).ok_or("invalid base64 char")?;
        accum = (accum << 6) | idx as u32;
        bits += 6;
        if bits >= 8 {
            bits -= 8;
            buf.push((accum >> bits) as u8);
        }
    }
    Ok(buf)
}
