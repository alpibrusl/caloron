# Live Demos

Real recordings of Caloron sprints — not simulations.

## Full Sprint: Charging Optimizer

PO Agent generates a DAG, agents build a charging window optimizer for electric trucks, reviewer catches a bug, agent fixes it.

<div id="demo-sprint"></div>
<script>
  AsciinemaPlayer.create('https://alpibrusl.github.io/caloron/casts/full-sprint.cast', 
    document.getElementById('demo-sprint'), {
    cols: 120, rows: 35, speed: 2, theme: 'monokai', idleTimeLimit: 2
  });
</script>

**What happens:**

1. PO Agent → 2 tasks (optimizer + tests) with dependency
2. Agent writes `src/optimizer.py` — sliding window algorithm
3. PR created on Gitea → Reviewer: "CHANGES_NEEDED: no tests"
4. Agent fixes → adds tests → Reviewer: "APPROVED"
5. PR merged → Task 2 unblocked → tests agent runs
6. Retro: clarity 7/10, 1 review cycle, agents evolve

---

## Supervisor in Action

What happens when an agent stalls? The supervisor detects it, probes, restarts, and escalates.

<div id="demo-supervisor"></div>
<script>
  AsciinemaPlayer.create('https://alpibrusl.github.io/caloron/casts/supervisor.cast', 
    document.getElementById('demo-supervisor'), {
    cols: 120, rows: 25, speed: 2, theme: 'monokai', idleTimeLimit: 2
  });
</script>

**What happens:**

1. Agent timeout set to 5 seconds (forcing stalls)
2. Supervisor: PROBE → posts comment on Gitea issue
3. Supervisor: RESTART → retries with simplified prompt
4. Task marked as FAILED → feedback posted
5. Retro: "4 supervisor interventions, reduce agent stalls"

---

## Sprint-Over-Sprint Learning

Sprint 2 PO receives learnings from Sprint 1 — task specs improve, review cycles decrease.

<div id="demo-learning"></div>
<script>
  AsciinemaPlayer.create('https://alpibrusl.github.io/caloron/casts/learning.cast', 
    document.getElementById('demo-learning'), {
    cols: 120, rows: 30, speed: 2, theme: 'monokai', idleTimeLimit: 2
  });
</script>

**What happens:**

1. Sprint 1: clarity 5.5/10, reviewer rejects → agent fixes → approved
2. Retro saves learnings: "improve task specs"
3. Sprint 2: PO receives learnings → clearer tasks
4. Sprint 2: clarity 7.0/10, approved on first review
5. Agents evolved v1.0 → v1.1 (added self-review instruction)

---

## Recording Your Own Demos

```bash
# Install asciinema
pip install asciinema

# Record a sprint
asciinema rec my-demo.cast -c "python3 examples/e2e-local/orchestrator.py 'your goal here'"

# Play it back
asciinema play my-demo.cast

# Upload to asciinema.org
asciinema upload my-demo.cast
```

!!! note
    The demo recordings use local Gitea and Claude Pro subscription.
    No API keys or external services required.
