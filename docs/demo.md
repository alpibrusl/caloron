# Live Demo

Real recording of a Caloron sprint — not a simulation.

## Full Sprint: Charging Optimizer

PO Agent generates a DAG, agents build a charging window optimizer for electric trucks, reviewer catches a bug, code gets merged on Gitea.

[![asciicast](https://asciinema.org/a/VtPLoUxYinUTiJjf.svg)](https://asciinema.org/a/VtPLoUxYinUTiJjf)

**What happens (3.5 minutes):**

1. **PO Agent** analyzes the goal → generates 2 tasks with dependency
2. **Agent 1** writes `src/charging_optimizer.py` (79 lines) — sliding window + SoC validation
3. **PR created on Gitea** → Reviewer approves → **merged**
4. **Agent 2** writes `tests/test_charging_optimizer.py` (21 test cases)
5. **PR created** → Reviewer: "APPROVED" → **merged**
6. **Code shown** — the actual generated optimizer code
7. **Tests run** — pytest results displayed
8. **Sprint KPIs** — tasks, PRs, reviews, test pass rate
9. **Next sprint proposal** — departure deadlines, multi-truck scheduling, input validation

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

---

## OTA Demo: Hotel Rate Anomaly Detector

PO Agent generates 3 tasks for a revenue management tool: anomaly detector, sample data, and tests.

[![asciicast](https://asciinema.org/a/AsOjukdSB9l968o5.svg)](https://asciinema.org/a/AsOjukdSB9l968o5)

**What the PO decided:**

1. **Implement anomaly detector** — z-score with rolling window, per-hotel grouping
2. **Generate sample CSV** — test data with known anomalies
3. **Write pytest tests** — covering all functions and edge cases

**Code produced (task 1, before rate limit):**

```python
def detect_anomalies(df, window=30, z_threshold=2.0):
    for _, group in df.groupby("hotel_id"):
        group["expected_rate"] = group["rate"].rolling(window, min_periods=7).mean()
        rolling_std = group["rate"].rolling(window, min_periods=7).std()
        group["z_score"] = (group["rate"] - group["expected_rate"]) / rolling_std
        group["is_anomaly"] = group["z_score"].abs() > z_threshold
```

**Next sprint proposal:**

1. Add competitor rate comparison
2. Seasonal decomposition (separate trend before z-score)
3. REST API endpoint (FastAPI)
4. Dashboard data export (JSON for revenue team)

!!! note
    Tasks 2-3 hit Claude Pro rate limit during recording.
    Re-record after limit resets for complete demo.
