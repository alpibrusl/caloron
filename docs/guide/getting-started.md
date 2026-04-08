# Getting Started

## Prerequisites

- **Rust** 1.75+ (for building Caloron)
- **Nix** 2.x+ (for agent environment isolation)
- **Git** (agents communicate through Git)
- **GitHub token** with `repo` and `workflow` scopes

## Installation

```bash
git clone https://github.com/caloron/caloron
cd caloron
cargo build --workspace
```

This produces two binaries:

- `target/debug/caloron-daemon` — The main orchestrator (aliased as `caloron`)
- `target/debug/caloron-harness` — The agent harness (runs inside Nix environments)

## Configuration

Create `caloron.toml` in your project root:

```toml
[project]
name = "my-project"
repo = "owner/repo"              # GitHub owner/repo
meta_repo = "owner/caloron-meta" # Where agent definitions live

[github]
token_env = "GITHUB_TOKEN"       # Env var name (not the token itself)
polling_interval_seconds = 5

[llm]
api_key_env = "ANTHROPIC_API_KEY"

[llm.aliases]
default = "claude-sonnet-4-6"
strong = "claude-opus-4-6"

[nix]
enabled = true

[supervisor]
stall_default_threshold_minutes = 20
max_review_cycles = 3

[retro]
enabled = true
auto_run = true
```

Set the required environment variables:

```bash
export GITHUB_TOKEN="ghp_..."
export ANTHROPIC_API_KEY="sk-ant-..."
```

## Validate an Agent

Agent definitions are YAML files describing what tools and LLM configuration an agent has:

```bash
caloron agent validate examples/agents/backend-developer.yaml
```

```
Agent: backend-developer v1.0
Model: default
Tools: github_mcp, noether, bash
Nix packages: nodejs_20, python311, rustc, cargo
Credentials: GITHUB_TOKEN, ANTHROPIC_API_KEY

Validation: PASSED
```

## Build a Nix Environment

Preview the Nix environment that will be created for an agent:

```bash
caloron agent build examples/agents/backend-developer.yaml
```

With `nix.enabled = true`, this writes a `flake.nix` and builds the environment. The Nix store caches the result, so subsequent builds are instant.

## Start a Sprint

```bash
# Interactive kickoff (PO Agent generates DAG)
caloron kickoff "implement user authentication"

# Or start directly with a pre-made DAG
caloron start --dag examples/dag.json
```

## Monitor Progress

```bash
caloron status
```

```
Sprint: sprint-2026-04-w2
Goal: Implement user authentication

Tasks:
  [v] task-1       JWT implementation                        DONE         (backend-1)
  [>] task-2       Session store                             IN_PROGRESS  (backend-2)
  [ ] task-3       Integration tests                         PENDING      (qa-1)

Progress: 1/3 tasks done
```

## Run Retro

After the sprint completes:

```bash
caloron retro
```

This generates a markdown report analyzing task clarity, blockers, review loops, and token efficiency.

## Next Steps

- [Core Concepts](concepts.md) — Understand sprints, agents, DAGs, and the Git protocol
- [Agent Definitions](agents.md) — Write custom agent definitions
- [Sprint Lifecycle](sprint-lifecycle.md) — The full execution flow
- [End-to-End Example](../examples/e2e-sprint.md) — A complete walkthrough
