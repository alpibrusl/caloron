# Caloron

Multi-Agent Orchestration Platform — agents collaborate through Git to build software.

Caloron orchestrates AI agents that communicate via GitHub/Gitea issues, pull requests, and code reviews. The orchestrator manages their lifecycle in sandboxed environments, detects failures via a structured supervisor, and learns between sprints through automated retrospectives.

## Proven End-to-End (no mocks)

```
Human: "Build a Python module with is_palindrome function. Include tests."
  ↓
PO Agent (Claude, sandboxed) → generates 2-task DAG
  ↓
Issue #1 created on Gitea → Agent writes src/palindrome.py (sandboxed)
  → PR #3 created → Reviewer: "CHANGES_NEEDED: No tests"
  → Agent fixes code, adds tests → Reviewer: "APPROVED" → PR merged
  ↓
Issue #2 created → Agent writes tests/test_palindrome.py
  → PR #4 created → Reviewer: "APPROVED" → PR merged
  ↓
Retro: 2/2 completed, clarity 5.5/10, 1 blocker ("no tests in impl task")
  → Learnings saved → Sprint 2 PO receives improvements
```

All steps are real: Claude writes code, Gitea has the PRs, the reviewer caught a real issue, the agent fixed it.

## Quick Start

```bash
# Build
cargo build --workspace

# Run a full autonomous sprint against local Gitea
docker run -d --name gitea -p 3000:3000 gitea/gitea:1.22
python3 examples/e2e-local/orchestrator.py "Build a calculator CLI with tests"

# Or use the CLI tools
caloron agent generate -p developer -c code-writing,python -m balanced -f claude-code
caloron agent validate examples/agents/backend-developer.yaml
caloron kickoff "implement user authentication"
caloron status
caloron retro
```

## What's Been Proven

| Capability | Status |
|---|---|
| PO Agent generates DAG from goal | Real Claude calls |
| Agents write code in sandboxed environments | bubblewrap + Nix |
| PRs created on Gitea with code reviews | Real reviewer agent |
| Changes requested → agent fixes → re-review | Proven in Sprint 1 |
| Supervisor detects stalls (probe → restart → escalate) | Proven with 5s timeout |
| Feedback posted as YAML on Gitea issues | Retro reads from Gitea |
| KPIs + improvements generated | Completion rate, clarity, blockers |
| Sprint-over-sprint learning | Sprint 2 PO received Sprint 1 learnings |

## Architecture

```
Human → PO Agent → DAG → Orchestrator
                           ├── Agent Spawner (bubblewrap + worktree)
                           ├── Gitea API (issues, PRs, reviews, merges)
                           ├── Supervisor (timeout → probe → restart → escalate)
                           └── Retro Engine (feedback → KPIs → learnings → next sprint)
```

Agents are defined by four composable axes:
**Personality** (developer, qa, reviewer, architect, designer, ux-researcher, devops) ×
**Capabilities** (code-writing, testing, rust, python, frontend, browser-research, noether) ×
**Model** (claude-sonnet, claude-opus, gemini-pro, gemini-flash) ×
**Framework** (claude-code, gemini-cli, aider, codex-cli)

## Documentation

Full docs at [docs/](docs/) — build with `mkdocs serve`.

- [Getting Started](docs/guide/getting-started.md)
- [Core Concepts](docs/guide/concepts.md)
- [Sprint Lifecycle](docs/guide/sprint-lifecycle.md)
- [Agent Definitions](docs/guide/agents.md)
- [Full Sprint Example](docs/examples/full-sprint.md)
- [Architecture](docs/architecture.md)
- [Scalability](docs/guide/scalability.md)

## License

[EUPL-1.2](LICENSE) (European Union Public License)
