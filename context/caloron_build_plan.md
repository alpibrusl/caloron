# Caloron v2 — Build Plan with Capability Demos

> 22-week implementation plan with 7 capability demos. Each demo proves a concrete system capability to stakeholders at a natural milestone.

---

## Demo Schedule Overview

| Demo | Week | Name | Key Proof | Best Audience |
|------|------|------|-----------|---------------|
| D0 | 2 | Config to Environment | YAML in, Nix env out, GitHub live | Engineering team |
| D1 | 5 | Spawn, Work, Destroy | Single agent completes a real task | Engineering team |
| D2 | 8 | Self-Healing System | Auto-recovery from 3 failure types | Engineering leadership |
| D3 | 10 | Dependency Chain | DAG state machine, sprint cancellation | Engineering team |
| **D4** | **11** | **Mini-Sprint (MVP)** | **First autonomous end-to-end sprint** | **All stakeholders** |
| D5 | 15 | Interactive Kickoff | Human intent to autonomous execution | Product / leadership |
| D6 | 18 | Learning Between Sprints | System improves sprint over sprint | Product / leadership |
| D7 | 22 | Production Readiness | Chaos testing with 5 concurrent agents | Eng leadership / ops |

**D4 is the most important milestone.** It's the first time the system runs autonomously. If stakeholders can only attend one demo, it should be D4.

---

## Phase 0 — Foundation (Weeks 1–2)

### Goal
A compiling Rust project with core data models, flake-based Nix integration, and live GitHub connectivity.

### Deliverables

#### 0.1 Rust project setup
- Initialize Cargo workspace: `caloron-daemon`, `caloron-harness`, `caloron-types`
- Write `flake.nix` for reproducible builds of both binaries
- Key dependencies: tokio, serde, serde_json, serde_yaml, chrono, octocrab, tracing, anyhow, clap

#### 0.2 Core data types (`caloron-types`)
- All types from Section 13.1: `Sprint`, `AgentNode`, `Task`, `TaskStatus`, `DagState`, `AgentHealth`, `AgentStatus`, `GitEvent`, `ReviewState`, `ErrorType`
- Apply **Addendum R4**: use `TaskState` wrapper struct instead of `(Task, TaskStatus)` tuple
- All types: `Serialize`, `Deserialize`, `Debug`, `Clone`
- Unit tests for all state machine transitions

#### 0.3 GitHub client
- `GitHubClient` wrapper around `octocrab`
- Methods: `create_issue`, `add_label`, `create_comment`, `list_events`, `assign_reviewer`
- Polling loop with **5-second default** interval (Addendum H4)
- Rate limiting: exponential backoff with jitter

#### 0.4 Nix flake generator
- Apply **Addendum E1**: generate flake devShell expressions (not channel-based shell.nix)
- `NixGenerator` takes `AgentDefinition` → produces flake devShell Nix expression
- Use `nativeBuildInputs` (not `buildInputs`)
- Snapshot tests for generated expressions
- Validate with `nix develop --dry-run`

#### 0.5 Configuration loading
- `Config` struct with TOML loading from `caloron.toml`
- Apply **Addendum R2**: model alias system (`[llm.aliases]` section)
- Validation: required env vars exist, GitHub token valid, meta repo accessible

### Demo D0 — "Config to Environment" (end of Week 2)

**Setup:** A test GitHub repository with a few issues and PRs. Nix installed on demo machine. One agent YAML definition (`agents/backend-developer.yaml`).

**Script:**
1. Run `caloron agent validate agents/backend-developer.yaml` → validation passes, show parsed config with resolved model alias
2. Run `caloron agent build agents/backend-developer.yaml` → Nix flake devShell builds, list available tools in the environment
3. Run `caloron events --repo owner/test-repo` → live stream of GitHub events appearing in the terminal

**What the audience sees:** A terminal session where YAML config goes in, a reproducible Nix environment comes out, and GitHub events stream in real time.

**What it proves:** The foundational pipeline works end to end. This is the substrate everything else builds on.

---

## Phase 1 — Agent Lifecycle (Weeks 3–5)

### Goal
The daemon can spawn and destroy agents as Nix environments with git worktrees. A single agent can receive a task, do work, and exit cleanly.

### Deliverables

#### 1.1 Agent definition parser
- YAML loading for `AgentDefinition`
- Validate required fields: `name`, `llm`, `system_prompt`, `tools`
- Model alias resolution from `caloron.toml`
- Tests with valid and invalid definitions

#### 1.2 Git worktree management
- `WorktreeManager`: `create`, `remove`, `list`
- Edge cases: worktree already exists (sprint resume), dirty worktree on removal
- Integration tests with real temp git repos

#### 1.3 Agent spawner
- Full spawn sequence (Section 8.4, corrected per Addendum E1)
- Apply **Addendum R3**: secrets injection via temporary file, not shellHook env vars
- Full destruction sequence (Section 8.5)
- Apply **Addendum H2**: daemon-level process monitoring for crash detection

#### 1.4 The harness binary (`caloron-harness`)
- Thin wrapper around configured LLM CLI
- Heartbeat loop: 60-second interval to daemon socket
- Apply **Addendum R3**: `load_and_delete_secrets()` on startup
- Error capture: detect repeated errors, report to daemon
- Feedback enforcer: verify feedback comment before allowing exit

#### 1.5 Daemon socket server
- Unix socket server in daemon
- Handle: heartbeat, status, error, completion messages
- Apply **Addendum H2**: PID monitoring for harness crash detection

### Demo D1 — "Spawn, Work, Destroy" (end of Week 5)

**Setup:** Phase 0 complete. A pre-created GitHub issue in the test repo: "Create a README.md file with project description." Anthropic API key configured.

**Script:**
1. Split terminal: left = `caloron status` (auto-refreshing), right = GitHub repo view
2. Run `caloron spawn --agent backend-developer --task issue-1`
3. Watch the lifecycle: SPAWNING → IDLE → WORKING → COMPLETING → DESTROYED
4. On GitHub: issue gets "@caloron-agent-backend-1 assigned" comment, branch appears, PR opens, feedback comment posted

**What the audience sees:** A real AI agent spawned from declarative config, doing real work in an isolated Nix environment, communicating entirely through Git.

**What it proves:** The core agent loop works. Spawn, work, feedback, destroy. The harness enforces the feedback contract. Secrets are injected securely.

**Duration:** ~2-3 minutes for a simple task.

---

## Phase 2 — Supervisor (Weeks 6–8)

### Goal
The Supervisor detects stalled agents and executes the intervention playbook. This directly fixes the primary v1 failure mode.

### Deliverables

#### 2.1 Health monitor
- 60-second health check loop
- `evaluate_health()` for each agent (all `HealthVerdict` variants)
- Unit tests with time-mocked inputs

#### 2.2 Stall intervention — No Git Activity
- Probe comment posting
- Restart flow: destroy agent, respawn with same task context
- Reassignment flow: find alternative role in DAG
- Track intervention history per task

#### 2.3 Stall intervention — Credentials Failure
- Escalation issue creation with structured format
- Monitor for human response
- Resume flow on resolution comment

#### 2.4 Review loop detection and mediation
- Review cycle counter per PR
- `analyze_review_loop()`: LLM-based thread analysis
- Mediation comment posting
- Escalation on mediation failure

#### 2.5 Supervisor agent harness
- Special agent with elevated permissions
- Tools: `inspect_agent_health`, `post_mediation`, `escalate_to_human`, `restart_agent`, `reassign_task`
- Apply **Addendum H1**: daemon-level watchdog on Supervisor process

#### 2.6 Supervisor watchdog (Addendum H1)
- `SupervisorWatchdog` in daemon (not in Supervisor itself)
- Heartbeat monitoring, auto-restart, direct human escalation bypass
- Max restart count before permanent escalation

### Demo D2 — "Self-Healing System" (end of Week 8)

**Setup:** Phases 0-1 complete. Agent spawned and working on a task.

**Script — three scenarios in sequence:**

**Scenario 1: Stall recovery**
1. Spawn agent on a task, let it start working
2. Kill the LLM process (simulate stall)
3. Watch: Supervisor detects no activity → posts probe comment → waits → restarts agent → agent resumes and completes
4. Show the GitHub issue timeline telling the story

**Scenario 2: Credential failure**
1. Spawn agent with an invalid API key
2. Watch: 3 consecutive 401 errors → Supervisor creates escalation issue with structured content → pauses agent
3. Human posts "resolved" comment → agent resumes

**Scenario 3: Supervisor crash**
1. Kill the Supervisor process
2. Watch: daemon watchdog detects missing heartbeat → restarts Supervisor within seconds
3. Show daemon logs confirming watchdog activation

**What the audience sees:** The GitHub issue timeline tells the recovery story. Probe comments, escalation issues, and self-healing all visible.

**What it proves:** The system handles the three most common failure modes automatically. This directly addresses v1's primary failure: agents stalled with no visibility and no intervention.

---

## Phase 3 — DAG Engine (Weeks 9–10)

### Goal
The daemon can load a DAG, track task state, and resolve dependencies correctly. Sprint cancellation is defined.

### Deliverables

#### 3.1 DAG loader and validator
- JSON loading for `DagState` from `dag.json`
- JSON Schema validation
- Semantic validation: no cycles, all references valid
- Tests: valid DAGs, cyclic DAGs, invalid references

#### 3.2 State machine
- All state transitions with preconditions
- `evaluate_unblocked()` for dependency resolution
- State persistence: `state/sprint-{id}.json` after each transition
- State recovery: reload on daemon restart (sprint resume)
- Apply **Addendum H3**: sprint cancellation flow via `caloron stop`

#### 3.3 DAG query API
- `get_task_by_issue_number(n) → Option<Task>`
- `get_agent_for_role(role) → Option<AgentNode>`
- `get_reviewer_for_task(task_id) → Option<AgentNode>`
- `get_unblocked_tasks() → Vec<Task>`
- `get_tasks_in_status(status) → Vec<Task>`
- `is_sprint_complete() → bool`

### Demo D3 — "Dependency Chain" (end of Week 10)

**Setup:** Phases 0-2 complete. A DAG with 4 tasks in a diamond pattern: A and B are independent, C depends on both, D depends on C.

**Script:**
1. Load the DAG, show `caloron status` with all tasks PENDING
2. Simulate task A completion → A moves to DONE, C stays PENDING (still waiting on B)
3. Simulate task B completion → B moves to DONE, C transitions to READY automatically
4. Show the dependency resolution in action
5. Start task C, then run `caloron stop` → demonstrate sprint cancellation:
   - C's issue gets cancellation comment
   - C's PR (if open) gets `caloron:sprint-cancelled` label
   - Partial retro generated for completed tasks A and B
   - State file captures final snapshot

**What the audience sees:** A terminal visualization of the DAG with states updating in real time. Dependencies unlock correctly. Cancellation is clean and auditable.

**What it proves:** The DAG engine handles complex dependency graphs and the cancellation edge case. State is deterministic and persistent.

---

## Integration Buffer — Week 11

### Goal
Wire Phases 1-3 together. Run the first real end-to-end sprint with multiple agents.

This week pulls forward minimal Git Monitor functionality (handling `pr.merged` → DAG state transition) to enable the MVP demo.

### Demo D4 — "Mini-Sprint" (MVP) (end of Week 11)

**Setup:** All prior phases complete and integrated. A simple 2-task DAG:
- Task 1: Agent creates a utility module (e.g., a string helper library)
- Task 2 (depends on Task 1): A second agent writes tests for that module

**Script:**
1. Load the DAG manually (PO Agent not yet built — hand-craft `dag.json`)
2. Run `caloron start`
3. Watch the full autonomous flow:
   - Issues created automatically from DAG
   - Agent 1 spawned, picks up Task 1, implements the module, opens PR
   - Reviewer agent reviews PR, approves (or requests changes → agent fixes → re-review)
   - PR merged → Task 1 DONE → Task 2 unblocked
   - Agent 2 spawned, picks up Task 2, writes tests, opens PR
   - Review → merge → Task 2 DONE → sprint complete
4. Show `caloron status` at the end: all tasks DONE, all agents DESTROYED
5. Show the GitHub repository: clean PRs with review comments, feedback comments on issues

**What the audience sees:** A real GitHub repository where work happens autonomously. Two PRs opened, reviewed, and merged in sequence. No human intervention after `caloron start`.

**What it proves:** The core orchestration loop works end to end. Agents collaborate through Git without knowing about each other. Dependencies are respected. **This is the minimum viable product.**

**Duration:** 15-30 minutes depending on task complexity.

**This is the single most important demo.** Everything before it is component-level; everything after it is refinement.

---

## Phase 4 — Git Monitor (Weeks 12–13)

### Goal
The daemon responds correctly to all Git events and drives the DAG state machine. Full event coverage.

### Deliverables

#### 4.1 Event polling loop
- Configurable interval (default 5s, per Addendum H4)
- Apply **Addendum H4**: event coalescing — process all events per cycle
- Event deduplication: track processed event IDs
- Event ordering: chronological within each cycle
- GitHub API pagination

#### 4.2 Event handlers
Implement all handlers from Section 4.1, with addendum corrections:
- `handle_issue_opened`
- `handle_issue_labeled`
- `handle_issue_closed`
- `handle_pr_opened`
- `handle_pr_review_submitted`
- `handle_pr_merged` — apply **Addendum E2**: atomic completion chain
- `handle_pr_closed` — **new**, per **Addendum E3**: task back to IN_PROGRESS
- `handle_comment_created` (including feedback comment detection)
- `handle_push_received`

Apply **Addendum R1**: use `@caloron-agent-{id}` for all agent mentions.

All handlers must be idempotent.

#### 4.3 Webhook receiver (Addendum H4)
- HTTP server for GitHub webhook events
- Webhook signature verification (`CALORON_WEBHOOK_SECRET`)
- Event deduplication shared with polling loop
- Fallback: polling continues even when webhooks are active

#### 4.4 Label management
- `LabelManager`: ensure Caloron labels exist, create on first run
- Atomic label transitions

#### 4.5 Integration tests
- Test full flow: issue → spawn → PR → approve → merge → task done
- Test stall detection: assign agent, no activity, verify probe
- Test PR closure without merge (Addendum E3)

*No standalone demo — Git Monitor is infrastructure, best demonstrated as part of D4 (already shown) and D5 (upcoming).*

---

## Phase 5 — PO Agent + Kickoff (Weeks 14–15)

### Goal
A human can run `caloron kickoff "goal"` and the PO Agent generates a valid DAG after interactive dialogue.

### Deliverables

#### 5.1 Kickoff CLI command
- `caloron kickoff <goal>`
- Spawn PO Agent with kickoff system prompt
- Interactive terminal session for human-PO dialogue
- DAG detection: parse JSON code block from PO output
- Validation → summary → human approval → write to meta repo → create issues → start daemon

#### 5.2 PO Agent tools
- `read_repository_state`: summarize open issues, recent commits, file structure
- `write_dag`: write validated DAG JSON to meta repo
- `create_issues_from_dag`: create all GitHub issues from DAG tasks
- `list_available_agent_types`: return agent definitions from meta repo

#### 5.3 Issue template rendering
- Template rendering: `{task_title}`, `{dependencies}`, `{acceptance_criteria}` placeholders
- Validation: minimum required sections present

### Demo D5 — "Interactive Kickoff to Working Sprint" (end of Week 15)

**Setup:** Phases 0-4 complete and integrated. A test repository with some existing code (e.g., a basic web API with a User model but no auth).

**Script:**
1. Run `caloron kickoff "add user profile page with avatar upload"`
2. PO Agent analyzes the repository:
   - "I see the project has a User model with name and email fields, a REST API with /users endpoints, but no profile page or file upload capability."
3. PO Agent asks clarifying questions:
   - "Should avatars be stored in S3 or local filesystem?"
   - "What's the max avatar file size?"
   - "Should the profile page be server-rendered or a separate frontend?"
4. Human answers
5. PO Agent generates DAG: 3 tasks (avatar upload endpoint, profile API, frontend page), 2 agent roles, dependency chain
6. PO Agent presents summary: "I propose 3 tasks across 2 agents. Critical path: upload endpoint → profile API → frontend page. Estimated duration: 4-6 hours."
7. Human approves
8. GitHub issues appear automatically, agents start picking up tasks

**What the audience sees:** An interactive conversation that feels like a planning meeting with a senior engineer. Then the repository lights up with well-specified issues and agents start working.

**What it proves:** The full product loop from vague human intent to autonomous execution. The PO Agent decomposes goals into actionable tasks. The kickoff-to-execution pipeline is seamless.

**Best audience:** Product leadership and non-technical stakeholders. This demo is the most intuitive — anyone who has participated in a sprint planning meeting will immediately understand what's happening.

---

## Integration Buffer — Week 16

### Goal
Run a full 3-task sprint from kickoff through execution. Fix integration issues between Phases 4 and 5. Validate the full loop before building the Retro Engine on top of it.

---

## Phase 6 — Retro Engine (Weeks 17–18)

### Goal
At sprint end, the Retro Engine produces a useful retro report automatically. The system learns between sprints.

### Deliverables

#### 6.1 Feedback collector
- YAML parser for `caloron_feedback:` blocks in issue comments
- Handle synthetic feedback from crashed agents (Addendum H2)
- Fetch all sprint issues, extract feedback, build `SprintFeedback` struct

#### 6.2 Pattern analyzer
- Clarity analysis: group low-clarity tasks by template type
- Dependency discovery: extract runtime dependencies from blocker lists
- Tool gap analysis: identify unavailable tools from error reports
- Review loop analysis: correlate PR cycle counts with root causes
- Token and time efficiency: flag anomalies vs. prior sprints

#### 6.3 Report generator
- Markdown report: `caloron-meta/retro/sprint-{id}.md`
- Actionable suggestions extraction
- Optional: post summary to project repo as GitHub discussion

#### 6.4 Retro CLI command
- `caloron retro` (manual or automatic at sprint end)
- `--sprint-id` flag for retroactive analysis

### Demo D6 — "Learning Between Sprints" (end of Week 18)

**Setup:** Phases 0-5 complete. Two sequential sprint runs planned.

**Script:**

**Sprint 1 — deliberately imperfect:**
1. Run a sprint with curated issues designed to surface problems:
   - One task with a vague description (low clarity score expected)
   - One agent configured without a needed tool (tool gap)
   - One PR that goes through 3 review cycles (ambiguous requirements)
2. Sprint completes (with some supervisor interventions)
3. Run `caloron retro`
4. Show the retro report:
   - "task-2 clarity: 3/10 — 2 agents reported 'error format not specified'"
   - "qa-engineer missing Redis MCP — add to agent definition"
   - "PR #14 went through 3 review cycles: root cause — ambiguous acceptance criteria"
   - Suggestions: improve task template, add tool, clarify criteria

**Sprint 2 — improved:**
5. Apply the retro suggestions (update agent YAML, improve task template)
6. Run a similar sprint
7. Show results: clarity scores higher, no tool gaps, review cycles reduced
8. Run `caloron retro` again — show improvement metrics

**What the audience sees:** A retro report that reads like a competent engineering retrospective. Then evidence that Sprint 2 ran smoother than Sprint 1 because of specific, actionable improvements.

**What it proves:** The system learns from its mistakes. The feedback loop from execution to planning is functional. This is what differentiates Caloron from a simple task runner — it gets better over time.

---

## Phase 7 — Noether Integration (Week 19)

### Goal
Agents can use Noether stages via MCP, and token savings are tracked in feedback.

### Deliverables

#### 7.1 Noether service management
- `NoetherService`: start Noether daemon as subprocess
- Health check before sprint start
- Graceful shutdown at sprint end

#### 7.2 Noether client
- `NoetherClient`: `compose`, `search_stages`, `get_trace`
- Dogfooding: use `compose` in Retro Engine for feedback pattern analysis

#### 7.3 Feedback integration
- Parse `tools_used` for Noether stages
- Track stage reuse rates in retro
- Add "token savings via Noether" section to retro report

*No standalone demo — Noether integration is an optimization layer shown as part of D7.*

---

## Phase 8 — Hardening (Weeks 20–22)

### Goal
Production-ready: graceful error handling, full observability, credential management, load tested.

### Deliverables

#### 8.1 Error handling and recovery
- Daemon restart recovery: reload from `state/sprint-{id}.json`
- Agent crash recovery: full Addendum H2 flow
- GitHub API failure: circuit breaker with cached state fallback
- No panics in production — all error paths logged

#### 8.2 Observability
- Structured JSON logging via `tracing` (sprint_id, task_id, agent_role on every line)
- `caloron status` CLI: current sprint state, agent health, recent events
- `caloron logs <agent-role>`: tail agent harness logs
- `caloron trace <task-id>`: full event history for a task

#### 8.3 Credential management
- Never log credentials
- Validate all credentials at startup, fail fast
- Credential rotation without daemon restart (re-read from secrets file per R3)

#### 8.4 Load testing
- 10 concurrent agents
- GitHub API rate limiting handled without event drops
- Health Monitor performance at scale

#### 8.5 Sprint cancellation (Addendum H3)
- Full implementation of `caloron stop` flow
- Integration test: cancel mid-sprint, verify all cleanup steps

### Demo D7 — "Production Readiness" (end of Week 22)

**Setup:** All phases complete. A real sprint with 5 concurrent agents working on a non-trivial project.

**Script — "The Chaos Demo":**

1. Start a sprint with 5 agents: 2 backend developers, 1 frontend developer, 1 QA engineer, 1 reviewer
2. Show `caloron status` — all agents working, tasks progressing
3. Introduce failures during execution:

**Failure 1: Kill an agent process**
- `kill -9` a backend agent's harness process
- Watch: daemon detects death → synthetic feedback posted → Supervisor restarts agent → agent resumes from git state

**Failure 2: Revoke a credential**
- Rotate the GitHub token mid-sprint
- Watch: agents hit 401 errors → Supervisor escalates → human provides new token → agents resume

**Failure 3: Network partition**
- Block GitHub API access via proxy for 2 minutes
- Watch: polling loop hits errors → circuit breaker activates → events buffered → connectivity returns → events processed → no data lost

**Failure 4: Conflicting requirements**
- Two agents working on overlapping code → merge conflict on PR
- Watch: Supervisor detects conflict → mediates via structured comment → resolution applied

4. Sprint completes despite all failures
5. Show `caloron status`: all tasks DONE
6. Run `caloron retro`: report captures all interventions and their resolutions

**What the audience sees:** A dashboard showing agents, health, and sprint progress. Failures flash yellow/red then return to green as the system self-heals. The sprint completes despite deliberate sabotage.

**What it proves:** The system is production-ready. It handles real-world failure modes gracefully. Observable and debuggable. Ready for daily use.

**Best audience:** Engineering leadership and operations teams who need confidence in reliability before adopting the system.

---

## Summary Timeline

```
Week  1  2  3  4  5  6  7  8  9 10 11 12 13 14 15 16 17 18 19 20 21 22
      ├──┤  ├─────┤  ├────────┤  ├──┤  █  ├──┤  ├──┤  █  ├──┤  │  ├─────┤
      P0     P1       P2        P3  INT  P4    P5   INT  P6    P7   P8
      D0     D1       D2        D3  D4        D5        D6         D7

P = Phase    INT = Integration buffer    D = Demo    █ = Buffer week
```

**Critical path:** Phase 2 (Supervisor) and Phase 4 (Git Monitor) are the riskiest components. The Week 11 integration buffer exists specifically to catch issues at the Phase 1-2-3 junction before building on top of it.

**Staffing note:** Phases 0-3 can be built by a single senior Rust engineer. Phase 4 onward benefits from a second engineer — one focused on the Git Monitor and event handlers, the other on the PO Agent and Retro Engine.

---

*Build Plan — Caloron v2, dated 2026-04-08. Companion to Caloron Developer Docs v2.0 and Addendum A.*
