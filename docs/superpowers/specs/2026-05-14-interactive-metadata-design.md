# 交互式元数据完善

## 概述

LLM 推理（`fan-files infer`）完成后，对置信度为 `low` / `medium` 的项目，以选择题方式引导用户补全物种、实验类型等信息。交互由 Claude Code Skill 驱动，fan-files 提供数据读写能力。

## 触发方式

| 时机 | 行为 |
|------|------|
| `fan-files infer` 完成后 | 自动扫描低置信度项目，LLM 生成候选选项，写入待完善清单 |
| Claude Code 启动时 | Skill 检查 `pending_review.json`，有待完善项则主动提醒 |
| 用户输入 `/fix-metadata` | 手动进入元数据完善流程 |

## 存储

`~/.fan-files/pending_review.json`：

```json
[
  {
    "project": "SMT2024_genome",
    "field": "species",
    "current_value": "SMT2024",
    "confidence": "medium",
    "candidates": ["苹果", "拟南芥", "水稻", "玉米"],
    "timestamp": 1715664000
  }
]
```

## CLI 命令

```
fan-files pending                          # 列出所有待完善项目（JSON 输出）
fan-files projects update <name> --species "拟南芥" --confidence high  # 更新物种
fan-files projects update <name> --assay-type "WGS"                    # 更新实验类型
fan-files projects update <name> --candidates "A,B,C"                  # 手动设置候选选项
fan-files pending clear                    # 清除已处理的项目
```

## 交互流程

```
fan-files infer 完成
  → 扫描 project 表中 species_confidence = low/medium 的项目
  → LLM 为每个项目生成候选选项（基于项目名、目录、现有物种列表）
  → 写入 pending_review.json

Claude Code 启动 / /fix-metadata 触发
  → fan-files pending --json
  → Claude 解析清单，逐个以选择题展示
  → 用户选择或输入
  → Claude 调 fan-files projects update ... 写入
  → 循环直到清单为空
```

## Skill 行为

`skills/fan-files.md` 新增元数据完善指引：

- 启动时检查 `fan-files pending --json`
- 如果清单非空，主动提醒用户
- 以选择题方式逐个引导，每轮展示当前项目名、不确定字段、候选选项
- 用户可选择候选、自定义输入、或跳过
- 完成后调 `fan-files pending clear`
