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

const PHASE1_DEPTH: u32 = 3;
const PHASE2_START_DEPTH: u32 = 4;
const MAX_RECURSE_DEPTH: u32 = 8;
const LARGE_DIR_THRESHOLD: usize = 10_000;
const COMPRESS_THRESHOLD: usize = 200;
const SMART_SAMPLE_COUNT: usize = 5;

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

/// Compress a large directory node by reducing sample count and using pattern descriptions.
/// For dirs with >COMPRESS_THRESHOLD files, samples are reduced to representative entries
/// grouped by extension, with filename patterns detected.
fn compress_node(node: &mut DirNode) {
    if node.file_count <= COMPRESS_THRESHOLD { return; }

    // Group samples by extension, pick up to SMART_SAMPLE_COUNT per group
    let mut by_ext: HashMap<String, Vec<String>> = HashMap::new();
    for sample in &node.samples {
        let ext = file_extension(sample);
        by_ext.entry(ext).or_default().push(sample.clone());
    }

    let mut compressed = Vec::new();
    for (ext, names) in &by_ext {
        let count = node.extensions.iter()
            .find(|(e, _)| e == ext)
            .map(|(_, c)| *c)
            .unwrap_or(names.len());

        // Pick diverse samples (prefer different filename prefixes)
        let samples = smart_sample(names, SMART_SAMPLE_COUNT);

        compressed.push(format!("{}×{} [{}]", ext, count, samples.join(", ")));
    }

    // Replace samples with compressed pattern description
    node.samples = vec![compressed.join("; ")];
    node.file_count = 1; // Signal to LLM: this is a compressed summary
}

/// Pick up to N diverse samples, preferring different semantic filename prefixes.
/// Normalizes common bioinformatics accession prefixes (SRR/ERR/DRR → "SRA_run" etc.)
fn smart_sample(names: &[String], n: usize) -> Vec<String> {
    if names.len() <= n { return names.to_vec(); }
    let mut result: Vec<String> = Vec::new();
    let mut seen_prefixes: Vec<String> = Vec::new();

    for name in names {
        if result.len() >= n { break; }
        let cat = normalize_prefix(name);

        // Prioritize names with different semantic categories
        if !seen_prefixes.contains(&cat) || result.len() < n {
            result.push(name.clone());
            if !seen_prefixes.contains(&cat) {
                seen_prefixes.push(cat);
            }
        }
    }
    result
}

/// Normalize filename prefix into a semantic category.
/// Merges SRA accession numbers (SRR/ERR/DRR → "SRA_run"),
/// GEO identifiers (GSM/E → category), BioProject codes, etc.
fn normalize_prefix(name: &str) -> String {
    // Strip extension(s) for prefix matching:
    // "SRR6246475_1.fastq.gz" → split('.') → ["SRR6246475_1", "fastq", "gz"] → take first
    let base = name.split('.').next().unwrap_or(name).to_uppercase();

    // SRA run accessions (NCBI / EBI / DDBJ)
    if base.starts_with("SRR") || base.starts_with("ERR") || base.starts_with("DRR") {
        return "SRA_run".to_string();
    }
    // SRA experiment
    if base.starts_with("SRX") || base.starts_with("ERX") || base.starts_with("DRX") {
        return "SRA_experiment".to_string();
    }
    // GEO
    if base.starts_with("GSM") { return "GEO_sample".to_string(); }
    if base.starts_with("GSE") { return "GEO_series".to_string(); }
    if base.starts_with("GPL") { return "GEO_platform".to_string(); }
    // BioProject (NCBI/EBI/DDBJ)
    if base.starts_with("PRJNA") || base.starts_with("PRJEB") || base.starts_with("PRJDB") {
        return "BioProject".to_string();
    }
    // ENA sample/study
    if base.starts_with("ERS") || base.starts_with("SAME") || base.starts_with("SAMN") || base.starts_with("SAMEA") {
        return "BioSample".to_string();
    }
    // Generic: characters before first digit
    let prefix: String = base.chars()
        .take_while(|c| !c.is_ascii_digit())
        .collect();
    let trimmed = prefix.trim_end_matches('_').to_string();
    if trimmed.is_empty() { "unknown".to_string() } else { trimmed }
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

    // Mark large flat dirs and compress
    if root_node.file_count > LARGE_DIR_THRESHOLD {
        root_node.is_large_flat = true;
        compress_node(&mut root_node);
        return root_node;
    }
    if root_node.file_count > COMPRESS_THRESHOLD {
        compress_node(&mut root_node);
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
        root_node.subdirs.truncate(15); // keep top 15 by file count
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

    // Check if compressed (samples are pattern strings, not filenames)
    let is_compressed = root.samples.len() == 1 && root.file_count == 1
        && root.samples[0].contains('×');

    if root.is_large_flat {
        let compressed_info = if is_compressed {
            format!(" — {}", root.samples[0])
        } else { String::new() };
        lines.push(format!(
            "{}📁 {}  (LARGE FLAT DIR{} — skip deep listing)",
            prefix, root.name, compressed_info
        ));
    } else if root.subdirs.is_empty() {
        if is_compressed {
            lines.push(format!(
                "{}📁 {}  ({})",
                prefix, root.name, root.samples[0]
            ));
        } else {
            let sample_str = if root.samples.is_empty() {
                String::new()
            } else {
                format!("  e.g. {}", root.samples.join(", "))
            };
            lines.push(format!(
                "{}📁 {}  ({} files: {}){}",
                prefix, root.name, root.file_count, ext_summary.join(", "), sample_str
            ));
        }
    } else {
        let compressed_info = if is_compressed {
            format!(" [{}]", root.samples[0])
        } else { String::new() };
        lines.push(format!(
            "{}📁 {}/  ({} files: {}, {} subdirs){}",
            prefix, root.name, root.file_count, ext_summary.join(", "), root.subdirs.len(), compressed_info
        ));
    }

    for child in &root.subdirs {
        let child_is_compressed = child.samples.len() == 1 && child.file_count == 1
            && child.samples[0].contains('×');

        if child.is_large_flat || !child.subdirs.is_empty() || child_is_compressed {
            lines.push(tree_to_prompt(child, indent + 1));
        } else {
            let ext_summary: Vec<String> = child.extensions.iter()
                .map(|(e, c)| format!("{}×{}", e, c))
                .collect();
            let sample_str = if child.samples.is_empty() {
                String::new()
            } else {
                format!("  e.g. {}", child.samples.join(", "))
            };
            lines.push(format!(
                "{}📁 {}  ({} files: {}){}",
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

/// Build a tree recursively until no more hidden subdirs, up to MAX_RECURSE_DEPTH.
/// Each pass builds 2 levels at a time, then checks if any branch still has subdirs.
fn build_recursive_deep(path: &str, files: &[(i64, String, i64)]) -> DirNode {
    let mut node = build_dir_tree_inner(path, PHASE2_START_DEPTH, files);
    expand_hidden_branches(&mut node, files, PHASE2_START_DEPTH);
    node
}

fn expand_hidden_branches(node: &mut DirNode, all_files: &[(i64, String, i64)], current_depth: u32) {
    if current_depth >= MAX_RECURSE_DEPTH { return; }
    for child in &mut node.subdirs {
        if child.subdirs.len() >= 2 && !child.is_large_flat {
            // Has hidden depth — rebuild this branch 2 levels deeper
            let sub_files: Vec<_> = all_files.iter()
                .filter(|(_, p, _)| p.starts_with(&format!("{}/", child.path)))
                .cloned()
                .collect();
            if sub_files.len() < 50 { continue; }
            let mut deep = build_dir_tree_inner(&child.path, current_depth + 2, &sub_files);
            expand_hidden_branches(&mut deep, all_files, current_depth + 2);
            *child = deep;
        }
    }
}

/// Recursively compress large subdirectories in a tree.
fn compress_large_subdirs(node: &mut DirNode) {
    if node.file_count > COMPRESS_THRESHOLD {
        compress_node(node);
    }
    for child in &mut node.subdirs {
        compress_large_subdirs(child);
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

    // Phase 1: build tree at depth 3, let LLM classify
    let tree = build_dir_tree(scan_root, PHASE1_DEPTH, &all_files);
    eprintln!("  Phase 1: depth {} tree ({} subdirs)", PHASE1_DEPTH, tree.subdirs.len());

    // Layer 2: Build prompt from tree and send to LLM
    let prompt_text = tree_to_prompt(&tree, 0);
    let full_prompt = format!(
        "你是生物信息学数据管理助手。下面是一个数据目录的树状结构。\n\n\
         任务: 推断每个目录属于什么物种、什么实验类型。\n\n\
         规则:\n\
         1. 从目录名推断物种(如Oryza_sativa=水稻,Triticum_aestivum=小麦,Zea_mays=玉米,Glycine_max=大豆,Fungi=真菌)\n\
         2. 从文件扩展名推断实验类型(fastq.gz=RNA-seq或WGS, vcf=variant_calling, fa/fna=genome, gff3/gtf=annotation)\n\
         3. 同一物种的多个目录合并为一个项目(如Oryza_sativa和Oryza_sativa_multi应合并)\n\
         4. 明显不是生信数据的目录标记skip_reason(如code/scripts/testdata/.pnpm-store)\n\
         5. 每个项目输出: name, dirs(数组), assay_type, species, species_confidence, summary\n\n\
         目录树:\n{}", prompt_text
    );

    let system_prompt = "你是生物信息学数据管理助手。用户提供一个扫描根目录的树状结构，\
        包含每个子目录的文件数量、扩展名分布和代表性文件名。\n\n\
        你的任务:\n\
        1. 识别每个顶层目录对应的生物学项目(物种+实验类型)\n\
        2. 将明显不是生信数据的目录标记skip:true(如code, venv, testdata, .pnpm-store, scripts)\n\
        3. 同一物种/项目的多个目录合并(如Oryza_sativa和Oryza_sativa_multi)\n\
        4. 对无法确定物种的目录，从目录名+文件名推断\n\n\
        输出JSON格式:\n\
        {\"projects\":[{\"name\":\"项目名\",\"dirs\":[\"子目录路径\"],\"assay_type\":\"RNA-seq|WGS|genome_annotation|variant_calling|epigenomics|transcriptomics|phenomics|germplasm\",\
        \"species\":\"物种拉丁名\",\"species_confidence\":\"high|medium|low\",\"summary\":\"描述\",\"skip\":false}]}";
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

    eprintln!("  Phase 1: sending depth-{} tree to LLM ({} chars)...", PHASE1_DEPTH, full_prompt.len());

    // Phase 1: get top-level classification
    let response: serde_json::Value = crate::llm::llm_api_call_with_retry(&llm_client.config, &body, 3)?;
    let content = response["choices"][0]["message"]["content"]
        .as_str().ok_or("No content in LLM response")?;

    let output: serde_json::Value = serde_json::from_str(content)
        .map_err(|e| format!("Failed to parse LLM JSON: {}", e))?;

    let empty_projects = vec![];
    let mut projects = output["projects"].as_array()
        .map(|a| a.clone())
        .unwrap_or(empty_projects.clone());

    // Phase 2: adaptive recursion — only for branches LLM did NOT skip
    // Collect skipped dirs from Phase 1 LLM response
    let skipped_dirs: std::collections::HashSet<String> = projects.iter()
        .filter(|p| p["skip"].as_bool().unwrap_or(false))
        .filter_map(|p| p["dirs"].as_array())
        .flat_map(|a| a.iter())
        .filter_map(|v| v.as_str().map(String::from))
        .collect();

    let mut phase2_count = 0;
    let mut deep_projects: Vec<serde_json::Value> = Vec::new();

    // Phase 2 candidates: subdirs with hidden depth AND not skipped by LLM
    let phase2_candidates: Vec<_> = tree.subdirs.iter()
        .filter(|c| !c.subdirs.is_empty() && !c.is_large_flat)
        .filter(|c| c.subdirs.len() >= 2)
        .filter(|c| !skipped_dirs.contains(&c.name)
                  && !skipped_dirs.iter().any(|d| d.ends_with(&format!("/{}", c.name))))
        .collect();

    if !phase2_candidates.is_empty() {
        eprintln!("  Phase 2: deep-diving {} branches (skipped {} by LLM)...",
            phase2_candidates.len(), skipped_dirs.len());
    }

    for child in &phase2_candidates {
        let child = *child; // deref from &&DirNode

        let subdir_files: Vec<_> = all_files.iter()
            .filter(|(_, p, _)| p.starts_with(&format!("{}/", child.path)))
            .cloned()
            .collect();
        if subdir_files.len() < 50 { continue; }

        // Adaptive depth: build recursively until flat, compress large dirs
        let mut deep_node = build_recursive_deep(&child.path, &subdir_files);
        compress_large_subdirs(&mut deep_node);
        let deep_prompt = tree_to_prompt(&deep_node, 0);
        if deep_prompt.len() < 200 { continue; }

        // If still too large after compression, split into top-level sub-branches
        let final_prompts: Vec<(String, String)> = if deep_prompt.len() > 100_000 {
            // Split: each major subdir becomes its own mini-prompt
            deep_node.subdirs.iter()
                .filter(|s| s.file_count > 0)
                .map(|s| {
                    let mut mini = deep_node.clone();
                    mini.subdirs = vec![s.clone()];
                    mini.name = format!("{}/{}", child.name, s.name);
                    (mini.name.clone(), tree_to_prompt(&mini, 0))
                })
                .filter(|(_, p)| p.len() >= 200 && p.len() < 100_000)
                .collect()
        } else {
            vec![(child.name.clone(), deep_prompt)]
        };

        for (sub_name, prompt) in &final_prompts {
            let deep_full = format!(
                "你是生物信息学数据管理助手。下面是一个子目录的深层树状结构(深度5)。\n\
                 大目录已被压缩为'扩展名×数量 [样本1, 样本2...]'格式。\n\
                 请推断其中包含的数据项目(物种+实验类型)，同一基因组的不同分析版本应合并。\n\
                 明显不是生物数据的标记skip:true。输出JSON projects数组。\n\n{}",
                prompt
            );

            eprintln!("  Phase 2: {} ({} chars)...", sub_name, deep_full.len());
            match crate::llm::llm_api_call_with_retry(&llm_client.config, &serde_json::json!({
                "model": llm_client.config.model,
                "messages": [
                    {"role": "system", "content": system_prompt},
                    {"role": "user", "content": deep_full}
                ],
                "response_format": {"type": "json_object"},
                "temperature": 0.1,
                "max_tokens": 16384
            }), 3) {
                Ok(deep_resp) => {
                    let deep_content = deep_resp["choices"][0]["message"]["content"].as_str().unwrap_or("");
                    if let Ok(deep_output) = serde_json::from_str::<serde_json::Value>(deep_content) {
                        if let Some(deep_arr) = deep_output["projects"].as_array() {
                            deep_projects.extend(deep_arr.clone());
                            phase2_count += deep_arr.len();
                        }
                    }
                }
                Err(e) => warn!("  Phase 2 failed for {}: {}", sub_name, e),
            }
        }
    }

    // Merge: remove Phase 1 projects covered by Phase 2 (dedup by dirs, not names)
    if !deep_projects.is_empty() {
        // Collect all dirs from Phase 2 projects
        let phase2_dirs: std::collections::HashSet<String> = deep_projects.iter()
            .filter(|p| !p["skip"].as_bool().unwrap_or(false))
            .filter_map(|p| p["dirs"].as_array())
            .flat_map(|a| a.iter())
            .filter_map(|v| v.as_str().map(String::from))
            .collect();

        // Remove Phase 1 projects whose dirs are all covered by Phase 2
        projects.retain(|p| {
            let skip = p["skip"].as_bool().unwrap_or(false);
            if skip { return true; } // keep skipped entries
            let dirs: Vec<&str> = p["dirs"].as_array()
                .map(|a| a.iter().filter_map(|v| v.as_str()).collect())
                .unwrap_or_default();
            // Keep only if at least one dir is NOT in Phase 2 (not fully covered)
            dirs.iter().any(|d| !phase2_dirs.contains(*d))
        });
        projects.extend(deep_projects);
        eprintln!("  Merged: {} phase-1 + {} phase-2 projects (deduped)",
            projects.len() - phase2_count, phase2_count);
    }

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
                // Link files: try dir as-is, also try as sub-path of scan_root
                for dir in &dirs {
                    let candidates = vec![
                        dir.clone(),
                        format!("{}/{}", scan_root.trim_end_matches('/'), dir.trim_start_matches('/')),
                    ];
                    for (file_id, file_path, _) in &all_files {
                        for candidate in &candidates {
                            if file_path.starts_with(candidate) {
                                project_store.link_file(proj_id, *file_id).ok();
                                files_updated += 1;
                                break;
                            }
                        }
                    }
                }
                eprintln!("  📦 {} ({} dirs)", name, dirs.len());
            }
            Err(e) => warn!("Failed to insert project {}: {}", name, e),
        }
    }

    eprintln!("  ✅ Hierarchical: {} projects, {} files tagged", projects_created, files_updated);
    Ok((projects_created, 0))
}
