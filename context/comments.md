# Caloron — Code Review Notes

> First-pass review of the current codebase. These are implementation gaps, wiring bugs, and TODOs that need attention before the live sprint loop is functional.

*April 2026*

---

## 1. Orchestrator does not spawn agents

**File:** `src/daemon/orchestrator.rs` → `execute_single_action`

The `SpawnAgent` action path is marked **TODO**. The orchestrator handles label/comment events and can determine which tasks are ready, but it never actually starts an agent process. The spawn infrastructure exists in `src/agent/spawner.rs` (worktree + Nix env + harness process), but `execute_single_action` does not call it.

**What to do:** wire `execute_single_action(OrchestratorAction::SpawnAgent { .. })` → `AgentSpawner::spawn(agent_def, task, config)`.

---

## 2. DAG state is never persisted across restarts

**File:** `src/daemon/orchestrator.rs` → `start_daemon`

`DagEngine::set_state_file` is never called in `start_daemon`. The sprint state path is computed (something like `~/.caloron/{project}/{sprint_id}/state.json`) but never passed to the engine. Restarting the daemon loses all in-memory task state.

**What to do:** call `engine.set_state_file(state_path)` right after constructing `DagEngine::load_from_file`, before entering the orchestrator loop.

---

## 3. `DaemonState::dag` is never populated

**File:** `src/daemon/state.rs`, `src/daemon/orchestrator.rs`

`DaemonState` has a `dag: Option<DagState>` field and a `set_dag` method. Neither is called anywhere in the orchestrator. The field exists for the socket / dashboard to query live DAG state, but it always returns `None`.

**What to do:** after each DAG engine tick, call `state.set_dag(engine.current_state().clone())` so the socket and dashboard can reflect live status.

---

## 4. Health map is always empty → supervisor is a no-op

**File:** `src/daemon/orchestrator.rs`, `src/supervisor/health_monitor.rs`

`run_health_checks` iterates over `DaemonState::agent_health`, but `register_agent` is never called when agents are spawned (because spawning is also TODO — see §1). The health map stays empty, so all supervisor checks, intervention decisions, and watchdog verdicts never fire.

**What to do:** call `state.register_agent(agent_id, initial_health)` at spawn time, and update the health entry on each heartbeat received over the daemon socket.

---

## 5. Harness default socket path does not match daemon socket path

**File:** `crates/caloron-harness/src/main.rs`, `src/daemon/socket.rs`

The harness defaults to `/run/caloron/daemon.sock` when `CALORON_DAEMON_SOCKET` is not set.
The daemon binds to `/run/caloron/{sprint_id}.sock` (sprint-scoped).

These will never connect unless `CALORON_DAEMON_SOCKET` is explicitly set in the agent's environment at spawn time.

**What to do:** when spawning a harness process (§1), set `CALORON_DAEMON_SOCKET=/run/caloron/{sprint_id}.sock` in the child environment. The spawner already builds an env map — add the socket path there.

---

## 6. Retro feedback buffer is always empty

**File:** `src/retro/collector.rs`, `src/main.rs` → `Commands::Retro`

The retro `Commands::Retro` command constructs a `RetroCollector` with an empty `Vec<CaloronFeedback>`. The collector has infrastructure to aggregate feedback, but loading the stored feedback buffer from disk is noted as a Phase 4 TODO in a comment.

**What to do:** before constructing `RetroCollector`, load the persisted feedback log (e.g. `~/.caloron/{project}/feedback.jsonl`) and pass it in. The `CaloronFeedback` type is already serde-serialisable.

---

## 7. Several CLI commands are `todo!()`

**File:** `src/main.rs`

The following subcommands panic at runtime:

| Command | Status |
|---|---|
| `caloron stop` | `todo!()` |
| `caloron logs` | `todo!()` |
| `caloron trace` | `todo!()` |
| `caloron agent list` | `todo!()` |

**What to do:** `stop` can send a shutdown message over the daemon socket or look up the PID file. `logs` and `trace` can tail `~/.caloron/{project}/logs/`. `agent list` can read `DagState` from the state file and print agent definitions.

---

## 8. Noether client is not integrated into the orchestrator loop

**File:** `src/noether/client.rs`, `src/daemon/orchestrator.rs`

`NoetherClient` and `NoetherService` are implemented and tested, but nothing in the orchestrator calls them. The `CaloronConfig::noether` section (`enabled`, `endpoint`, `binary`) is parsed but its value is never acted on.

**What to do:** in `start_daemon`, if `config.noether.enabled`, construct a `NoetherService` and pass it into the orchestrator. The natural integration point is task dispatch — when a task's type is `"composition"`, delegate to Noether instead of spawning a raw agent.

---

## 9. Supervisor escalation uses hardcoded issue number `0`

**File:** `src/supervisor/escalation.rs`

Escalation issues are created with a placeholder `issue_number: 0` in at least one code path. This makes it impossible to subsequently close or reference the escalation issue.

**What to do:** use the `issue_number` returned by `GitHubClient::create_issue` and store it in `AgentHealth::escalation_issue` (the field exists in the type).

---

## 10. Missing root `README.md`

**File:** repository root

There is no `README.md`. The `docs/index.md` covers the same ground for the MkDocs site but GitHub renders nothing at the repo root.

**What to do:** add a minimal `README.md` that links to the docs site and covers the one-liner install + quickstart. Can mirror the first section of `docs/index.md`.

---

## Summary table

| # | Area | Severity | Effort |
|---|---|---|---|
| 1 | Agent spawn not wired | 🔴 Blocking | Medium |
| 2 | DAG state not persisted | 🔴 Blocking | Small |
| 3 | `DaemonState::dag` never set | 🟠 High | Small |
| 4 | Health map empty → supervisor no-op | 🟠 High | Medium (depends on §1) |
| 5 | Harness/daemon socket path mismatch | 🔴 Blocking | Small |
| 6 | Retro feedback buffer empty | 🟡 Medium | Small |
| 7 | CLI commands are `todo!()` | 🟡 Medium | Medium |
| 8 | Noether not integrated into loop | 🟡 Medium | Medium |
| 9 | Escalation issue number hardcoded `0` | 🟠 High | Small |
| 10 | No root `README.md` | 🟢 Low | Small |
