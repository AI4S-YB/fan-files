/// System prompt for the LLM
pub fn system_prompt() -> &'static str {
    "你是一个生物信息学数据管理助手。用户会给你一个服务器目录的扫描结果，\
     包含目录结构和代表性文件列表。请分析这些目录，返回结构化的 JSON。\n\n\
     你需要：\n\
     1. 将目录合并成\"数据项目\"——同一个生物学项目的文件即使分散在多个子目录，也应该归为一个项目\n\
     2. 推断项目的实验类型（assay_type）：RNA-seq, ChIP-seq, WGS, WGBS, ATAC-seq, \
        genome_annotation, variant_calling, epigenomics, transcriptomics, phenomics, germplasm 等\n\
     3. 推断物种信息（species）和置信度（species_confidence: high/medium/low）\n\
     4. 判断不同项目之间是否有关联（同一物种、互补实验类型等）\n\
     5. 每个项目写一句简短描述（summary）"
}

use serde::Deserialize;

#[derive(Debug, Deserialize)]
pub struct LlmOutput {
    pub projects: Vec<LlmProject>,
    #[serde(default)]
    pub relations: Vec<LlmRelation>,
}

#[derive(Debug, Deserialize)]
pub struct LlmProject {
    pub name: String,
    pub dirs: Vec<String>,
    pub assay_type: Option<String>,
    pub species: Option<String>,
    #[serde(default)]
    pub species_confidence: Option<String>,
    pub summary: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct LlmRelation {
    pub project_a: String,
    pub project_b: String,
    pub relation: String,
    pub score: f64,
}

/// Filter for key bioinformatics file extensions (shown first in summary)
fn is_key_file(name: &str) -> bool {
    let lower = name.to_lowercase();
    lower.ends_with(".fastq.gz") || lower.ends_with(".fq.gz") || lower.ends_with(".fastq") ||
    lower.ends_with(".fasta") || lower.ends_with(".fa.gz") || lower.ends_with(".fa") ||
    lower.ends_with(".bam") || lower.ends_with(".vcf.gz") || lower.ends_with(".vcf") ||
    lower.ends_with(".gff3") || lower.ends_with(".gtf") || lower.ends_with(".bed") ||
    lower.ends_with(".h5") || lower.ends_with(".hdf5")
}

/// Generate a directory summary text from the index for LLM consumption
pub fn build_directory_summary(
    root: &str,
    dirs: &[(String, usize, Vec<String>)],
) -> String {
    let mut lines = vec![
        format!("## 扫描结果\n\n根目录: {}\n", root),
    ];

    for (path, count, samples) in dirs {
        let key_files: Vec<&str> = samples.iter()
            .filter(|n| is_key_file(n))
            .map(|s| s.as_str())
            .collect();
        let other: Vec<&str> = samples.iter()
            .filter(|n| !is_key_file(n))
            .take(5)
            .map(|s| s.as_str())
            .collect();
        let all_samples: Vec<&str> = key_files.iter().chain(other.iter()).copied().collect();
        let display = all_samples.join(", ");
        let display = if display.len() > 120 {
            format!("{}...", &display[..117])
        } else {
            display
        };

        lines.push(format!("{}  ({} files)", path, count));
        lines.push(format!(
            "  代表性文件: {}",
            if display.is_empty() { "(无)" } else { &display }
        ));
    }

    lines.join("\n")
}

/// Parse LLM JSON response into structured data
pub fn parse_llm_response(json: &str) -> Result<LlmOutput, serde_json::Error> {
    serde_json::from_str(json)
}
