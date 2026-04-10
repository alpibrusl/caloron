# Caloron

**Multi-Agent Orchestration Platform — agents collaborate through Git to build software.**

## What It Does

You give Caloron a goal. It builds the software:

```
$ python3 orchestrator.py "Build a charging optimizer for electric trucks"

  PO Agent → 2 tasks with dependencies
  Agent 1 → src/optimizer.py (sliding window + SoC validation)
  Agent 2 → tests/test_optimizer.py (29 tests)
  Reviewer → "CHANGES_NEEDED: no input validation"
  Agent 1 → fixes code, adds validation
  Reviewer → "APPROVED"
  PR merged
  Retro → clarity 7/10, 0 blockers, agents evolved v1.0 → v1.1
```

All real: Claude writes code, Gitea has the PRs, the reviewer catches real bugs, agents learn from feedback.

## Proven Capabilities

| Feature | Evidence |
|---------|----------|
| PO generates DAG dynamically | Claude decides tasks + dependencies from goal |
| Non-linear DAGs | Fan-out, diamond joins, complex dependency graphs |
| Agents write real code | 29 passing tests on a charging optimizer |
| PR review cycle | Reviewer rejected → agent fixed → approved |
| Supervisor | Probe → restart → escalate (proven with forced timeout) |
| Agent feedback | Agents report clarity, blockers, tools used |
| Retro with real KPIs | Completion rate, clarity, review cycles |
| Sprint-over-sprint learning | Sprint 2 PO received Sprint 1 learnings |
| Agent versioning | Auto-evolve: low clarity → add instructions, high failure → stronger model |
| Filesystem sandbox | bubblewrap: host filesystem hidden from agents |
| Multi-framework | claude-code, gemini-cli, aider, codex-cli per task |

## Quick Start

```bash
git clone https://github.com/alpibrusl/caloron
cd caloron

# Run a real sprint against local Gitea
docker run -d --name gitea -p 3000:3000 gitea/gitea:1.22
python3 examples/e2e-local/orchestrator.py "Build a Python calculator with tests"
```

See the [Full Sprint Demo](examples/full-sprint.md) for the complete walkthrough.

## Architecture

```
Human: "Build X"
  → PO Agent generates DAG (tasks + deps + agent specs)
  → Agent Version Store loads/creates agent configs
  → For each task (respecting dependency order):
      Agent writes code (sandboxed via bubblewrap)
      → Branch pushed to Gitea
      → PR created
      → Reviewer reviews (may request changes → agent fixes → re-review)
      → PR merged
      → Feedback posted (agent's own assessment)
  → Retro: collect feedback → KPIs → improvements
  → Auto-evolve agents based on retro findings
  → Learnings saved for next sprint
```

## Two Implementations

| | [caloron](https://github.com/alpibrusl/caloron) | [caloron-noether](https://github.com/alpibrusl/caloron-noether) |
|---|---|---|
| Language | Rust + Python orchestrator | Python stages + Rust shell |
| Architecture | Monolith daemon | Noether composition graphs |
| Scaling | Single machine → workers | Docker Compose → Kubernetes |
| Lines | ~10,000 (Rust) + 900 (Python) | ~1,200 (Python) + 200 (Rust) |
| Best for | CLI tools, type safety | Enterprise, distribution |
