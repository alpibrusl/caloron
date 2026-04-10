# Live Demo

Real recording of a Caloron sprint — not a simulation.

## Full Sprint: Charging Optimizer

PO Agent generates a DAG, agents build a charging window optimizer for electric trucks, reviewer catches a bug, code gets merged on Gitea.

[![asciicast](https://asciinema.org/a/ZsYVCt7uFAiTnEoP.svg)](https://asciinema.org/a/ZsYVCt7uFAiTnEoP)

**What happens (5 minutes):**

1. **PO Agent** analyzes the goal → generates 2 tasks with dependency
2. **Agent 1** writes `src/optimizer.py` — sliding window + SoC validation
3. **PR created on Gitea** → Reviewer: "CHANGES_NEEDED: crashes when..."
4. **Agent 2** writes `tests/test_optimizer.py`
5. **PR created** → Reviewer: "APPROVED" → **merged**
6. **Sprint retro** — KPIs, blockers, audit trail

All real: Claude writes code, Gitea has the PRs, the reviewer catches a real bug.

---

## Recording Your Own

```bash
# Record a sprint with any goal
asciinema rec my-demo.cast \
  -c "python3 examples/e2e-local/orchestrator.py 'your goal here'"

# Play it back (2x speed recommended)
asciinema play my-demo.cast --speed 2

# Upload to share
asciinema upload my-demo.cast
```

## Also See

- [caloron-noether demo](https://asciinema.org/a/GOMIILJSz8ZpeF0R) — same sprint via Noether stages
- [Full Sprint Walkthrough](examples/full-sprint.md) — detailed text explanation
