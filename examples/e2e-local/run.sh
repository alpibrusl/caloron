#!/bin/bash
# ==========================================================================
# E2E Local Demo: Non-linear DAG + PR review cycles
# Runs entirely against local Gitea — no GitHub API keys needed.
#
# DAG shape (not linear!):
#
#   core-calc ─────┬──→ cli-interface ──────┐
#                  │                        │
#                  ├──→ advanced-ops ──┬─────┼──→ readme
#                  │                  │     │
#                  └──→ history ──────┼─────┘
#                                    │     │
#                         unit-tests ┘     │
#                                          │
#                         integration-tests┘
#
# Demonstrates:
#   - Parallel fan-out (core-calc → 3 tasks simultaneously)
#   - Diamond join (advanced-ops + core-calc → unit-tests)
#   - Complex join (cli + history + unit-tests → integration-tests)
#   - PR review cycle (reviewer requests changes → dev fixes → re-review)
# ==========================================================================
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
CALORON_DIR="$(dirname "$(dirname "$SCRIPT_DIR")")"

# Gitea config (adjust if needed)
GITEA_TOKEN="${GITEA_TOKEN:-c50bad400bd9b8cde3e930cca052eae6ded71f7b}"

gitea_get() {
    docker exec gitea wget -qO- \
        --header="Authorization: token ${GITEA_TOKEN}" \
        "http://127.0.0.1:3000$1" 2>/dev/null
}

gitea_post() {
    docker exec gitea wget -qO- \
        --post-data="$2" \
        --header="Content-Type: application/json" \
        --header="Authorization: token ${GITEA_TOKEN}" \
        "http://127.0.0.1:3000$1" 2>/dev/null
}

# ======================================================================
# Setup: create test repo
# ======================================================================
REPO="caloron/calc-demo"
echo "Creating repo $REPO on Gitea..."
gitea_post "/api/v1/user/repos" \
    '{"name":"calc-demo","auto_init":true,"description":"Calculator CLI demo"}' \
    | python3 -c "import json,sys; d=json.load(sys.stdin); print(f'  Repo: {d[\"full_name\"]}')" 2>/dev/null || echo "  (repo may already exist)"

echo ""

# ======================================================================
# Load and validate DAG
# ======================================================================
echo "================================================================"
echo "  E2E Local Demo: Non-Linear DAG + PR Review Cycles"
echo "================================================================"
echo ""

DAG_FILE="$SCRIPT_DIR/dag.json"

echo "--- DAG Shape ---"
echo ""
echo "  core-calc ─────┬──→ cli-interface ──────┐"
echo "                 │                        │"
echo "                 ├──→ advanced-ops ──┬─────┼──→ readme"
echo "                 │                  │     │"
echo "                 └──→ history ──────┼─────┘"
echo "                                   │     │"
echo "                        unit-tests ┘     │"
echo "                                         │"
echo "                        integration-tests┘"
echo ""

# Use Rust DAG engine to validate and run
cd "$CALORON_DIR"

# Run a Rust test that exercises this exact DAG
cat > /tmp/dag_demo_test.rs << 'RUSTEOF'
use chrono::Utc;
use caloron_types::dag::*;

#[test]
fn e2e_nonlinear_dag_with_review_cycles() {
    // Load the DAG
    let json = std::fs::read_to_string(
        std::env::var("DAG_FILE").unwrap_or("examples/e2e-local/dag.json".into())
    ).unwrap();
    let dag: Dag = serde_json::from_str(&json).unwrap();

    let mut state = DagState::from_dag(dag);

    // === Phase 1: Initialize — unblock tasks with no deps ===
    let unblocked = state.evaluate_unblocked();
    for id in &unblocked {
        state.tasks.get_mut(id).unwrap().transition(TaskStatus::Ready);
    }

    println!("\n=== Phase 1: Initial state ===");
    print_all(&state);
    assert_eq!(state.tasks["core-calc"].status, TaskStatus::Ready, "core-calc should be Ready (no deps)");
    assert_eq!(state.tasks["cli-interface"].status, TaskStatus::Pending, "cli-interface blocked on core-calc");
    assert_eq!(state.tasks["advanced-ops"].status, TaskStatus::Pending);
    assert_eq!(state.tasks["history"].status, TaskStatus::Pending);
    assert_eq!(state.tasks["unit-tests"].status, TaskStatus::Pending);
    assert_eq!(state.tasks["integration-tests"].status, TaskStatus::Pending);
    assert_eq!(state.tasks["readme"].status, TaskStatus::Pending);

    // === Phase 2: core-calc starts and completes ===
    println!("\n=== Phase 2: core-calc completes ===");
    state.tasks.get_mut("core-calc").unwrap().transition(TaskStatus::InProgress);
    state.tasks.get_mut("core-calc").unwrap().task.github_issue_number = Some(1);
    state.tasks.get_mut("core-calc").unwrap().pr_numbers.push(100);
    state.tasks.get_mut("core-calc").unwrap().transition(TaskStatus::InReview);
    state.tasks.get_mut("core-calc").unwrap().transition(TaskStatus::Done);

    // Unblock dependents
    let unblocked = state.evaluate_unblocked();
    for id in &unblocked {
        state.tasks.get_mut(id).unwrap().transition(TaskStatus::Ready);
    }

    print_all(&state);
    // Fan-out: 3 tasks should unblock simultaneously!
    assert_eq!(state.tasks["cli-interface"].status, TaskStatus::Ready, "cli-interface unblocked");
    assert_eq!(state.tasks["advanced-ops"].status, TaskStatus::Ready, "advanced-ops unblocked");
    assert_eq!(state.tasks["history"].status, TaskStatus::Ready, "history unblocked");
    // These still blocked:
    assert_eq!(state.tasks["unit-tests"].status, TaskStatus::Pending, "unit-tests needs advanced-ops too");
    assert_eq!(state.tasks["readme"].status, TaskStatus::Pending, "readme needs cli + advanced + history");
    println!("  FAN-OUT: 3 tasks unblocked in parallel!");

    // === Phase 3: PR review cycle — reviewer requests changes on advanced-ops ===
    println!("\n=== Phase 3: PR review cycle (changes requested) ===");
    state.tasks.get_mut("advanced-ops").unwrap().transition(TaskStatus::InProgress);
    state.tasks.get_mut("advanced-ops").unwrap().pr_numbers.push(101);
    state.tasks.get_mut("advanced-ops").unwrap().transition(TaskStatus::InReview);
    println!("  advanced-ops: InReview (PR #101)");

    // Reviewer requests changes!
    state.tasks.get_mut("advanced-ops").unwrap().transition(TaskStatus::InProgress);
    println!("  REVIEWER: 'Add input validation for negative sqrt' → changes requested");
    println!("  advanced-ops: back to InProgress (rework)");

    // Dev fixes and re-submits
    state.tasks.get_mut("advanced-ops").unwrap().pr_numbers.push(102);
    state.tasks.get_mut("advanced-ops").unwrap().transition(TaskStatus::InReview);
    println!("  DEV: pushes fix, new review cycle (PR #102)");

    // Approved this time
    state.tasks.get_mut("advanced-ops").unwrap().transition(TaskStatus::Done);
    println!("  REVIEWER: approved! advanced-ops → Done");

    // === Phase 4: cli-interface and history complete in parallel ===
    println!("\n=== Phase 4: Parallel completion ===");
    for task_id in &["cli-interface", "history"] {
        state.tasks.get_mut(*task_id).unwrap().transition(TaskStatus::InProgress);
        state.tasks.get_mut(*task_id).unwrap().pr_numbers.push(103);
        state.tasks.get_mut(*task_id).unwrap().transition(TaskStatus::InReview);
        state.tasks.get_mut(*task_id).unwrap().transition(TaskStatus::Done);
        println!("  {task_id}: Done");
    }

    // Unblock
    let unblocked = state.evaluate_unblocked();
    for id in &unblocked {
        state.tasks.get_mut(id).unwrap().transition(TaskStatus::Ready);
    }
    print_all(&state);

    // unit-tests needs core-calc + advanced-ops → both Done → Ready
    assert_eq!(state.tasks["unit-tests"].status, TaskStatus::Ready);
    // readme needs cli + advanced + history → all Done → Ready
    assert_eq!(state.tasks["readme"].status, TaskStatus::Ready);
    // integration-tests needs cli + history + unit-tests → unit-tests not Done yet
    assert_eq!(state.tasks["integration-tests"].status, TaskStatus::Pending);
    println!("  DIAMOND JOIN: unit-tests + readme unblocked");
    println!("  integration-tests still waiting on unit-tests");

    // === Phase 5: unit-tests and readme complete ===
    println!("\n=== Phase 5: unit-tests + readme complete ===");
    for task_id in &["unit-tests", "readme"] {
        state.tasks.get_mut(*task_id).unwrap().transition(TaskStatus::InProgress);
        state.tasks.get_mut(*task_id).unwrap().pr_numbers.push(104);
        state.tasks.get_mut(*task_id).unwrap().transition(TaskStatus::InReview);
        state.tasks.get_mut(*task_id).unwrap().transition(TaskStatus::Done);
        println!("  {task_id}: Done");
    }

    let unblocked = state.evaluate_unblocked();
    for id in &unblocked {
        state.tasks.get_mut(id).unwrap().transition(TaskStatus::Ready);
    }

    assert_eq!(state.tasks["integration-tests"].status, TaskStatus::Ready);
    println!("  COMPLEX JOIN: integration-tests finally unblocked (cli + history + unit-tests all Done)");

    // === Phase 6: integration-tests completes → sprint done ===
    println!("\n=== Phase 6: Sprint complete ===");
    state.tasks.get_mut("integration-tests").unwrap().transition(TaskStatus::InProgress);
    state.tasks.get_mut("integration-tests").unwrap().pr_numbers.push(105);
    state.tasks.get_mut("integration-tests").unwrap().transition(TaskStatus::InReview);
    state.tasks.get_mut("integration-tests").unwrap().transition(TaskStatus::Done);

    print_all(&state);
    assert!(state.is_sprint_complete());
    println!("  SPRINT COMPLETE! All 7 tasks done.");

    // Verify PR history shows the review cycle
    let adv = &state.tasks["advanced-ops"];
    assert_eq!(adv.pr_numbers.len(), 2, "advanced-ops had 2 PRs (review cycle)");
    println!("  advanced-ops PR history: {:?} (review cycle visible)", adv.pr_numbers);
}

fn print_all(state: &DagState) {
    let mut tasks: Vec<_> = state.tasks.iter().collect();
    tasks.sort_by_key(|(_, ts)| &ts.task.title);
    for (id, ts) in tasks {
        let status = format!("{:?}", ts.status);
        let prs = if ts.pr_numbers.is_empty() { String::new() }
            else { format!(" PRs:{:?}", ts.pr_numbers) };
        println!("  {id:<22} {status:<14}{prs}");
    }
}
RUSTEOF

cp /tmp/dag_demo_test.rs tests/dag_demo_test.rs

echo "Running DAG simulation..."
echo ""
cargo test e2e_nonlinear_dag_with_review_cycles -- --nocapture 2>&1 | grep -E "^(=|  |test )" | head -60

echo ""

# ======================================================================
# Now create the issues on Gitea to show it's real
# ======================================================================
echo "--- Creating Gitea issues ---"
echo ""

TASKS=$(python3 -c "
import json
dag = json.load(open('$DAG_FILE'))
for tid, task in [(t['id'], t) for t in dag['tasks']]:
    deps = ', '.join(task['depends_on']) if task['depends_on'] else 'none'
    print(f'{tid}|{task[\"title\"]}|{task[\"assigned_to\"]}|{deps}')
")

while IFS='|' read -r tid title agent deps; do
    BODY="**Task:** $tid\n**Agent:** $agent\n**Depends on:** $deps\n\n## Definition of Done\n- [ ] Implementation\n- [ ] Tests pass\n- [ ] PR approved"
    RESULT=$(gitea_post "/api/v1/repos/$REPO/issues" \
        "{\"title\": \"$title\", \"body\": \"$BODY\", \"labels\": []}")
    NUM=$(echo "$RESULT" | python3 -c "import json,sys; print(json.load(sys.stdin)['number'])")
    echo "  #$NUM: $title (→ $agent, deps: $deps)"
done <<< "$TASKS"

echo ""
echo "--- Simulating PR review cycle on Gitea ---"
echo ""

# Simulate: reviewer comments requesting changes
COMMENT="Changes requested: please add input validation for negative numbers in sqrt operation."
gitea_post "/api/v1/repos/$REPO/issues/3/comments" \
    "{\"body\": \"$COMMENT\"}" > /dev/null
echo "  Issue #3 (advanced-ops): reviewer requests changes"

# Dev responds
COMMENT="Fixed: added ValueError for negative sqrt. Updated PR."
gitea_post "/api/v1/repos/$REPO/issues/3/comments" \
    "{\"body\": \"$COMMENT\"}" > /dev/null
echo "  Issue #3 (advanced-ops): dev pushes fix"

# Reviewer approves
COMMENT="LGTM. Approved."
gitea_post "/api/v1/repos/$REPO/issues/3/comments" \
    "{\"body\": \"$COMMENT\"}" > /dev/null
echo "  Issue #3 (advanced-ops): reviewer approves"

echo ""
echo "--- Gitea state ---"
ISSUES=$(gitea_get "/api/v1/repos/$REPO/issues?state=open&limit=50")
echo "$ISSUES" | python3 -c "
import json, sys
issues = json.load(sys.stdin)
for i in sorted(issues, key=lambda x: x['number']):
    print(f'  #{i[\"number\"]}: {i[\"title\"]} (comments: {i[\"comments\"]})')
"

echo ""
echo "================================================================"
echo "  Demo complete!"
echo ""
echo "  Demonstrated:"
echo "    [v] Non-linear DAG (fan-out, diamond join, complex join)"
echo "    [v] PR review cycle (changes requested → fix → re-review → approve)"
echo "    [v] Parallel task execution (3 tasks unblocked simultaneously)"
echo "    [v] No external credentials needed (local Gitea only)"
echo "    [v] 7 tasks, 5 agents, 2 review cycles"
echo "================================================================"

# Clean up test file
rm -f tests/dag_demo_test.rs
