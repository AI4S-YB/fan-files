# Fan-Files Plugin for Claude Code

This plugin gives Claude Code access to a local `fan-files` server that has indexed all files on a bioinformatics server, including LLM-inferred project metadata, species, and assay types.

## Prerequisites

- `fan-files` binary installed and configured on the server
- LLM API key configured (DeepSeek, OpenAI, etc.)
- `fan-files daemon` has been run at least once to index files

## What This Plugin Does

When you ask Claude Code questions about available data on the server, it automatically:

1. Runs `fan-files search` to find relevant files
2. Shows available projects with `fan-files projects`
3. Gets file details with `fan-files info`
4. Suggests related data with `fan-files suggest`
