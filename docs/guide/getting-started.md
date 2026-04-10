# Getting Started

## Prerequisites

- **Python 3.11+** (for the orchestrator and agents)
- **Claude Code** (your Pro subscription works, or set `ANTHROPIC_API_KEY`)
- **Docker** (for local Gitea — optional if using GitHub)
- **Git**

Optional:
- **Nix** (for hermetic agent environments — not required)
- **bubblewrap** (`bwrap`) — for filesystem sandboxing

## Installation

```bash
git clone https://github.com/alpibrusl/caloron
cd caloron
cargo build --workspace
```

## Run Your First Sprint

The fastest way to see Caloron work — a real sprint against local Gitea:

```bash
# Start Gitea (one-time)
docker run -d --name gitea -p 3000:3000 gitea/gitea:1.22

# Run a sprint
python3 examples/e2e-local/orchestrator.py \
  "Build a Python module with is_palindrome function. Include tests."
```

This will:
1. PO Agent (Claude) generates a task DAG
2. Agents write real code in sandboxed environments
3. PRs created on Gitea with code reviews
4. Reviewer catches issues → agent fixes → re-review
5. Retro analyzes feedback and evolves agents

See [Full Sprint Example](../examples/full-sprint.md) for the complete walkthrough.

## Using with GitHub (instead of Gitea)

Set your GitHub token:

```bash
export GITHUB_TOKEN="ghp_..."
export REPO="owner/repo"
```

The orchestrator works with any GitHub-compatible API.

## Using API Keys (instead of Claude Pro)

```bash
export ANTHROPIC_API_KEY="sk-ant-..."
# Or for other frameworks:
export GOOGLE_API_KEY="..."     # for Gemini
export OPENAI_API_KEY="..."     # for Codex
```

## CLI Tools

```bash
# Generate an agent definition
caloron agent generate -p developer -c code-writing,python -m balanced -f claude-code

# Validate an agent definition
caloron agent validate examples/agents/backend-developer.yaml

# View sprint status
caloron status

# Run retro
caloron retro
```

## Next Steps

- [Full Sprint Example](../examples/full-sprint.md) — proven end-to-end with real code
- [Core Concepts](concepts.md) — sprints, agents, DAGs, Git protocol
- [Agent Definitions](agents.md) — configure agents with 4-axis composition
- [Scalability](scalability.md) — single machine to Kubernetes
