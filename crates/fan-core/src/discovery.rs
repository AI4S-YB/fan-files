//! Progressive Discovery: Phase A — lightweight directory walk + LLM pre-filter.
//!
//! Instead of scanning all files first, this module walks the directory structure
//! at a shallow depth (3-4 levels), sends the tree to the LLM, and returns
//! only the directories worth indexing. The heavy file scanning (Phase B) then
//! targets only these valuable directories.

use crate::llm::LlmClient;
use std::collections::HashMap;
use std::path::Path;
/// A lightweight directory node built from filesystem metadata only.
/// Does NOT open files, read magic bytes, or touch SQLite.
#[derive(Debug, Clone)]
pub struct LightDirNode {
    pub name: String,
    pub path: String,
    pub file_count: usize,
    pub subdir_count: usize,
    pub extensions: Vec<(String, usize)>,
    pub subdirs: Vec<LightDirNode>,
}

/// Walk a directory tree to the given depth, collecting only directory-level
/// metadata (file counts + extension distributions). Does NOT read file contents.
pub fn lightweight_walk(root: &str, depth: u32) -> LightDirNode {
    let root_path = Path::new(root);
    let name = root_path
        .file_name().map(|n| n.to_string_lossy().to_string())
        .unwrap_or_else(|| root.to_string());

    let mut node = LightDirNode {
        name,
        path: root.to_string(),
        file_count: 0,
        subdir_count: 0,
        extensions: Vec::new(),
        subdirs: Vec::new(),
    };

    if depth == 0 { return node; }

    let mut ext_counts: HashMap<String, usize> = HashMap::new();

    if let Ok(entries) = std::fs::read_dir(root) {
        for entry in entries.flatten() {
            let ft = entry.file_type().ok();
            if ft.as_ref().map_or(false, |t| t.is_dir()) {
                node.subdir_count += 1;
                if depth > 1 {
                    let sub_path = entry.path().to_string_lossy().to_string();
                    let child = lightweight_walk(&sub_path, depth - 1);
                    if child.file_count > 0 || child.subdir_count > 0 {
                        node.subdirs.push(child);
                    }
                }
            } else if ft.as_ref().map_or(false, |t| t.is_file()) {
                node.file_count += 1;
                let fname = entry.file_name().to_string_lossy().to_string();
                let ext = light_file_extension(&fname);
                *ext_counts.entry(ext).or_insert(0) += 1;
            }
        }
    }

    let mut ext_vec: Vec<_> = ext_counts.into_iter().collect();
    ext_vec.sort_by(|a, b| b.1.cmp(&a.1));
    ext_vec.truncate(5);
    node.extensions = ext_vec;

    node.subdirs.sort_by(|a, b| b.file_count.cmp(&a.file_count));
    // Limit to top 20 subdirs to keep prompt size manageable
    node.subdirs.truncate(20);

    node
}

/// Lightweight extension extraction (no file open).
fn light_file_extension(name: &str) -> String {
    let lower = name.to_lowercase();
    for compound in &[".fastq.gz", ".fq.gz", ".vcf.gz", ".gff.gz", ".tsv.gz", ".csv.gz", ".txt.gz", ".tab.gz", ".fa.gz"] {
        if lower.ends_with(compound) { return compound[1..].to_string(); }
    }
    if let Some(pos) = name.rfind('.') {
        name[pos+1..].to_lowercase()
    } else {
        "(noext)".to_string()
    }
}

/// Convert a light directory tree to a prompt for the LLM.
pub fn light_tree_to_prompt(root: &LightDirNode, indent: usize) -> String {
    let mut lines = Vec::new();
    let prefix = "  ".repeat(indent);
    let ext_summary: Vec<String> = root.extensions.iter()
        .map(|(e, c)| format!("{}×{}", e, c))
        .collect();

    if root.subdirs.is_empty() {
        lines.push(format!(
            "{}📁 {}/  ({} files: {})",
            prefix, root.name, root.file_count, ext_summary.join(", ")
        ));
    } else {
        lines.push(format!(
            "{}📁 {}/  ({} files: {}, {} subdirs)",
            prefix, root.name, root.file_count, ext_summary.join(", "), root.subdirs.len()
        ));
        for child in &root.subdirs {
            lines.push(light_tree_to_prompt(child, indent + 1));
        }
    }
    lines.join("\n")
}

/// Phase A: light walk + LLM → scan_targets + skip_dirs.
/// Returns (paths_to_scan, paths_to_skip).
pub fn run_phase_a(
    scan_root: &str,
    llm_client: &LlmClient,
) -> Result<(Vec<String>, Vec<String>), Box<dyn std::error::Error>> {
    if !llm_client.is_configured() {
        return Err("LLM not configured".into());
    }

    // Lightweight walk at depth 3
    eprintln!("  Phase A: lightweight directory walk (depth 3)...");
    let tree = lightweight_walk(scan_root, 3);
    let prompt = light_tree_to_prompt(&tree, 0);
    eprintln!("  Phase A: tree built ({} chars prompt)", prompt.len());

    let full_prompt = format!(
        "你是数据管理助手。下面是一个目录的轻量扫描结果(只读了目录名和文件扩展名分布,没有读文件内容)。\n\n\
         任务:\n\
         1. 判断哪些子目录值得深入扫描(scan=true)\n\
         2. 判断哪些子目录应该跳过(skip=true, 如 .pnpm-store, node_modules, __pycache__, scripts, venv, .git, dist, cache)\n\
         3. 对 scan 的目录, 推断可能的物种/实验类型(可选)\n\
         输出JSON: {{\"targets\":[{{\"path\":\"子目录路径\",\"species\":\"推测物种\",\"assay\":\"推测实验\"}}], \
         \"skips\":[{{\"path\":\"路径\",\"reason\":\"原因\"}}]}}\n\n{}",
        prompt
    );

    let system_prompt = "你是数据管理助手。你看到的目录树来自轻量扫描(只看目录名+扩展名分布)。请判断哪些目录值得深度扫描。";

    let body = serde_json::json!({
        "model": llm_client.config.model,
        "messages": [
            {"role": "system", "content": system_prompt},
            {"role": "user", "content": full_prompt}
        ],
        "response_format": {"type": "json_object"},
        "temperature": 0.1,
        "max_tokens": 8192
    });

    eprintln!("  Phase A: asking LLM to classify...");
    let response: serde_json::Value = crate::llm::llm_api_call_with_retry(&llm_client.config, &body, 3)?;
    let content = response["choices"][0]["message"]["content"]
        .as_str().ok_or("No content")?;

    let output: serde_json::Value = serde_json::from_str(content)?;

    let targets: Vec<String> = output["targets"].as_array()
        .map(|a| a.iter().filter_map(|v| v["path"].as_str().map(String::from)).collect())
        .unwrap_or_default();
    let skips: Vec<String> = output["skips"].as_array()
        .map(|a| a.iter().filter_map(|v| v["path"].as_str().map(String::from)).collect())
        .unwrap_or_default();

    eprintln!("  Phase A: {} targets to scan, {} dirs to skip", targets.len(), skips.len());
    Ok((targets, skips))
}

/// Recursive Phase A: progressively expand depth 3→5→7 based on LLM "need_deeper" signals.
/// Each round walks from the previous round's truncation point, not from the root.
pub fn run_recursive_phase_a(
    scan_root: &str,
    llm_client: &LlmClient,
    max_depth: u32,
) -> Result<(Vec<String>, Vec<String>), Box<dyn std::error::Error>> {
    if !llm_client.is_configured() {
        return Err("LLM not configured".into());
    }

    let mut all_targets: Vec<String> = Vec::new();
    let mut all_skips: Vec<String> = Vec::new();
    let mut current_roots: Vec<String> = vec![scan_root.to_string()];
    let mut round = 1;

    while !current_roots.is_empty() && round <= 3 {
        let walk_depth: u32 = 3; // relative depth from each truncation point

        eprintln!("  Phase A Round {}: {} root(s), depth {}...", round, current_roots.len(), walk_depth);

        let mut next_roots: Vec<String> = Vec::new();

        for root in &current_roots {
            let tree = lightweight_walk(root, walk_depth);
            let prompt = light_tree_to_prompt(&tree, 0);
            if prompt.len() < 50 { continue; }

            let full_prompt = format!(
                "你是数据管理助手。下面是一个目录的轻量扫描结果(只读目录名和扩展名分布)。\n\n\
                 任务:\n\
                 1. 对每个子目录, 判断: scan(值得深度扫描), skip(跳过), 或 deeper(需要再往下看几层才能判断)\n\
                 2. 标记deeper的目录通常是: 目录名不明确, 子目录很多, 或扩展名分布不典型\n\
                 3. 明显噪音: 直接skip (.pnpm-store, node_modules, __pycache__, scripts, venv, .git, dist, cache, working_dir, temp)\n\
                 输出JSON: {{\"targets\":[{{\"path\":\"路径\"}}], \"skips\":[{{\"path\":\"路径\",\"reason\":\"原因\"}}], \"deeper\":[{{\"path\":\"路径\"}}]}}\n\n{}",
                prompt
            );

            let body = serde_json::json!({
                "model": llm_client.config.model,
                "messages": [
                    {"role": "system", "content": "你是数据管理助手。你的任务是递归地扫描目录树，逐层判断哪些值得深入、哪些跳过、哪些需要再展开一层。"},
                    {"role": "user", "content": full_prompt}
                ],
                "response_format": {"type": "json_object"},
                "temperature": 0.1,
                "max_tokens": 4096
            });

            match crate::llm::llm_api_call_with_retry(&llm_client.config, &body, 2) {
                Ok(response) => {
                    let content = response["choices"][0]["message"]["content"].as_str().unwrap_or("");
                    if let Ok(output) = serde_json::from_str::<serde_json::Value>(content) {
                        // Collect targets
                        if let Some(arr) = output["targets"].as_array() {
                            for v in arr {
                                if let Some(p) = v["path"].as_str() {
                                    let abs = if p.starts_with('/') { p.to_string() }
                                        else { format!("{}/{}", root.trim_end_matches('/'), p.trim_start_matches('/')) };
                                    all_targets.push(abs);
                                }
                            }
                        }
                        // Collect skips
                        if let Some(arr) = output["skips"].as_array() {
                            for v in arr {
                                if let Some(p) = v["path"].as_str() {
                                    all_skips.push(p.to_string());
                                }
                            }
                        }
                        // Collect deeper — these become roots for next round
                        if let Some(arr) = output["deeper"].as_array() {
                            for v in arr {
                                if let Some(p) = v["path"].as_str() {
                                    let abs = if p.starts_with('/') { p.to_string() }
                                        else { format!("{}/{}", root.trim_end_matches('/'), p.trim_start_matches('/')) };
                                    next_roots.push(abs);
                                }
                            }
                        }
                    }
                }
                Err(e) => eprintln!("  Round {} LLM failed: {}", round, e),
            }
        }

        eprintln!("  Round {}: {} targets, {} deeper", round, all_targets.len(), next_roots.len());
        current_roots = next_roots;
        round += 1;

        // Cap total depth
        if round as u32 * 3 > max_depth { break; }
    }

    // Dedup
    all_targets.sort();
    all_targets.dedup();
    all_skips.sort();
    all_skips.dedup();

    eprintln!("  Recursive Phase A complete: {} targets, {} skipped", all_targets.len(), all_skips.len());
    Ok((all_targets, all_skips))
}
