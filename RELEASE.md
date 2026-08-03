# fan-files v0.2.0 — LLM Container Classification & Compression Handling

> The "LLM as Generalization Mechanism" release — zero hardcoded naming rules, 99.2% recall on 60 plant orders.

## New Features

### LLM-Assisted Container Classification
PropagatedBio directories (0 own files, children have BIO data) are now classified by LLM into:
- **analysis_project**: Pipeline projects like `tie_sRNA/` with ordered steps → kept as Dataset
- **taxonomic_container**: Species/cultivar containers like `Brassica_napus/` → correctly excluded

This replaces the previous hardcoded PropagatedBio exclusion, fixing the bak_201 analysis pipeline regression while maintaining high precision on Orders genomic data.

### Generic Compression Suffix Recognition
Any BIO file extension × any compression format (`.gz`, `.bz2`, `.bz`, `.xz`, `.zst`, `.zstd`) is automatically recognized — no need to enumerate combinations in code.

### path_segments in Phase C JSON
Candidate entries now include `path_segments` (directory hierarchy as a string array), enabling the LLM to understand parent-child relationships and infer Dataset boundaries without hardcoded depth rules.

### Three-Layer Prompt System (L1/L2/L3)
- **L1** (system): Core concepts — Dataset/Asset/File model, path_segments interpretation
- **L2** (user rules): Editable Markdown for project-specific naming conventions, noise filtering, container classification patterns
- **L3** (auto-generated): Correction memory — `fan-files correct` accumulates patterns, auto-activates at threshold ≥ 3

## Improvements

| Metric | v0.1.8 | v0.2.0 | Change |
|--------|:------:|:------:|:------:|
| Active Recall (Orders, 1431 gold) | 96.9% | **99.2%** | +2.3% |
| Precision | 82.3% | **96.3%** | +14.0% |
| Extra Datasets (noise) | 321 | **58** | -82% |
| Cucurbitaceae Active Recall | 100% | 100% | — |
| bak_201 Analysis Pipeline | 3/6 | **6/6** | +50% |

## Fixes

- **Phase C threshold**: `< 2 files` → `is_empty()` — single-file datasets (protein.fa) are no longer skipped
- **PropagatedBio injection removed**: Containers are no longer sent as Phase C candidates, eliminating duplicate file listings that caused LLM confusion
- **Acorale exclude pattern**: `starts_with(".../Acorale")` now uses trailing `/` to avoid matching `Acorales`
- **LLM failure resilience**: Phase A auto_targets are preserved even when LLM calls fail

## Cross-Dataset Generalization (Zero-Shot)

Tested on 10 previously unseen data roots without any code adaptation:

| Dataset | Type | Files | Datasets | Types |
|---------|------|:-----:|:--------:|:-----:|
| hse (1.4TB) | Mixed genomics | 26,171 | 565 | 6 |
| bak_202 (54GB) | Hi-C + RNA-seq + BLAST | 13,025 | 168 | 9 |
| share/projects (6.5TB) | 8 research projects | 2,066 | 16 | 6 |
| tian-rnaseq (3.7GB) | RNA-seq | 3,921 | 16 | 2 |
| Total (10 roots) | — | 45,506 | 784 | 9 |

## Assets

- `fan-files-linux-x86_64.tar.gz` — Linux (glibc, x86_64)
- `fan-files-macos-arm64.tar.gz` — macOS (Apple Silicon)

## Installation

```bash
# Linux
tar xzf fan-files-linux-x86_64.tar.gz
sudo mv fan-files /usr/local/bin/

# macOS
tar xzf fan-files-macos-arm64.tar.gz
sudo mv fan-files /usr/local/bin/

# Verify
fan-files --version
fan-files discover
```
