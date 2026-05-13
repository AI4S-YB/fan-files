---
name: analysis-assistant
description: Use when planning bioinformatics analysis — search literature, find data, create analysis plans
---

# Bioinformatics Analysis Assistant

When a user describes a scientific question or analysis goal, follow this workflow.

## Workflow

### Step 1: Literature Search
Search PubMed for relevant background:
```
WebSearch "topic keywords recent year"
```
Extract: known genes, pathways, common analysis methods, typical data requirements.

### Step 2: Local Data Search
Use fan-files to find available data on this server:
```
fan-files search "<species> <assay_type>"
fan-files projects
fan-files search "<gene name>"
```
Ask about data quality (replicates, conditions) if relevant.

### Step 3: Gap Analysis
Compare what the literature suggests vs. what's available on the server.
Tell the user what's available and what might be missing.

### Step 4: Analysis Plan
Output a clear plan with sections:

```
## Analysis Plan: [Goal]

### Available Data
- project_name (assay_type, N files, species)

### Missing/Optional
- data not on this server that could be useful

### Steps
1. step description
2. ...
```

### Step 5: Execution (Optional)
Ask the user: "Execute this plan?" If yes, run each step via bash commands.
For each step completed, report the result before moving to the next.

## Key Principles
- Always search BOTH literature AND local data before proposing a plan
- If critical data is missing, warn the user
- Adapt the plan to what's actually available, not what would be ideal
- Use fan-files projects to understand data context (species, assay type)
