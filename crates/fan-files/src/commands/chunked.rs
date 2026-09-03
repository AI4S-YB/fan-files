//! 统一分块层：分块计算、清单持久化、缺失块调度（QUIC 多流 / relay 多线程共用）

use serde::{Deserialize, Serialize};

/// 默认块大小 4MB
pub const DEFAULT_CHUNK_SIZE: u64 = 4 * 1024 * 1024;
/// 块数上限（超限自动放大块）
pub const MAX_CHUNK_COUNT: u64 = 512;

/// 一个块：文件内 offset + size
#[derive(Debug, Clone, PartialEq)]
pub struct Chunk {
    pub index: u32,
    pub offset: u64,
    pub size: u64,
}

/// 分块计划：文件 → 块列表（自适应块大小，块数 ≤512）
pub fn chunk_plan(file_size: u64, mut chunk_size: u64) -> Vec<Chunk> {
    if file_size == 0 {
        return vec![];
    }
    if chunk_size == 0 {
        chunk_size = DEFAULT_CHUNK_SIZE;
    }
    // 自适应：块数超上限则放大块
    while file_size.div_ceil(chunk_size) > MAX_CHUNK_COUNT {
        chunk_size *= 2;
    }
    let mut out = Vec::new();
    let mut offset = 0u64;
    let mut index = 0u32;
    while offset < file_size {
        let size = chunk_size.min(file_size - offset);
        out.push(Chunk { index, offset, size });
        offset += size;
        index += 1;
    }
    out
}

/// 清单：接收方持久化的已完成块集合（JSON）
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
pub struct Manifest {
    pub file_name: String,
    pub file_size: u64,
    pub chunk_size: u64,
    pub done: Vec<u32>,
}

/// 清单路径：.fan-files/partial/<hash16>.chunks.json
pub fn manifest_path(hash16: &str) -> std::path::PathBuf {
    fan_core::config::dirs_fan().join("partial").join(format!("{hash16}.chunks.json"))
}

/// 部分文件路径：.fan-files/partial/<hash16>.part（块按 offset 稀疏写入）
pub fn partial_path(hash16: &str) -> std::path::PathBuf {
    fan_core::config::dirs_fan().join("partial").join(format!("{hash16}.part"))
}

/// 保存清单（原子：临时文件 + rename，防崩溃损坏）
pub fn save_manifest(hash16: &str, m: &Manifest) -> std::io::Result<()> {
    let path = manifest_path(hash16);
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let tmp = path.with_extension("json.tmp");
    std::fs::write(&tmp, serde_json::to_vec(m).map_err(std::io::Error::other)?)?;
    std::fs::rename(tmp, path)
}

/// 加载清单（不存在 → None；损坏 → None（视为无清单全量重传））
pub fn load_manifest(hash16: &str) -> Option<Manifest> {
    let path = manifest_path(hash16);
    let data = std::fs::read(&path).ok()?;
    serde_json::from_slice(&data).ok()
}

/// 清理清单与部分文件（传输完成）
pub fn clear_manifest(hash16: &str) {
    let _ = std::fs::remove_file(manifest_path(hash16));
    let _ = std::fs::remove_file(partial_path(hash16));
}

/// 缺失块集合：done 之外的所有块索引（保持有序）
pub fn missing_chunks(plan_len: usize, done: &std::collections::BTreeSet<u32>) -> Vec<u32> {
    (0..plan_len as u32).filter(|i| !done.contains(i)).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn chunk_plan_small_file_single_chunk() {
        let plan = chunk_plan(3 * 1024 * 1024, DEFAULT_CHUNK_SIZE);
        assert_eq!(plan.len(), 1);
        assert_eq!(plan[0].offset, 0);
        assert_eq!(plan[0].size, 3 * 1024 * 1024);
    }

    #[test]
    fn chunk_plan_multi_chunk() {
        let plan = chunk_plan(10 * 1024 * 1024, DEFAULT_CHUNK_SIZE);
        assert_eq!(plan.len(), 3);
        assert_eq!(plan[0].offset, 0);
        assert_eq!(plan[1].offset, 4 * 1024 * 1024);
        assert_eq!(plan[2].offset, 8 * 1024 * 1024);
        assert_eq!(plan[2].size, 2 * 1024 * 1024);
    }

    #[test]
    fn chunk_plan_caps_chunk_count_512() {
        let plan = chunk_plan(3 * 1024 * 1024 * 1024, DEFAULT_CHUNK_SIZE);
        assert!(plan.len() <= 512, "块数应 ≤512，实际 {}", plan.len());
        assert!(plan[0].size >= 8 * 1024 * 1024);
    }

    #[test]
    fn manifest_roundtrip_and_atomic_save() {
        let m = Manifest {
            file_name: "test.bin".into(),
            file_size: 10 * 1024 * 1024,
            chunk_size: DEFAULT_CHUNK_SIZE,
            done: vec![0, 1],
        };
        let hash = "abcdef0123456789";
        save_manifest(hash, &m).unwrap();
        let back = load_manifest(hash).unwrap();
        assert_eq!(back, m);
        // 无残留临时文件
        assert!(!manifest_path(hash).with_extension("json.tmp").exists());
        clear_manifest(hash);
        assert!(load_manifest(hash).is_none());
    }

    #[test]
    fn missing_chunks_computes() {
        let plan = chunk_plan(20 * 1024 * 1024, DEFAULT_CHUNK_SIZE); // 5 块
        let done: std::collections::BTreeSet<u32> = [0, 1].into_iter().collect();
        assert_eq!(missing_chunks(plan.len(), &done), vec![2, 3, 4]);
        let empty: std::collections::BTreeSet<u32> = std::collections::BTreeSet::new();
        assert_eq!(missing_chunks(plan.len(), &empty), vec![0, 1, 2, 3, 4]);
    }
}
