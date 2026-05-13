# 分析助手 Skill

## 概述

一个 Claude Code Skill，将 fan-files 数据搜索 + WebSearch 文献调研 + 分析规划编排成一个工作流。用户用自然语言描述科学问题，Skill 驱动 Claude 完成调研→搜索→规划→执行。

## 工作流

```
用户: "我想在苹果中找出和花青素合成相关的基因"
  ↓
步骤1 — 文献调研
  Claude 调 WebSearch 搜 PubMed：
  "apple anthocyanin biosynthesis gene 2024"
  提取关键基因名、已知通路、常用分析方法
  ↓
步骤2 — 数据搜索
  Claude 调 fan-files search "apple RNA-seq"
  Claude 调 fan-files search "apple genome"
  Claude 调 fan-files search "apple annotation"
  列出服务器上所有可用的苹果相关数据
  ↓
步骤3 — 匹配评估
  对比文献提到的数据需求 vs 服务器已有数据
  指出缺什么、有什么
  ↓
步骤4 — 输出分析计划
  ## 分析计划：苹果花青素合成基因挖掘
  ### 可用数据
  - apple_rnaseq_test (RNA-seq, 3条件×2重复)
  - SMT2024_genome (参考基因组)
  ### 缺失数据
  - 可能需要更多组织的表达数据
  ### 分析步骤
  1. 用 SMT2024_genome 做参考比对
  2. 对 apple_rnaseq_test 做差异表达分析
  3. 与已知花青素基因集做交集
  ...
  ↓
步骤5 — 用户确认 (可选)
  "要执行这个分析计划吗？"
```

## 实现方式

纯 Skill 文件，不涉及 fan-files 代码改动。Skill 内容定义 Claude 的行为流程。依赖：

| 能力 | 来源 |
|------|------|
| 文献搜索 | Claude 内置 WebSearch |
| 本地数据搜索 | `fan-files search` |
| 分析规划 | Claude 自身推理能力 |
| 执行 | Claude 的 bash 工具 + 用户已有的生物信息 skill |

## Skill 文件

`skills/analysis-assistant.md`，包含上述工作流指引。
