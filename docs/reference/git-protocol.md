# Git Protocol

Git is the communication bus between agents. This page defines the semantics of each event and the orchestrator's response.

## Event Handlers

### `issue.opened`

When a new issue with `caloron:task` label is created:

1. Find a Ready task in the DAG without an assigned issue
2. Transition task to InProgress
3. Add `caloron:assigned` label
4. Comment: `@caloron-agent-{id} has been assigned this task`
5. Spawn the assigned agent

### `pr.opened`

When an agent opens a pull request with `Closes #N`:

1. Find the task linked to issue #N
2. Transition task from InProgress to InReview
3. Add `caloron:review-pending` label
4. Assign the reviewer agent from the DAG

### `pr.review_submitted` (approved)

1. Trigger auto-merge (if configured)

### `pr.review_submitted` (changes_requested)

1. Remove `caloron:review-pending` label
2. Add `caloron:changes-requested` label
3. Notify the author agent
4. Increment review cycle counter

### `pr.merged` (Canonical Completion Signal)

This is the single source of truth for task completion:

1. Transition task to **Done** in DAG state
2. Add `caloron:done` label to linked issue
3. Close the linked issue: "Completed via PR #N"
4. Evaluate all Pending tasks for newly unblocked dependencies
5. Spawn agents for newly Ready tasks

### `pr.closed` (without merge)

1. Transition task from InReview back to **InProgress**
2. Comment: `@caloron-agent-{author}: please review the closure reason and rework`
3. If this is the 2nd+ PR closure for the same task, notify supervisor

### `comment.created`

Two patterns are recognized:

**Feedback comment** — YAML block starting with `caloron_feedback:`:

- Parsed and stored in the retro buffer
- No agent action triggered

**Agent mention** — `@caloron-agent-{id}`:

- The mentioned agent is notified
- Stall timer is reset

### `push`

Push events reset the stall timer for the pushing agent. No DAG transition.

## Idempotency

All event handlers are idempotent. Processing the same event twice produces the same result. The Git Monitor deduplicates events by tracking processed event IDs.
