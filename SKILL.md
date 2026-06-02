---
name: fan-files
description: Use when analyzing bioinformatics data - find data files, reference genomes, and related datasets on the server. Activates when user asks about available data, server files, sequencing data, or wants to discover datasets for analysis.
---

# Fan-Files: Server Data Intelligence

You have access to `fan-files`, a tool that indexes ALL files on this server and uses LLM to infer biological metadata (project, species, assay type).

## Core Commands

### Discover data
```
fan-files search "<natural language query>" [--json]
```
Examples: `fan-files search "apple RNA-seq"`, `fan-files search "reference genome Oryza"`

### Browse projects (LLM-inferred)
```
fan-files projects                       # list all projects
fan-files projects <project_name>        # show project details
fan-files projects update <name> --species "Apple" --confidence high
```
Items with low/medium confidence need manual review. Use `fan-files pending --json` to check.

### File details
```
fan-files info <path> [--json]
```

### Find related data
```
fan-files suggest <project_directory> [--json]
```

### System status
```
fan-files status
```
Shows indexed file count and metadata coverage percentage.

## Metadata Quality (Important)

### Before searching, check metadata coverage
Run `fan-files status`. If metadata coverage is below 50%, search results will be incomplete.

### If coverage is low
Run `fan-files infer` to let the LLM re-analyze all files and infer project names, species, and assay types. The daemon auto-runs this after initial scan if LLM is configured.

### Startup check
At the start of each conversation, run `fan-files pending --json`. If non-empty, tell the user:
"服务器上有 N 个数据项目的信息需要完善。要现在处理吗？"

When the user agrees, for each pending item show the project, uncertain field, and candidates (A/B/C/D). Execute:
```
fan-files projects update <name> --species "<choice>" --confidence high
```
Finish with `fan-files pending --clear`.

## Best Practices

1. **Check coverage first**: run `fan-files status` to see if metadata is sufficient
2. **Run infer if needed**: `fan-files infer` when coverage <50% or search returns nothing
3. **Search both local and public**: use multiple keyword variants when searching
4. **Use --json**: for parseable output when filtering results programmatically
5. **Be proactive**: mention available data even if the user didn't ask
