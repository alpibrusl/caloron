# End-to-End Sprint Example

This walkthrough runs a complete sprint lifecycle locally, from DAG creation through execution to retro.

## Scenario

We have a small web API project and want to add two features:

1. A `/health` endpoint (simple, no dependencies)
2. A `/metrics` endpoint (depends on health endpoint being done first)

Two agents work on these tasks, with one reviewer.

## Step 1: Agent Definitions

### `agents/api-developer.yaml`

```yaml
name: api-developer
version: "1.0"
description: "Implements API endpoints"

llm:
  model: default
  max_tokens: 8192
  temperature: 0.2

system_prompt: |
  You are an API developer. You receive tasks as GitHub issues.
  Read the issue, implement the endpoint, write tests, and open a PR.

tools:
  - github_mcp
  - bash

nix:
  packages:
    - nodejs_20
    - python311
  env:
    NODE_ENV: test

credentials:
  - GITHUB_TOKEN
  - ANTHROPIC_API_KEY

stall_threshold_minutes: 15
max_review_cycles: 3
```

### `agents/code-reviewer.yaml`

```yaml
name: code-reviewer
version: "1.0"
description: "Reviews PRs for quality"

llm:
  model: strong
  temperature: 0.1

system_prompt: |
  You are a code reviewer. Review PRs for correctness and test coverage.
  Approve if good, request changes with specific feedback if not.

tools:
  - github_mcp
  - bash

nix:
  packages:
    - nodejs_20

credentials:
  - GITHUB_TOKEN
  - ANTHROPIC_API_KEY

stall_threshold_minutes: 30
max_review_cycles: 3
```

## Step 2: DAG Definition

### `dag.json`

```json
{
  "sprint": {
    "id": "sprint-e2e-demo",
    "goal": "Add health and metrics endpoints",
    "start": "2026-04-08T10:00:00Z",
    "max_duration_hours": 4
  },
  "agents": [
    {
      "id": "dev-1",
      "role": "api-developer",
      "definition_path": "agents/api-developer.yaml"
    },
    {
      "id": "dev-2",
      "role": "api-developer",
      "definition_path": "agents/api-developer.yaml"
    },
    {
      "id": "rev-1",
      "role": "code-reviewer",
      "definition_path": "agents/code-reviewer.yaml"
    }
  ],
  "tasks": [
    {
      "id": "health-endpoint",
      "title": "Implement /health endpoint",
      "assigned_to": "dev-1",
      "issue_template": "tasks/endpoint.md",
      "depends_on": [],
      "reviewed_by": "rev-1"
    },
    {
      "id": "metrics-endpoint",
      "title": "Implement /metrics endpoint",
      "assigned_to": "dev-2",
      "issue_template": "tasks/endpoint.md",
      "depends_on": ["health-endpoint"],
      "reviewed_by": "rev-1"
    }
  ],
  "review_policy": {
    "required_approvals": 1,
    "auto_merge": true,
    "max_review_cycles": 3
  },
  "escalation": {
    "stall_threshold_minutes": 15,
    "supervisor_id": "supervisor",
    "human_contact": "github_issue"
  }
}
```

## Step 3: Validate

```bash
$ caloron agent validate agents/api-developer.yaml
Agent: api-developer v1.0
Model: default
Tools: github_mcp, bash
Validation: PASSED

$ caloron agent validate agents/code-reviewer.yaml
Agent: code-reviewer v1.0
Model: strong
Tools: github_mcp, bash
Validation: PASSED
```

## Step 4: Start the Sprint

```bash
$ caloron start --dag dag.json
```

What happens:

1. DAG is loaded and validated (no cycles, all references valid)
2. `health-endpoint` has no dependencies → transitions to **Ready**
3. `metrics-endpoint` depends on `health-endpoint` → stays **Pending**
4. Orchestrator creates GitHub issue for `health-endpoint`
5. Agent `dev-1` is spawned in a Nix environment

## Step 5: Execution Flow

```
Time 0:00  health-endpoint → READY → issue created → dev-1 spawned
Time 0:01  dev-1 reads issue, starts implementing
Time 0:10  dev-1 opens PR #1 → health-endpoint → IN_REVIEW
Time 0:10  rev-1 assigned to PR #1
Time 0:12  rev-1 approves PR #1 → auto-merge
Time 0:12  PR #1 merged → health-endpoint → DONE
Time 0:12  metrics-endpoint dependencies satisfied → READY
Time 0:12  Issue created → dev-2 spawned
Time 0:13  dev-2 reads issue, starts implementing
Time 0:20  dev-2 opens PR #2 → metrics-endpoint → IN_REVIEW
Time 0:22  rev-1 requests changes on PR #2
Time 0:25  dev-2 pushes fixes, re-requests review
Time 0:27  rev-1 approves PR #2 → auto-merge
Time 0:27  PR #2 merged → metrics-endpoint → DONE
Time 0:27  Sprint complete!
```

## Step 6: Check Status

```bash
$ caloron status

Sprint: sprint-e2e-demo
Goal: Add health and metrics endpoints

Tasks:
  [v] health-endpoint  Implement /health endpoint       DONE         (dev-1)
  [v] metrics-endpoint Implement /metrics endpoint      DONE         (dev-2)

Sprint: COMPLETE
```

## Step 7: Run Retro

```bash
$ caloron retro
```

Output:

```markdown
# Sprint Retro — sprint-e2e-demo

## Summary
- Tasks completed: 2/2
- Average task clarity: 7.5/10
- Total tokens consumed: 22000
- Supervisor interventions: 0

## What Worked Well
- health-endpoint completed successfully (clarity: 8/10, 10000 tokens, 10min)
- metrics-endpoint completed successfully (clarity: 7/10, 12000 tokens, 15min)
```

## Running It Programmatically

The same flow can be driven entirely through the Rust API. Here's the core lifecycle as a test:

```rust
use caloron_types::dag::*;
use caloron_types::git::{GitEvent, ReviewState, labels};
use chrono::Utc;

// 1. Create DAG
let dag = Dag { /* ... as above ... */ };
let mut engine = DagEngine::from_dag(dag).unwrap();

// 2. health-endpoint is Ready (no deps), metrics-endpoint is Pending
assert_eq!(engine.state().tasks["health-endpoint"].status, TaskStatus::Ready);
assert_eq!(engine.state().tasks["metrics-endpoint"].status, TaskStatus::Pending);

// 3. Simulate: issue opened → agent starts
engine.task_started("health-endpoint", 10).unwrap();

// 4. Simulate: PR opened → in review
engine.task_in_review("health-endpoint", 100).unwrap();

// 5. Simulate: PR merged → task done, metrics-endpoint unblocked
let unblocked = engine.task_completed("health-endpoint").unwrap();
assert_eq!(unblocked, vec!["metrics-endpoint"]);
assert_eq!(engine.state().tasks["metrics-endpoint"].status, TaskStatus::Ready);

// 6. Simulate: metrics-endpoint lifecycle
engine.task_started("metrics-endpoint", 11).unwrap();
engine.task_in_review("metrics-endpoint", 101).unwrap();
engine.task_completed("metrics-endpoint").unwrap();

// 7. Sprint complete
assert!(engine.is_sprint_complete());
```

See `tests/integration/e2e_sprint.rs` for the full runnable version.
