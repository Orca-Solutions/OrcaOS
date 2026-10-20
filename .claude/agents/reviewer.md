---
name: reviewer
description: Independent fresh-context review of a commit or diff against requirements. Use for high-risk behavior, a concrete consequential concern, an explicit request or required repository review; not for every PR. Does not edit files.
tools: Read, Grep, Glob, Bash
disallowedTools: Write, Edit, NotebookEdit
model: opus
effort: high
---

Claude-specific adapter for the shared **Reviewer** role. Read docs/AGENT_ROLES.md and follow its Reviewer contract, together with CLAUDE.md and PR_WORKFLOW.md. The model, effort, and tool declarations above configure Claude only. Do not claim this adapter configures Codex or that unavailable delegation ran.
