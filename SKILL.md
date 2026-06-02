---
name: fan-files
description: Use when the user needs to install, configure, search, or manage bioinformatics data on the server. Activates for data discovery, file search, project management, metadata review, installing fan-files, setting up scanning, upgrading, or uninstalling.
---

# Fan-Files: Server Data Intelligence

You have access to `fan-files`, a tool that indexes ALL files on this server and uses LLM to infer biological metadata (project, species, assay type). You can perform ALL fan-files operations on behalf of the user — from installation to daily data search.

## Administrative Commands

### Installation
User can ask you to install fan-files from scratch. Always install the Claude Code skill at the same time:
```bash
curl -fsSL https://raw.githubusercontent.com/AI4S-YB/fan-files/main/install.sh | bash
cp /path/to/fan-files/SKILL.md ~/.claude/skills/fan-files.md
```

### First-time setup
Run the interactive wizard:
```bash
fan-files init
```
It guides through: choose scan directories → select LLM provider → enter API key → start scanning. After init, the user can start using search immediately.

### Start background scanning
```bash
fan-files daemon
```
Scans configured directories, watches for changes, auto-runs LLM inference when 10+ new files appear.

### Upgrade to latest
```bash
fan-files update
```

### Uninstall
```bash
fan-files uninstall
```
Option 1: remove program + skill, keep data. Option 2: full wipe.

## Daily Commands

### Discover data
```
fan-files search "<natural language query>" [--json]
```

### Browse projects (LLM-inferred)
```
fan-files projects                       # list all
fan-files projects <project_name>        # details
fan-files projects update <name> --species "Apple" --confidence high
```

### File details
```
fan-files info <path> [--json]
```

### Find related data
```
fan-files suggest <project_directory> [--json]
```

### Precise tag-based listing
```
fan-files list --tag paired-end
fan-files list --category genome [--json]
```

### System status
```
fan-files status
```

### Re-run metadata inference
```
fan-files infer
```
Use when `status` shows low metadata coverage or search returns nothing.

## Metadata Quality Check (Startup + Periodic)

At the start of each conversation, run:
```bash
fan-files pending --json
```
If non-empty, tell the user: "服务器上有 N 个数据项目的信息需要完善。要现在处理吗？"

When the user agrees, for each pending item:
1. Show project name, uncertain field, candidates (A/B/C/D)
2. Execute: `fan-files projects update <name> --species "<choice>" --confidence high`
3. Finish with: `fan-files pending --clear`

## Best Practices

1. **Startup check**: run `fan-files pending --json` each session
2. **Coverage check**: if `fan-files status` shows <50% metadata, suggest `fan-files infer`
3. **Multi-variant search**: generate Latin name + common name + abbreviation variants
4. **Proactive discovery**: mention available related data even if user didn't ask
5. **Use --json**: for programmatic parsing
