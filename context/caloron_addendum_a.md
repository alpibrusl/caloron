# Caloron Developer Docs v2.0 — Addendum A

## Architecture Review Findings

> This addendum corrects errata, adds missing failure handling, and proposes design refinements to the Caloron v2 developer documentation. Each item references the original document's section numbers for cross-reference.

---

## 1. Errata

These items are factually wrong or internally contradictory in the original document.

### E1: Nix Shell Form Inconsistency

**Affected sections:** 8.2 (Agent Environment as Nix Derivation), 12.1 (Repository Structure)

**Problem:** The repository structure declares `flake.nix` (line 1081) for reproducible builds, but the generated agent environments in Section 8.2 use `pkgs.mkShell` with `import <nixpkgs> {}` — the channel-based `nix-shell` approach. This mixes two incompatible Nix paradigms. Additionally, `buildInputs` is used where `nativeBuildInputs` is correct for `mkShell` in modern Nix (post-23.05).

**Correction:** All environments must use `nix develop` with flake-based devShells. The generated expression should be a flake devShell, not a standalone `shell.nix`.

Replace Section 8.2 example with:

```nix
# Generated from agents/backend-developer.yaml
# Part of the Caloron flake's devShells output
{
  devShells.x86_64-linux.agent-backend-developer = pkgs.mkShell {
    name = "caloron-agent-backend-developer";

    nativeBuildInputs = with pkgs; [
      nodejs_20
      python311
      rustc
      cargo
      git
      caloron-harness
      claude-code
    ];

    shellHook = ''
      export CALORON_AGENT_ROLE="backend-developer"
      export CALORON_DAEMON_SOCKET="${daemonSocket}"
      export CALORON_WORKTREE="${worktreePath}"
      export CALORON_TASK_ID="${taskId}"
      # Secrets injected via file, not env vars — see R3
      export CALORON_SECRETS_FILE="${secretsFilePath}"
    '';
  };
}
```

Replace Section 8.4 commands:

```
# Step 4: Build Nix environment (was: nix-shell --pure)
nix develop .#agent-backend-developer --command echo ready

# Step 7: Start harness process (was: nix-shell ... --run)
nix develop .#agent-backend-developer --command caloron-harness start
```

---

### E2: Ambiguous Task Completion Signal

**Affected sections:** 4.1 (Event Types and Handlers), 4.3 (Label Taxonomy), 6.2 (DAG State Machine), 16.3 (Common Debugging Scenarios)

**Problem:** Three different signals are used for task completion across the document:
- Section 4.1 `pr.merged` handler: "Mark task as `completed` in DAG state"
- Section 4.3 label table: `caloron:done` "Task is complete" — set by Orchestrator
- Section 16.3 debugging tip: "the PR was merged but the linked issue was not closed, so the DAG did not receive the `issue.closed` event" — implies `issue.closed` is the canonical signal

**Correction:** `pr.merged` is the single canonical completion trigger. The orchestrator then performs a completion chain. Redefine the `pr.merged` handler:

```
#### `pr.merged`
Orchestrator action (atomic completion chain):
1. Transition task to `DONE` in DAG state
2. Add `caloron:done` label to the linked issue
3. Close the linked issue with comment: "Completed via PR #{pr_number}"
4. Record completion time and token cost from the feedback comment
5. Evaluate all PENDING tasks for newly unblocked dependencies
6. For each newly unblocked task, transition to READY and create GitHub issue
```

The issue is closed **by the orchestrator**, not by the agent. The debugging tip in Section 16.3 should read:

> **DAG not unblocking tasks after dependency completion:** If the issue was not closed, verify the `pr.merged` event was received by the Git Monitor. The most common cause: the PR was merged outside Caloron (manual merge) and the polling loop missed the event window. Check `caloron trace <task-id>` for the event timeline.

---

### E3: Missing `pr.closed` Handler

**Affected sections:** 4.1 (Event Types and Handlers), 13.1 (Core Rust Types — GitEvent enum)

**Problem:** No handler exists for a PR that is closed without being merged. If a reviewer closes a PR (e.g., the approach is fundamentally wrong), the task remains stuck in `IN_REVIEW` forever.

**Correction:** Add `PrClosed` to the `GitEvent` enum:

```rust
pub enum GitEvent {
    // ... existing variants ...
    PrClosed { number: u64, closer: String, linked_issue: Option<u64> },
}
```

Add handler to Section 4.1:

```
#### `pr.closed` (without merge)
Orchestrator action:
1. Look up linked issue and task in DAG
2. Remove `caloron:review-pending` and `caloron:changes-requested` labels
3. Transition task from IN_REVIEW back to IN_PROGRESS
4. Post comment on linked issue: "PR #{pr_number} was closed without merge.
   @caloron-agent-{author-id}: please review the closure reason and rework."
5. Reset stall timer for the author agent
6. If this is the 2nd PR closure for the same task, notify Supervisor
   for potential task reassignment or escalation
```

---

## 2. Hardening Requirements

These are gaps where the docs describe the happy path but not what happens when things go wrong.

### H1: Supervisor Self-Monitoring

**Affected section:** 7 (The Supervisor Agent)

**Problem:** The Supervisor is described as "the most critical component" but has no self-monitoring mechanism. If the Supervisor process crashes or stalls, no component detects or recovers from it.

**Requirement:** The Caloron daemon must implement a Supervisor watchdog at the daemon level (not within the Supervisor itself):

```rust
// src/daemon/supervisor_watchdog.rs

pub struct SupervisorWatchdog {
    last_heartbeat: DateTime<Utc>,
    heartbeat_interval: Duration,       // expected: 60 seconds
    max_missed_heartbeats: u32,         // default: 2
    restart_count: u32,
    max_restarts: u32,                  // default: 3
}

impl SupervisorWatchdog {
    pub fn check(&self) -> WatchdogVerdict {
        let now = Utc::now();
        let missed = (now - self.last_heartbeat).num_seconds()
            / self.heartbeat_interval.num_seconds();

        if missed as u32 > self.max_missed_heartbeats {
            if self.restart_count >= self.max_restarts {
                return WatchdogVerdict::EscalateToHuman;
            }
            return WatchdogVerdict::RestartSupervisor;
        }
        WatchdogVerdict::Healthy
    }
}

pub enum WatchdogVerdict {
    Healthy,
    RestartSupervisor,
    EscalateToHuman,  // daemon creates escalation issue directly
}
```

When `EscalateToHuman` triggers, the daemon bypasses the Supervisor's escalation gateway and creates a GitHub issue directly:

```
CRITICAL: Supervisor process unresponsive

The Supervisor agent has failed to produce a heartbeat for {N} minutes.
Automatic restart has been attempted {M} times.

Sprint: {sprint_id}
Running agents: {count} (operating without health monitoring)

Immediate action required:
1. Check daemon logs: `caloron logs supervisor`
2. Manually restart: `caloron supervisor restart`
3. If persistent, stop the sprint: `caloron stop`
```

Add `supervisor_watchdog.rs` to the repository structure in Section 12.1 under `src/daemon/`.

---

### H2: Harness Crash Recovery

**Affected sections:** 5.4 (The Harness), 8.5 (Agent Destruction Sequence)

**Problem:** Section 5.4 states the harness "will not exit without" a feedback comment, but OOM kills, SIGKILL, host restarts, and timeouts bypass graceful exit. The feedback enforcement is only reliable for clean shutdowns.

**Requirement:** The daemon must detect harness process death independently and handle the missing feedback case:

```rust
// In Agent Spawner — monitor harness process
fn monitor_harness_process(&self, pid: Pid, task_id: &str, agent_role: &str) {
    // Wait for process exit
    let exit_status = waitpid(pid);

    match exit_status {
        Ok(normal_exit) => {
            // Harness exited cleanly — feedback comment should exist
            // Verify anyway
            if !self.feedback_exists(task_id) {
                tracing::warn!(task_id, agent_role, "Clean exit but no feedback comment");
                self.post_synthetic_feedback(task_id, agent_role, "clean_exit_no_feedback");
            }
        }
        Err(signal) => {
            // Harness was killed (OOM, SIGKILL, etc.)
            tracing::error!(task_id, agent_role, ?signal, "Harness process killed");
            self.post_synthetic_feedback(task_id, agent_role, "crashed");
            self.health_monitor.report_verdict(
                agent_role,
                HealthVerdict::ProcessDead,
            );
        }
    }
}
```

The synthetic feedback comment format:

```yaml
---
caloron_feedback:
  task_id: "{task_id}"
  agent_role: "{agent_role}"
  task_clarity: 0            # unknown — agent did not self-report
  blockers:
    - "Agent process terminated unexpectedly ({signal})"
  tools_used: []             # unknown
  tokens_consumed: 0         # unknown — check billing separately
  time_to_complete_min: 0    # did not complete
  self_assessment: "crashed"
  notes: "Synthetic feedback generated by daemon. Process exit: {exit_details}"
---
```

Update Section 8.5 (Agent Destruction Sequence) to include the crash path:

```
Crash path (harness process dies unexpectedly):
1. Daemon detects process death via PID monitoring
2. Daemon posts synthetic feedback comment with self_assessment: "crashed"
3. Health Monitor receives ProcessDead verdict
4. Supervisor intervention playbook triggers (restart or escalate)
5. Git worktree is preserved (not removed) for debugging
6. Worktree cleaned up after Supervisor reviews the crash
```

---

### H3: Sprint Cancellation Semantics

**Affected sections:** 2.1 (Sprint), 6.2 (DAG State Machine)

**Problem:** The DAG is immutable per sprint and `CANCELLED` is a valid `TaskStatus`, but there is no defined flow for what happens when a sprint is cancelled. In-flight PRs, partially merged work, and agent state have no defined cleanup path.

**Requirement:** Define the `caloron stop` cancellation flow:

```
Sprint Cancellation Flow (`caloron stop`):

1. Daemon sets sprint state to CANCELLING (prevents new task transitions)

2. For each task in IN_PROGRESS:
   a. Post cancellation comment on the linked issue:
      "Sprint cancelled. Work in progress has been preserved in branch
       agent/{role}/sprint-{id}. Resume in next sprint if needed."
   b. Transition task to CANCELLED { reason: "sprint_cancelled" }
   c. Do NOT close the issue (preserves context for next sprint)

3. For each open PR (any caloron: label):
   a. Add label `caloron:sprint-cancelled`
   b. Do NOT close the PR (preserves partial work and review comments)
   c. Post comment: "This PR's sprint has been cancelled.
      It can be adopted into the next sprint or closed manually."

4. For each running agent:
   a. Send graceful shutdown signal (SIGTERM)
   b. Wait up to 30 seconds for feedback comment
   c. If no feedback, post synthetic feedback (see H2)
   d. Destroy agent (remove Nix env reference, but preserve worktree)

5. For tasks in DONE:
   a. No action — completed work is preserved

6. For tasks in PENDING or READY:
   a. Transition to CANCELLED { reason: "sprint_cancelled" }

7. Run partial retro on completed tasks only
   a. Store retro at caloron-meta/retro/sprint-{id}-partial.md

8. Persist final sprint state to caloron-meta/state/sprint-{id}.json
   with all task statuses and cancellation metadata

9. Git worktrees are preserved (not removed) with a marker file
   .caloron/worktrees/{role}-{sprint-id}/.cancelled
```

---

### H4: Polling Latency and Event Processing

**Affected section:** 13.2 (Configuration — `polling_interval_seconds`)

**Problem:** At 30-second polling intervals, each DAG state transition incurs up to 30 seconds of latency. For a 5-task dependency chain, this compounds to 2.5 minutes of pure wait time. The document defers webhook support to "v2.1" without a concrete plan.

**Requirement:**

**Immediate (Phase 0):** Reduce default polling interval to 5 seconds:

```toml
[github]
polling_interval_seconds = 5    # was: 30
```

**Immediate (Phase 4):** Implement event coalescing — process ALL available events per polling cycle, not one at a time:

```rust
async fn poll_cycle(&mut self) -> Result<()> {
    let events = self.github.list_events_since(self.last_poll_timestamp).await?;

    // Process all events in chronological order within a single cycle
    for event in events {
        self.handle_event(event).await?;
        self.last_poll_timestamp = event.timestamp;
    }

    Ok(())
}
```

**Phase 4 deliverable (not deferred to v2.1):** Implement webhook receiver as the primary event source with polling as fallback:

```rust
pub struct GitMonitor {
    webhook_receiver: Option<WebhookReceiver>,  // primary, if configured
    polling_loop: PollingLoop,                   // fallback, always active
    dedup: EventDeduplicator,                    // prevents double-processing
}

impl GitMonitor {
    async fn next_event(&mut self) -> GitEvent {
        if let Some(webhook) = &mut self.webhook_receiver {
            // Prefer webhook events (near-zero latency)
            tokio::select! {
                event = webhook.recv() => return self.dedup.process(event),
                event = self.polling_loop.next() => return self.dedup.process(event),
            }
        }
        // Fallback: polling only
        self.polling_loop.next().await
    }
}
```

Configuration:

```toml
[github]
polling_interval_seconds = 5
webhook_enabled = false           # opt-in, requires public endpoint
webhook_port = 9443
webhook_secret_env = "CALORON_WEBHOOK_SECRET"
```

---

## 3. Design Refinements

These items function correctly but have better alternatives.

### R1: Agent Mention Pattern — Use IDs, Not Roles

**Affected sections:** 4.1 (all `@agent-{role}` references), 7.4 (Supervisor Intervention Playbook)

**Problem:** The `@agent-{role}` pattern is ambiguous when multiple agents share the same role (e.g., `backend-1` and `backend-2` are both `backend-developer`). It also risks collision with real GitHub usernames.

**Refinement:** Use `@caloron-agent-{id}` where `{id}` is the unique agent ID from the DAG (e.g., `backend-1`, `reviewer-1`). The `caloron-agent-` prefix avoids GitHub username collisions.

All occurrences of `@agent-{role}` in the document should be replaced:
- `@agent-{role}` → `@caloron-agent-{id}`
- `@agent-backend-developer` → `@caloron-agent-backend-1`

The Git Monitor maps these IDs to agent instances using the DAG's agent registry.

---

### R2: Model Reference Indirection

**Affected sections:** 5.1 (Agent Definition), 13.2 (Configuration)

**Problem:** `claude-sonnet-4` hardcoded in agent definitions and `caloron.toml` will go stale as models are updated. Updating requires touching every agent YAML file.

**Refinement:** Add a model alias system to `caloron.toml`:

```toml
[llm]
api_key_env = "ANTHROPIC_API_KEY"

[llm.aliases]
default = "claude-sonnet-4-6"
fast = "claude-haiku-4-5"
strong = "claude-opus-4-6"
reviewer = "claude-opus-4-6"     # reviewers benefit from stronger reasoning
```

Agent definitions reference aliases:

```yaml
llm:
  model: "default"              # resolved at spawn time via caloron.toml
  max_tokens: 8192
  temperature: 0.2
```

A single config change (`default = "claude-sonnet-4-8"`) updates all agents using that alias. Agent definitions that need a specific model can still use a literal model ID.

---

### R3: Secrets Injection Mechanism

**Affected section:** 8.2 (Agent Environment as Nix Derivation)

**Problem:** Credentials injected via `shellHook` environment variables are visible in `/proc/<pid>/environ` on Linux and in shell history. The claim that "Nix's pure shell mode ensures only declared variables are available" is true for isolation between agents but does not protect against local privilege escalation or process inspection.

**Refinement:** Inject secrets via a temporary file that is read and deleted on startup:

**At spawn time (Agent Spawner):**

```rust
fn inject_secrets(&self, agent_id: &str, credentials: &[(&str, &str)]) -> PathBuf {
    let secrets_dir = PathBuf::from("/run/caloron/secrets");
    let secrets_file = secrets_dir.join(format!("{}.env", agent_id));

    // Write secrets file with restrictive permissions
    let mut file = OpenOptions::new()
        .create(true)
        .write(true)
        .mode(0o600)
        .open(&secrets_file)?;

    for (key, value) in credentials {
        writeln!(file, "{}={}", key, value)?;
    }

    secrets_file
}
```

**In the harness (startup):**

```rust
fn load_and_delete_secrets() -> HashMap<String, String> {
    let secrets_path = std::env::var("CALORON_SECRETS_FILE")
        .expect("CALORON_SECRETS_FILE must be set");

    let contents = std::fs::read_to_string(&secrets_path)
        .expect("Failed to read secrets file");

    // Delete immediately after reading
    std::fs::remove_file(&secrets_path)
        .expect("Failed to delete secrets file");

    // Parse and set as env vars for tools that need them
    contents.lines()
        .filter_map(|line| line.split_once('='))
        .map(|(k, v)| (k.to_string(), v.to_string()))
        .collect()
}
```

The `shellHook` in the Nix expression only sets non-secret configuration (see corrected example in E1).

---

### R4: TaskState Wrapper Struct

**Affected section:** 13.1 (Core Rust Types)

**Problem:** `HashMap<String, (Task, TaskStatus)>` is ergonomically awkward and loses useful metadata.

**Refinement:** Replace with a `TaskState` wrapper:

```rust
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TaskState {
    pub task: Task,
    pub status: TaskStatus,
    pub status_changed_at: DateTime<Utc>,
    pub intervention_count: u32,
    pub pr_numbers: Vec<u64>,          // PRs opened for this task (may be >1 if reworked)
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DagState {
    pub sprint: Sprint,
    pub tasks: HashMap<String, TaskState>,   // was: HashMap<String, (Task, TaskStatus)>
    pub agents: HashMap<String, AgentNode>,
    pub last_updated: DateTime<Utc>,
}
```

This enables cleaner access patterns (`dag.tasks["task-1"].status` vs `dag.tasks["task-1"].1`) and tracks per-task metadata that the Supervisor and Retro Engine need.

---

### R5: Timeline Adjustment

**Affected section:** 14 (Implementation Roadmap)

**Problem:** 19 weeks with zero integration buffer between phases is optimistic. Phase 4 (Git Monitor) depends heavily on Phase 3 (DAG Engine) and Phase 2 (Supervisor), and integration between these components will surface issues not caught by unit tests.

**Refinement:** Add 3 buffer weeks for a total of **22 weeks**:

| Week | Phase | Notes |
|------|-------|-------|
| 1-2 | Phase 0 — Foundation | |
| 3-5 | Phase 1 — Agent Lifecycle | |
| 6-8 | Phase 2 — Supervisor | |
| 9-10 | Phase 3 — DAG Engine | |
| **11** | **Integration buffer** | Wire Phases 1-3 together, run first mini-sprint |
| 12-13 | Phase 4 — Git Monitor | |
| 14-15 | Phase 5 — PO Agent + Kickoff | |
| **16** | **Integration buffer** | End-to-end sprint test before Retro |
| 17-18 | Phase 6 — Retro Engine | |
| 19 | Phase 7 — Noether Integration | |
| 20-22 | Phase 8 — Hardening | Extended from 2 to 3 weeks |

---

## 4. Updated Open Questions

Items marked **RESOLVED** are addressed by this addendum. Remaining items are unchanged.

| # | Question | Status | Resolution |
|---|----------|--------|------------|
| 1 | Should Caloron use webhooks instead of polling? | **RESOLVED (H4)** | Both. Webhook as primary, polling as fallback. Implemented in Phase 4. |
| 2 | How to handle multiple simultaneous sprints? | Open | Post-Phase 8 |
| 3 | Should agent definitions be versioned in caloron-meta or project repo? | Open | Phase 5 design review |
| 4 | Right stall threshold for different agent types? | Open | Configurable per agent definition (already supported in YAML) |
| 5 | Should Supervisor have direct write access to project repo? | **RESOLVED (H1)** | No. Supervisor communicates via Git Protocol only. Daemon handles direct escalation as a fallback when Supervisor is down. |
| 6 | How does sprint cancellation handle partial work? | **RESOLVED (H3)** | Graceful cancellation flow preserves PRs, worktrees, and runs partial retro. |
| 7 | Should retro suggestions be auto-applied? | Open | Human approval required in v1 |
| 8 | Private repos with self-hosted runners? | Open | Post-Phase 8 |
| **9** | **What is the canonical task completion signal?** | **RESOLVED (E2)** | `pr.merged` triggers atomic completion chain in orchestrator. |
| **10** | **What happens when a PR is closed without merge?** | **RESOLVED (E3)** | Task transitions back to IN_PROGRESS; Supervisor notified after 2nd closure. |
| **11** | **How is the Supervisor itself monitored?** | **RESOLVED (H1)** | Daemon-level watchdog with auto-restart and direct human escalation. |

---

*Addendum A — reviewed 2026-04-08. Apply alongside Caloron Developer Docs v2.0.*
