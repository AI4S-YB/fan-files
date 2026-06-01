---
name: fan-files
description: Use when analyzing bioinformatics data - find data files, reference genomes, and related datasets on the server. Activates when user asks about available data, server files, sequencing data, or wants to discover datasets for analysis.
---

# Fan-Files: Server Data Intelligence

You have access to `fan-files` for local file search and the public Plant SRA API on `47.95.117.10:8080` for 6.8M+ public plant records.

## Data Search (Local + Public + LLM-driven)

When searching for data, use a **3-step strategy**:

### Step 1: Generate search variants with LLM
Based on user's analysis goal, generate 3-5 search keyword variants. For example:
- User: "找苹果的RNA-seq数据"
- Variants: "Malus domestica RNA-Seq", "apple transcriptome", "Malus RNA-Seq", "apple expression"

### Step 2: Search both sources
```
# Local search
fan-files search "<keyword>" --json

# Public SRA API (Deduplicated by BioProject)
curl -s "http://47.95.117.10:8080/search?q=<keyword>&dedup=true&limit=10"
```

### Step 3: Merge and present
- Group results by organism name and BioProject
- Remove duplicate runs from the same project
- Clearly label source: `[local]` / `[public]`
- Present as: organism | project_title | source | accessions_count

## Commands

### Local search
```
fan-files search "<query>" [--json]
```

### List projects
```
fan-files projects
fan-files projects <name>
```

### File details
```
fan-files info <path> [--json]
```

### System status
```
fan-files status
```

## Best Practices

1. **Generate search variants**: never search with user's raw input only — expand to Latin names, synonyms
2. **Deduplicate**: public results are SRA runs — aggregate by BioProject
3. **Both sources always**: local + public for every search
4. **Present findings clearly**: organism → project title → how many runs → source
