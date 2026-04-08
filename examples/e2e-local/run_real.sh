#!/bin/bash
# ==========================================================================
# REAL E2E Sprint: PO Agent generates DAG, Claude agents write code
#
# Flow:
#   1. PO Agent (Claude) analyzes the goal and generates the DAG
#   2. For each task (respecting dependency order), an agent (Claude) writes code
#   3. Tests are run to verify the output
#
# Uses your Claude Pro subscription. No API key needed.
# ==========================================================================
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
WORK_DIR="/tmp/caloron-real-sprint-$(date +%s)"
GOAL="${1:-Build a Python CLI calculator with add, subtract, multiply, divide, power, sqrt operations. Include argparse CLI and pytest tests.}"

echo "================================================================"
echo "  REAL Sprint: PO + Agent pipeline"
echo "  Goal: $GOAL"
echo "================================================================"
echo ""

# ======================================================================
# Setup: create project
# ======================================================================
mkdir -p "$WORK_DIR/project"
cd "$WORK_DIR/project"
git init -q
mkdir -p src tests

cat > pyproject.toml << 'PYEOF'
[project]
name = "calc"
version = "0.1.0"
requires-python = ">=3.11"
PYEOF

cat > src/__init__.py << 'PYEOF'
"""Project built by Caloron agents."""
PYEOF

cat > tests/__init__.py << 'PYEOF'
PYEOF

git add -A && git commit -q -m "Initial project structure"
echo "Project created at: $WORK_DIR/project"
echo ""

# ======================================================================
# Step 1: PO Agent generates the DAG
# ======================================================================
echo "================================================================"
echo "  Step 1: PO Agent generates the sprint plan"
echo "================================================================"
echo ""

PO_PROMPT="You are a Product Owner planning a software sprint.

## Goal
$GOAL

## Current project
A Python project with:
- src/ directory (empty, has __init__.py)
- tests/ directory (empty)
- pyproject.toml

## Your job
Output a JSON array of tasks. Each task has:
- id: short identifier (e.g. \"core-logic\")
- title: one-line description
- depends_on: array of task IDs that must complete first ([] if none)
- agent_prompt: the full instructions for the developer agent working on this task

Rules:
- Keep it to 3-5 tasks
- Respect dependency order (tests depend on implementation)
- Each agent_prompt must be specific: which files to create, what functions, what behavior
- Agents can only create/modify files, not run network commands
- Output ONLY the JSON array, no other text

Example format:
[
  {\"id\": \"core\", \"title\": \"Implement core module\", \"depends_on\": [], \"agent_prompt\": \"Create src/core.py with...\"},
  {\"id\": \"tests\", \"title\": \"Write tests\", \"depends_on\": [\"core\"], \"agent_prompt\": \"Create tests/test_core.py with...\"}
]"

echo "Asking PO Agent to plan the sprint..."
PO_START=$(date +%s)

DAG_JSON=$(claude -p "$PO_PROMPT" --dangerously-skip-permissions 2>/dev/null)
PO_END=$(date +%s)
PO_TIME=$((PO_END - PO_START))

# Extract JSON array from the output (PO may include markdown fences)
DAG_JSON=$(echo "$DAG_JSON" | python3 -c "
import sys, json, re
text = sys.stdin.read()
# Try to find JSON array in the text
match = re.search(r'\[.*\]', text, re.DOTALL)
if match:
    arr = json.loads(match.group())
    print(json.dumps(arr, indent=2))
else:
    print('[]')
    print('ERROR: PO did not produce valid JSON', file=sys.stderr)
")

TASK_COUNT=$(echo "$DAG_JSON" | python3 -c "import json,sys; print(len(json.load(sys.stdin)))")

echo ""
echo "PO Agent generated $TASK_COUNT tasks in ${PO_TIME}s:"
echo ""
echo "$DAG_JSON" | python3 -c "
import json, sys
tasks = json.load(sys.stdin)
for t in tasks:
    deps = ', '.join(t.get('depends_on', [])) or 'none'
    print(f'  {t[\"id\"]:<20} {t[\"title\"]}')
    print(f'  {\"\":<20} depends on: {deps}')
    print()
"

if [ "$TASK_COUNT" = "0" ]; then
    echo "ERROR: PO produced no tasks. Aborting."
    exit 1
fi

# Save DAG for reference
echo "$DAG_JSON" > "$WORK_DIR/dag.json"
echo "DAG saved to: $WORK_DIR/dag.json"
echo ""

# ======================================================================
# Step 2: Execute tasks in dependency order
# ======================================================================
echo "================================================================"
echo "  Step 2: Executing tasks (agents write code)"
echo "================================================================"
echo ""

# Topological sort and execute
python3 << PYEOF
import json, sys, subprocess, time, os

os.chdir("$WORK_DIR/project")

with open("$WORK_DIR/dag.json") as f:
    tasks = json.load(f)

task_map = {t["id"]: t for t in tasks}
completed = set()
total_time = 0

def deps_satisfied(task):
    return all(d in completed for d in task.get("depends_on", []))

# Simple topological execution
remaining = list(tasks)
iteration = 0

while remaining:
    iteration += 1
    if iteration > len(tasks) + 1:
        print("ERROR: Circular dependency or unsatisfiable deps!")
        for t in remaining:
            print(f"  stuck: {t['id']} (needs: {t.get('depends_on', [])})")
        sys.exit(1)

    # Find tasks whose deps are all done
    ready = [t for t in remaining if deps_satisfied(t)]
    if not ready:
        print(f"  Waiting... completed so far: {completed}")
        continue

    for task in ready:
        tid = task["id"]
        title = task["title"]
        prompt = task.get("agent_prompt", task["title"])

        print(f"--- Task: {tid} — {title} ---")
        print(f"  Dependencies: {task.get('depends_on', []) or 'none'}")
        print(f"  Running Claude Code...")

        # Build agent prompt with context
        full_prompt = f"""You are working in a Python project at the current directory.

{prompt}

## Rules
- Only create or modify files in src/ and tests/ directories
- Use type hints
- Do not install packages or run network commands
- When done, just stop — do not commit
"""

        start = time.time()
        result = subprocess.run(
            ["claude", "-p", full_prompt, "--dangerously-skip-permissions"],
            capture_output=True, text=True, cwd="$WORK_DIR/project"
        )
        elapsed = time.time() - start
        total_time += elapsed

        # Show last few lines of output
        output_lines = (result.stdout or "").strip().split("\n")
        for line in output_lines[-5:]:
            print(f"    {line}")

        # Commit
        subprocess.run(["git", "add", "-A"], cwd="$WORK_DIR/project", capture_output=True)
        diff = subprocess.run(["git", "diff", "--cached", "--quiet"], cwd="$WORK_DIR/project")
        if diff.returncode != 0:
            subprocess.run(
                ["git", "commit", "-m", f"[{tid}] {title}"],
                cwd="$WORK_DIR/project", capture_output=True
            )
            # Show what was created/changed
            stat = subprocess.run(
                ["git", "diff", "--stat", "HEAD~1"],
                cwd="$WORK_DIR/project", capture_output=True, text=True
            )
            print(f"  Committed in {elapsed:.0f}s:")
            for line in stat.stdout.strip().split("\n"):
                print(f"    {line}")
        else:
            print(f"  WARNING: No changes produced ({elapsed:.0f}s)")

        completed.add(tid)
        remaining.remove(task)
        print()

print(f"All {len(tasks)} tasks completed in {total_time:.0f}s total")
PYEOF

echo ""

# ======================================================================
# Step 3: Verify — run tests
# ======================================================================
echo "================================================================"
echo "  Step 3: Verification"
echo "================================================================"
echo ""

cd "$WORK_DIR/project"

echo "Project files:"
find src tests -name "*.py" 2>/dev/null | sort
echo ""

echo "Git log:"
git log --oneline
echo ""

echo "Running tests..."
echo ""
python3 -m pytest tests/ -v 2>&1 || echo ""
echo ""

# Show the actual generated code
echo "================================================================"
echo "  Generated code"
echo "================================================================"
echo ""
for f in $(find src -name "*.py" ! -name "__init__.py" | sort); do
    echo "--- $f ---"
    cat "$f"
    echo ""
done

echo "================================================================"
echo "  Sprint complete!"
echo "  Work directory: $WORK_DIR/project"
echo "================================================================"
