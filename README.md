# Caloron

Multi-Agent Orchestration Platform — agents collaborate through Git to build software.

Caloron orchestrates AI agents that communicate via GitHub issues, pull requests, and code reviews. The orchestrator manages their lifecycle in isolated Nix environments, detects failures via a structured supervisor, and learns between sprints through automated retrospectives.

## Quick Start

```bash
cargo build --workspace

# Validate an agent definition
caloron agent validate examples/agents/backend-developer.yaml

# Generate an agent from composable axes
caloron agent generate -p developer -c code-writing,testing,rust -m balanced -f claude-code

# Start a sprint
caloron start --dag examples/dag.json

# Check status (current project)
caloron status

# Cross-project dashboard
caloron dashboard

# Run retro with KPIs and improvements
caloron retro
```

## Documentation

Full docs at [docs/](docs/) — build with `mkdocs serve`.

- [Getting Started](docs/guide/getting-started.md)
- [Core Concepts](docs/guide/concepts.md)
- [Agent Definitions](docs/guide/agents.md)
- [Sprint Lifecycle](docs/guide/sprint-lifecycle.md)
- [Architecture](docs/architecture.md)
- [End-to-End Example](docs/examples/e2e-sprint.md)

## Architecture

```
Human → caloron kickoff → PO Agent → DAG → Daemon
                                              ├── Agent Spawner (Nix + worktree)
                                              ├── Git Monitor (events → DAG transitions)
                                              ├── Supervisor (health → probe/restart/escalate)
                                              └── Retro Engine (feedback → KPIs → improvements)
```

Agents are defined by four composable axes:
**Personality** (developer, qa, reviewer, architect, designer, ux-researcher, devops) ×
**Capabilities** (code-writing, testing, rust, python, frontend, browser-research, noether) ×
**Model** (claude-sonnet, claude-opus, gemini-pro, gemini-flash) ×
**Framework** (claude-code, gemini-cli, aider, codex-cli)

## License

[EUPL-1.2](LICENSE) (European Union Public License)
