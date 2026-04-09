# Full Sprint Example (Proven, No Mocks)

This example runs an autonomous sprint against local Gitea. Every step is real: Claude writes code, Gitea has real PRs, the reviewer finds real issues, the agent fixes them.

## Prerequisites

- Docker (for Gitea)
- Claude Code (your Pro subscription)
- Python 3.11+

## Setup

```bash
# Start Gitea
docker run -d --name gitea -p 3000:3000 gitea/gitea:1.22

# Create user and token (one-time)
docker exec -u git gitea gitea admin user create \
  --username caloron --password caloron123 \
  --email caloron@test.local --admin --must-change-password=false
```

## Run a Sprint

```bash
python3 examples/e2e-local/orchestrator.py \
  "Build a Python module with is_palindrome function. Include pytest tests."
```

## What Happens

### Step 1: PO Agent generates the DAG

Claude analyzes the goal and decides what tasks are needed:

```
--- Step 1: PO Agent ---
  1: Implement is_palindrome function (deps: none)
  2: Write pytest tests for is_palindrome (deps: 1)
```

### Step 2: Issues created on Gitea

```
--- Step 2: Issues ---
  Issue #1: Implement is_palindrome function
  Issue #2: Write pytest tests for is_palindrome
```

### Step 3: Agent writes code (sandboxed)

Each agent runs inside bubblewrap — can only write to its worktree, host filesystem hidden.

```
  Agent running (sandboxed, supervised)...
    Created src/palindrome.py with is_palindrome function.
  Uploaded: src/palindrome.py
  PR #3 created
```

### Step 4: Reviewer catches a real issue

```
  Reviewer (cycle 1)...
  Review: CHANGES_NEEDED: No tests. A tests/test_palindrome.py file is required.
```

### Step 5: Agent fixes the code

```
  Agent fixing: No tests required...
    Added tests covering spaces, punctuation, non-palindromes.
  Pushed fix (2 files)
```

### Step 6: Reviewer approves on second review

```
  Reviewer (cycle 2)...
  Review: APPROVED
  PR #3 MERGED ✓
```

### Step 7: Retro

```
=== RETRO ===
  Tasks completed:      2/2
  Avg clarity:          5.5/10
  Blockers (1):
    - Review cycle 1: No tests required
  Improvements:
    → Improve task specifications — 1 tasks had clarity < 5/10
  Learnings saved (1 sprints total)
```

## Sprint-Over-Sprint Learning

Run a second sprint — the PO receives learnings from Sprint 1:

```bash
python3 examples/e2e-local/orchestrator.py \
  "Build count_vowels and reverse_string functions. Include tests."
```

Sprint 2 output shows:

```
  FULL AUTONOMOUS SPRINT #2
  Goal: Build count_vowels and reverse_string functions. Include tests.
  (with learnings from 1 previous sprint(s))
```

The PO prompt now includes:

```
Last sprint: 2/2 completed, clarity 5.5/10, 0 supervisor interventions.
Pending improvements: Improve task specifications
Common blockers: No tests required in implementation task
```

## Supervisor Demo

Force a timeout to see the supervisor in action:

```bash
AGENT_TIMEOUT=5 python3 examples/e2e-local/orchestrator.py "Build a calculator"
```

Output:

```
  SUPERVISOR: [PROBE] Agent timed out after 5s (attempt 1)
  SUPERVISOR: [RESTART] Agent stalled again after probe. Simplifying prompt.
```

On Gitea, Issue #1 shows:
- ⚠️ Supervisor probe comment
- 🔄 Supervisor restart comment
- caloron_feedback YAML with `self_assessment: "failed"`

## Generated Code

The agents produce real, working Python:

```python
# src/palindrome.py
import re

def is_palindrome(s: str) -> bool:
    cleaned = re.sub(r'[^a-zA-Z0-9]', '', s).lower()
    return cleaned == cleaned[::-1]
```

```python
# tests/test_palindrome.py
@pytest.mark.parametrize("s", ["racecar", "A man, a plan, a canal: Panama"])
def test_valid_palindromes(s):
    assert is_palindrome(s) is True
```

Tests pass: 25/25 (validators), 12/12 (string utils), 8/8 (palindrome).
