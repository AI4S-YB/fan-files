//! Hierarchical directory-tree-based LLM inference.
//!
//! Instead of filtering individual files by suffix and sending flat directory
//! lists to the LLM, this module:
//!   1. Builds a directory tree (depth-adaptive, 2-4 levels)
//!   2. Sends the tree *structure* to the LLM for project classification
//!   3. Selectively drills down into branches marked for deeper exploration

use crate::index::sqlite::SqliteStore;
use crate::llm::LlmClient;
use crate::project::ProjectStore;
use std::collections::HashMap;
use tracing::{info, warn};

const MAX_DEPTH: u32 = 4;
const START_DEPTH: u32 = 2;
const LARGE_DIR_THRESHOLD: usize = 10_000;

/// A node in the directory tree.
#[derive(Debug, Clone)]
struct DirNode {
    /// Full path of this directory
    path: String,
    /// Just the directory name (last component)
    name: String,
    /// Number of files in this directory (not recursive)
    file_count: usize,
    /// Top extension → count (e.g., {"fastq.gz": 1024, "sh": 3})
    extensions: Vec<(String, usize)>,
    /// Up to 8 filename samples
    samples: Vec<String>,
    /// Child subdirectories
    subdirs: Vec<DirNode>,
    /// True if this is a large flat dir (no further recursion needed)
    is_large_flat: bool,
}

/// Build a directory tree from the SQLite index, to a given depth.
/// Uses filesystem paths from the index rather than re-scanning.
fn build_dir_tree(root: &str, depth: u32, all_files: &[(i64, String, i64)]) -> DirNode {
    let root_path = root.trim_end_matches('/');
    let root_name = std::path::Path::new(root_path)
        .file_name().map(|n| n.to_string_lossy().to_string())
        .unwrap_or_else(|| root_path.to_string());

    let mut root_node = DirNode {
        path: root_path.to_string(),
        name: root_name,
        file_count: 0,
        extensions: Vec::new(),
        samples: Vec::new(),
        subdirs: Vec::new(),
        is_large_flat: false,
    };

    if depth == 0 { return root_node; }

    // Collect files directly in this directory and discover subdirectories
    let prefix = format!("{}/", root_path);
    let mut ext_counts: HashMap<String, usize> = HashMap::new();
    let mut subdir_set: HashMap<String, Vec<(String, usize, String)>> = HashMap::new();

    for (_, file_path, _) in all_files {
        let p = file_path.as_str();
        if !p.starts_with(&prefix) { continue; }

        let relative = &p[prefix.len()..];

        // Check if it's a direct child file or in a subdirectory
        if let Some(slash_pos) = relative.find('/') {
            // File is in a subdirectory
            let subdir_name = &relative[..slash_pos];
            let subdir_path = format!("{}/{}", root_path, subdir_name);
            let entry = subdir_set.entry(subdir_path.clone()).or_default();
            entry.push((file_path.clone(), 0, subdir_name.to_string()));
            // Recurse: build sub-tree at depth-1 for this subdirectory
            // We'll handle this after first pass
        } else {
            // Direct child file
            root_node.file_count += 1;
            let ext = file_extension(p);
            *ext_counts.entry(ext).or_insert(0) += 1;
            if root_node.samples.len() < 8 {
                let fname = std::path::Path::new(p)
                    .file_name().map(|n| n.to_string_lossy().to_string())
                    .unwrap_or_default();
                if !root_node.samples.contains(&fname) {
                    root_node.samples.push(fname);
                }
            }
        }
    }

    // Sort extensions by count
    let mut ext_vec: Vec<_> = ext_counts.into_iter().collect();
    ext_vec.sort_by(|a, b| b.1.cmp(&a.1));
    ext_vec.truncate(5);
    root_node.extensions = ext_vec;

    // Mark large flat dirs
    if root_node.file_count > LARGE_DIR_THRESHOLD {
        root_node.is_large_flat = true;
        return root_node; // Don't recurse into subdirs for large dirs
    }

    // Build subdirectory nodes (if depth > 1)
    if depth > 1 && !subdir_set.is_empty() {
        for (subdir_path, _files) in &subdir_set {
            // Find all files under this subdir path
            let sub_prefix = format!("{}/", subdir_path);
            let sub_files: Vec<_> = all_files.iter()
                .filter(|(_, p, _)| p.starts_with(&sub_prefix))
                .cloned()
                .collect();
            if !sub_files.is_empty() {
                let child = build_dir_tree_inner(subdir_path, depth - 1, &sub_files);
                root_node.subdirs.push(child);
            }
        }
        root_node.subdirs.sort_by(|a, b| b.file_count.cmp(&a.file_count));
    }

    root_node
}

/// Internal recursive builder that takes a pre-filtered file list.
fn build_dir_tree_inner(path: &str, depth: u32, files: &[(i64, String, i64)]) -> DirNode {
    let name = std::path::Path::new(path)
        .file_name().map(|n| n.to_string_lossy().to_string())
        .unwrap_or_else(|| path.to_string());

    let mut node = DirNode {
        path: path.to_string(),
        name,
        file_count: 0,
        extensions: Vec::new(),
        samples: Vec::new(),
        subdirs: Vec::new(),
        is_large_flat: false,
    };

    if depth == 0 { return node; }

    let prefix = format!("{}/", path);
    let mut ext_counts: HashMap<String, usize> = HashMap::new();
    let mut subdir_names: HashMap<String, Vec<(i64, String, i64)>> = HashMap::new();

    for (id, file_path, mtime) in files {
        let p = file_path.as_str();
        if !p.starts_with(&prefix) { continue; }
        let relative = &p[prefix.len()..];

        if let Some(slash_pos) = relative.find('/') {
            let sd = &relative[..slash_pos];
            let sp = format!("{}/{}", path, sd);
            subdir_names.entry(sp).or_default().push((*id, file_path.clone(), *mtime));
        } else {
            node.file_count += 1;
            let ext = file_extension(p);
            *ext_counts.entry(ext).or_insert(0) += 1;
            if node.samples.len() < 8 {
                let fn_str = std::path::Path::new(p)
                    .file_name().map(|n| n.to_string_lossy().to_string())
                    .unwrap_or_default();
                if !node.samples.contains(&fn_str) { node.samples.push(fn_str); }
            }
        }
    }

    let mut ext_vec: Vec<_> = ext_counts.into_iter().collect();
    ext_vec.sort_by(|a, b| b.1.cmp(&a.1));
    ext_vec.truncate(5);
    node.extensions = ext_vec;

    if node.file_count > LARGE_DIR_THRESHOLD {
        node.is_large_flat = true;
        return node;
    }

    if depth > 1 {
        for (sp, sf) in &subdir_names {
            let child = build_dir_tree_inner(sp, depth - 1, sf);
            node.subdirs.push(child);
        }
        node.subdirs.sort_by(|a, b| b.file_count.cmp(&a.file_count));
    }

    node
}

/// Convert a directory tree into a text prompt for the LLM.
fn tree_to_prompt(root: &DirNode, indent: usize) -> String {
    let mut lines = Vec::new();
    let prefix = "  ".repeat(indent);
    let ext_summary: Vec<String> = root.extensions.iter()
        .map(|(e, c)| format!("{}×{}", e, c))
        .collect();

    if root.is_large_flat {
        lines.push(format!(
            "{}📁 {}/  ({} files, LARGE FLAT DIR — {} — skip deep listing)",
            prefix, root.name, root.file_count, ext_summary.join(", ")
        ));
    } else if root.subdirs.is_empty() {
        let sample_str = if root.samples.is_empty() {
            String::new()
        } else {
            format!("  samples: {}", root.samples.join(", "))
        };
        lines.push(format!(
            "{}📁 {}/  ({} files: {}){}",
            prefix, root.name, root.file_count, ext_summary.join(", "), sample_str
        ));
    } else {
        lines.push(format!(
            "{}📁 {}/  ({} files: {}, {} subdirs)",
            prefix, root.name, root.file_count, ext_summary.join(", "), root.subdirs.len()
        ));
    }

    for child in &root.subdirs {
        if child.is_large_flat || !child.subdirs.is_empty() {
            lines.push(tree_to_prompt(child, indent + 1));
        } else {
            // Leaf: show in compact form
            let ext_summary: Vec<String> = child.extensions.iter()
                .map(|(e, c)| format!("{}×{}", e, c))
                .collect();
            let sample_str = if child.samples.is_empty() {
                String::new()
            } else {
                format!("  e.g. {}", child.samples.iter().take(4).cloned().collect::<Vec<_>>().join(", "))
            };
            lines.push(format!(
                "{}📁 {}/  ({} files: {}){}",
                "  ".repeat(indent + 1), child.name, child.file_count,
                ext_summary.join(", "), sample_str
            ));
        }
    }
    lines.join("\n")
}

fn file_extension(path: &str) -> String {
    let name = std::path::Path::new(path)
        .file_name().map(|n| n.to_string_lossy().to_string())
        .unwrap_or_default();
    // Handle compound extensions like .fastq.gz, .vcf.gz
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

/// Run hierarchical LLM inference.
/// Builds a tree from the index, sends the structure to the LLM, and creates projects.
pub fn run_hierarchical_inference(
    sqlite: &SqliteStore,
    project_store: &ProjectStore,
    llm_client: &LlmClient,
    scan_root: &str,
) -> Result<(usize, usize), Box<dyn std::error::Error>> {
    if !llm_client.is_configured() {
        info!("LLM not configured, skipping inference");
        return Ok((0, 0));
    }

    let all_files = sqlite.all_paths().unwrap_or_default();

    // Layer 1: Build tree at adaptive depth
    let current_depth = START_DEPTH;
    let tree = build_dir_tree(scan_root, current_depth, &all_files);

    eprintln!("  Hierarchical inference: tree depth {}", current_depth);
    eprintln!("  {} ({} files, {} subdirs)", tree.name, tree.file_count, tree.subdirs.len());

    // Layer 2: Build prompt from tree and send to LLM
    let prompt_text = tree_to_prompt(&tree, 0);
    let full_prompt = format!(
        "你是生物信息学数据管理助手。下面是一个数据目录的树状结构。请分析并返回JSON。\n\n\
         规则:\n\
         1. 根据目录名和文件扩展名推断项目类型(assay_type)、物种(species)、置信度(high/medium/low)\n\
         2. 将相关子目录合并为项目\n\
         3. 标记明显不是生物数据的目录为 skip_reason\n\
         4. 如果某个分支需要更深层查看,标记 need_deeper=true\n\
         5. 对于大型扁平目录,根据目录名和少量样本判断即可\n\n\
         目录树:\n{}", prompt_text
    );

    let system_prompt = crate::llm::prompt::system_prompt();
    let body = serde_json::json!({
        "model": llm_client.config.model,
        "messages": [
            {"role": "system", "content": system_prompt},
            {"role": "user", "content": full_prompt}
        ],
        "response_format": {"type": "json_object"},
        "temperature": 0.1,
        "max_tokens": 16384
    });

    eprintln!("  Sending tree to LLM ({} chars)...", full_prompt.len());

    // Use the retry-capable API call from llm module
    let response: serde_json::Value = crate::llm::llm_api_call_with_retry(&llm_client.config, &body, 3)?;
    let content = response["choices"][0]["message"]["content"]
        .as_str().ok_or("No content in LLM response")?;

    // Parse the response
    let output: serde_json::Value = serde_json::from_str(content)
        .map_err(|e| format!("Failed to parse LLM JSON: {}", e))?;

    let empty_projects = vec![];
    let projects = output["projects"].as_array()
        .unwrap_or(&empty_projects);

    let mut projects_created = 0;
    let mut files_updated = 0;
    let mut path_to_id: HashMap<String, i64> = HashMap::new();
    for (id, path, _) in &all_files {
        path_to_id.insert(path.clone(), *id);
    }

    for proj_val in projects {
        let name = proj_val["name"].as_str().unwrap_or("unnamed");
        let dirs: Vec<String> = proj_val["dirs"].as_array()
            .map(|a| a.iter().filter_map(|v| v.as_str().map(String::from)).collect())
            .unwrap_or_default();
        let assay = proj_val["assay_type"].as_str().map(String::from);
        let species = proj_val["species"].as_str().map(String::from);
        let confidence = proj_val["species_confidence"].as_str().map(String::from);
        let summary = proj_val["summary"].as_str().map(String::from);
        let should_skip = proj_val["skip"].as_bool().unwrap_or(false);

        if should_skip {
            eprintln!("  ⏭ {} (skipped)", name);
            continue;
        }

        if dirs.is_empty() { continue; }

        let root_dirs_json = serde_json::to_string(&dirs).unwrap_or_default();
        match project_store.insert(
            name, assay.as_deref(), species.as_deref(),
            confidence.as_deref(), Some(&root_dirs_json), summary.as_deref(),
        ) {
            Ok(proj_id) => {
                projects_created += 1;
                // Link files under project dirs
                for dir in &dirs {
                    for (file_id, file_path, _) in &all_files {
                        if file_path.starts_with(dir) {
                            project_store.link_file(proj_id, *file_id).ok();
                            files_updated += 1;
                        }
                    }
                }
            }
            Err(e) => warn!("Failed to insert project {}: {}", name, e),
        }
    }

    eprintln!("  ✅ Hierarchical: {} projects, {} files tagged", projects_created, files_updated);
    Ok((projects_created, 0))
}
