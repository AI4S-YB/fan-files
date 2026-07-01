---
name: fan-files
description: Use when the user needs to install, configure, search, or manage bioinformatics data on the server. Activates for data discovery, file search, project management, metadata review, server management, installing fan-files, setting up scanning, upgrading, or uninstalling.
---

# Fan-Files: Server Data Intelligence

You have access to `fan-files`, a tool that indexes ALL files on servers and uses LLM to infer biological metadata (project, species, assay type). It supports multi-user public/private indexes, remote server scanning, and real-time file monitoring. You can perform ALL fan-files operations on behalf of the user.

## Administrative Commands

### Installation
User can ask you to install fan-files from scratch:
```bash
curl -fsSL https://raw.githubusercontent.com/AI4S-YB/fan-files/main/install.sh | bash
```
Or download pre-built binary from GitHub Releases:
```bash
# macOS (Apple Silicon)
curl -fsSL https://github.com/AI4S-YB/fan-files/releases/latest/download/fan-files-aarch64-apple-darwin.tar.gz | tar -xz
```

### First-time setup (user)
```bash
fan-files init
```
Guides through: scan directories → remote servers → LLM provider → start scanning.

### First-time setup (admin — global public index)
```bash
sudo fan-files init --global
sudo fan-files daemon --global
```
Creates public index at `/var/lib/fan-files/`, readable by all users.

### Start background scanning
```bash
fan-files daemon              # User private index
sudo fan-files daemon --global # Global public index
```
Scans configured directories, watches for changes, auto-runs LLM inference.

### Server Management
```bash
fan-files servers list                    # List registered servers (with source info)
fan-files servers add <name>              # Interactive add (supports multiple scan paths)
fan-files servers remove <name>           # Remove a server
fan-files servers scan <name>             # Scan a server (cache-first mode)
fan-files servers scan --agent <name>     # Scan using fan-agent (remote local scan)
fan-files servers watch <name>            # Real-time file monitoring (inotify)
```

### Upgrade
```bash
fan-files update
```
Downloads latest binary from GitHub Releases, replaces current installation.

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
Searches both private and public indexes. Results show `<server>:<path>` format.

### Browse projects (LLM-inferred)
```
fan-files projects                       # list all with species, assay, server
fan-files projects <project_name>        # details
fan-files projects update <name> --species "Apple" --confidence high
```

### File details
```
fan-files info <path> [--json]
```
Shows source server, size, format, bio metadata.

### Find related data
```
fan-files suggest <project_directory> [--json]
```

### Precise listing
```
fan-files list --tag paired-end
fan-files list --server dev-server
fan-files list --category genome [--json]
```

### System status
```
fan-files status                  # Private index
fan-files --global status         # Global public index
```
Shows file count, metadata coverage, and per-server breakdown.

### Re-run metadata inference
```
fan-files infer                   # Private index
fan-files --global infer          # Global index
```
Uses batched LLM calls (80 dirs/batch) for large datasets.

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

## Multi-User Architecture

fan-files supports two data layers:
- **User private** (`~/.fan-files/`): Each user's own scans, only accessible to them
- **Global public** (`/var/lib/fan-files/`): Admin-managed, read-only for all users

All queries automatically merge both layers. Use `--global` flag to operate on the global layer.

## Best Practices

1. **Startup check**: run `fan-files pending --json` each session
2. **Coverage check**: if `fan-files status` shows <50% metadata, suggest `fan-files infer`
3. **Multi-variant search**: generate Latin name + common name + abbreviation variants
4. **Server awareness**: use `fan-files servers list` to show available data sources
5. **Proactive discovery**: mention available related data even if user didn't ask
6. **Use --json**: for programmatic parsing
7. **Global queries**: when searching for shared/public data, try `fan-files --global search`
