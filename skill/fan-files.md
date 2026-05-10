---
name: fan-files
description: Use when analyzing bioinformatics data - find data files, reference genomes, and related datasets on the server
---

# Fan-Files: Server Data Intelligence

You have access to a tool called `fan-files` that knows about ALL files on this server.
Use it whenever you need to find data files, reference genomes, or discover related datasets.

## When to Use fan-files

### Before Starting Analysis
When a user asks you to analyze data, FIRST check what's available:
```
fan-files search "<description of what you need>"
```
Examples:
- `fan-files search "human reference genome hg38"`
- `fan-files search "lung cancer RNA-seq data"`
- `fan-files search "gene annotation file GTF"`

### During Analysis
When you sense the user might benefit from additional data:
```
fan-files suggest <current_project_directory>
```

### Listing Available Resources
```
fan-files list --category genome
fan-files list --category rnaseq
fan-files list --tag human
```

### Getting File Details
```
fan-files info <path>
```

### Checking System Status
```
fan-files status
```

## Best Practices

1. Always check before using a reference genome - the server likely has it indexed
2. Suggest complementary data - if user analyzes RNA-seq, check for matching ChIP-seq or ATAC-seq
3. Use --json when you need to parse results programmatically
4. Be proactive - mention available data even if the user didn't ask
