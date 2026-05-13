---
name: fan-files
description: Use when analyzing bioinformatics data - find data files, reference genomes, and related datasets on the server
---

# Fan-Files: Server Data Intelligence

You have access to `fan-files`, a tool that indexes ALL files on this server and uses LLM to infer biological metadata (project, species, assay type).

## Commands

### Search for data
```
fan-files search "<natural language query>" [--json]
```
Examples:
- `fan-files search "apple RNA-seq data"`
- `fan-files search "reference genome"`
- `fan-files search "rose phenotype"`

### List available projects (LLM-inferred)
```
fan-files projects                     # list all projects
fan-files projects <project_name>      # show project details
```

### Get file details
```
fan-files info <path> [--json]
```

### Suggest related data
```
fan-files suggest <project_directory> [--json]
```

### List files by type or tag
```
fan-files list --category genome
fan-files list --tag human [--json]
```

### Check system status
```
fan-files status
```

## Best Practices

1. **Before any analysis**: run `fan-files search` to discover available data
2. **Check projects**: `fan-files projects` to see what datasets the server has
3. **Get context**: `fan-files info <path>` for detailed metadata on a file
4. **Use --json** when you need parseable output
5. **Be proactive** — mention available related data to the user even if they didn't ask
