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

## 元数据完善

### 启动时检查
每次对话开始时，后台运行检查：
```
fan-files pending --json
```
如果返回非空数组，主动告知用户，例如：
"服务器上有 N 个数据项目的信息需要完善。要现在处理吗？"

### 交互流程
当用户同意处理时，逐个展示每个待完善项目：

1. 显示项目名、不确定的字段和当前猜测值
2. 显示候选选项（A/B/C/D），让用户选择或自行输入
3. 用户选择后，执行：
```
fan-files projects update <name> --species "<用户选择>" --confidence high
```
4. 移到下一个项目，直到全部完成

### 完成后
```
fan-files pending --clear
```
告知用户所有项目已更新。
