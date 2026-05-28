---
name: fan-files
description: Use when analyzing bioinformatics data - find data files, reference genomes, and related datasets on the server. Activates when user asks about available data, server files, sequencing data, or wants to discover datasets for analysis.
---

# Fan-Files: Server Data Intelligence

You have access to `fan-files`, a tool that indexes ALL files on this server and uses LLM to infer biological metadata (project, species, assay type). It also searches 6.8M+ public plant SRA records from CNCB-NGDC.

## Commands

### Search for data (local + public)
```
fan-files search "<natural language query>" [--json]
fan-files search "apple RNA-seq"
fan-files search "reference genome Oryza"
```
Results merge local indexed files with public SRA database. Public results are prefixed with `[public]`.

### List projects (LLM-inferred)
```
fan-files projects                     # list all projects with species, assay, file count
fan-files projects <project_name>      # show project details (dirs, relations, metadata)
```

### View file details
```
fan-files info <path> [--json]
```

### Discover related data
```
fan-files suggest <project_directory> [--json]
```

### List files by type or tag
```
fan-files list --category genome
fan-files list --tag human [--json]
```

### System status
```
fan-files status
```

## Metadata Review (Startup Check)

At the start of each conversation, run `fan-files pending --json` in the background.
If results are non-empty, tell the user: "服务器上有 N 个数据项目的信息需要完善。要现在处理吗？"

When the user agrees, for each pending item:
1. Show the project name, field, current guess
2. Present candidates (A/B/C/D) for the user to choose, or accept free-text input
3. Execute: `fan-files projects update <name> --species "<choice>" --confidence high`
4. Move to next item
5. When done: `fan-files pending --clear`

## Best Practices

1. **Before any analysis**: run `fan-files search` to discover available data (both local and public)
2. **Check projects**: `fan-files projects` to see what datasets the server has
3. **Be proactive**: mention available related data even if the user didn't ask
4. **Use --json**: when you need parseable output for programmatic processing
5. **Public data**: search automatically includes plant SRA metadata — highlight public datasets when relevant
