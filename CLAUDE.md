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

## Interactive Metadata Review

At the start of each conversation, Claude will check for pending metadata items by running `fan-files pending --json` in the background. If there are items with uncertain metadata (low-confidence LLM inferences), Claude will proactively offer to help the user review and correct them. The flow:

1. Claude displays the project name, uncertain fields, and current guesses
2. Claude presents candidate options (A/B/C/D) for the user to choose from, or the user can type their own answer
3. Claude runs `fan-files projects update <name> --species "<choice>" --confidence high`
4. Repeats for each pending item until all are resolved
5. Finishes with `fan-files pending --clear`
