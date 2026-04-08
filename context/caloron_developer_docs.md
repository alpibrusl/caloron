# CALORON
## Multi-Agent Orchestration Platform — Developer Documentation v2.0

> Caloron orchestrates AI agents that collaborate through Git to build software. Agents communicate via issues, pull requests, and code reviews. The orchestrator manages their lifecycle, detects failures, and learns between sprints.

*This document covers the complete v2 architecture, implementation roadmap, and developer guide. Written for engineers building Caloron from scratch.*

---

## Table of Contents

1. [Why We Are Rebuilding](#1-why-we-are-rebuilding)
2. [Core Concepts](#2-core-concepts)
3. [Architecture Overview](#3-architecture-overview)
4. [The Git Protocol](#4-the-git-protocol)
5. [Agent Model](#5-agent-model)
6. [The DAG Engine](#6-the-dag-engine)
7. [The Supervisor Agent](#7-the-supervisor-agent)
8. [The Nix Execution Layer](#8-the-nix-execution-layer)
9. [The Retro Engine](#9-the-retro-engine)
10. [Noether Integration](#10-noether-integration)
11. [The PO Agent and Kickoff](#11-the-po-agent-and-kickoff)
12. [Repository Structure](#12-repository-structure)
13. [Data Models](#13-data-models)
14. [Implementation Roadmap](#14-implementation-roadmap)
15. [Configuration Reference](#15-configuration-reference)
16. [Observability and Debugging](#16-observability-and-debugging)
17. [Security Model](#17-security-model)
18. [Open Questions and Deferred Decisions](#18-open-questions-and-deferred-decisions)

---

## 1. Why We Are Rebuilding

### What We Built in v1

Caloron v1 used [Scion](https://github.com/GoogleCloudPlatform/scion) as the execution substrate. Scion runs agents as Docker containers with isolated git worktrees and communicates via tmux sessions. It is a solid foundation for parallel agent execution, but it does not match Caloron's communication model.

### The Core Problem with v1

Scion's mental model is: *"run N agents in parallel, each in their own container."* Caloron's mental model is: *"agents collaborate through Git artifacts — issues, PRs, reviews — and the orchestrator observes those artifacts to manage the system."*

These two models are fundamentally incompatible. Specifically:

**Problem 1 — Docker overhead and iteration cost.** Every agent configuration change required rebuilding or restarting containers. In a system where agents are frequently reconfigured (different tools, different MCPs, different prompts), this friction was constant.

**Problem 2 — No health model.** Scion knows if a process is alive or dead. It does not know if an agent is productively working, silently stuck in a loop, waiting for credentials that will never arrive, or blocked on a dependency. We had agents stalled for hours with no visibility into why.

**Problem 3 — No structured learning.** After a sprint, we had no systematic way to understand what went wrong, which tasks were unclear, or how to improve the next sprint. The retro was manual and inconsistent.

**Problem 4 — Communication leakage.** Agents communicated through tmux messages and shared filesystem state, not through Git. This meant the communication was ephemeral and not auditable.

### What v2 Fixes

| Problem | v1 | v2 |
|---|---|---|
| Execution substrate | Docker containers (Scion) | Nix environments (lightweight, reproducible) |
| Agent isolation | Container filesystem | Nix hermetic env + git worktree |
| Health monitoring | Process alive/dead only | Structured health contracts with stall detection |
| Agent communication | tmux messages + shared FS | Git events (issues, PRs, comments) only |
| Learning | Manual retro | Structured feedback + automated retro engine |
| Visibility | None into stalls | Supervisor with full project visibility |

---

## 2. Core Concepts

Before reading the architecture, understand these five concepts. Everything else builds on them.

### 2.1 Sprint

A sprint is the unit of work in Caloron. A sprint has a start (kickoff), an execution phase, and an end (retro). The DAG that defines which agents do what is **fixed for the duration of a sprint**. If the sprint needs to be fundamentally restructured, it is cancelled and a new sprint is started with the current repository state as input.

This is a deliberate constraint. A DAG that changes during execution is a non-deterministic control system that is extremely difficult to debug. Sprints are designed to be short enough that a fixed DAG is not a significant limitation.

### 2.2 Agent

An agent is a combination of a **system prompt**, a set of **tools/MCPs**, and an **LLM configuration**. It is stateless — all its state lives in Git. Agents are created at the start of a sprint and destroyed at the end. They are designed to be disposable and reproducible.

An agent is not a Docker container. It is a Nix environment with a specific set of tools available, running an LLM harness (Claude Code, or any ACLI-compatible harness) pointed at a specific git worktree.

### 2.3 DAG

The Directed Acyclic Graph defines the structure of a sprint: which agents exist, what roles they have, and how information flows between them. A DAG node is an agent. A DAG edge is a dependency — "agent B can only start working on task X after agent A has completed task Y."

The DAG is generated at kickoff by the PO Agent and stored in the Caloron metadata repository. It does not change during the sprint.

### 2.4 Git as Communication Protocol

Agents do not communicate directly with each other. They communicate exclusively through Git artifacts in the project repository:

- **Issues** — task assignments and task status
- **Pull Requests** — work output from an agent, requesting review from another
- **PR Reviews** — feedback from a reviewer agent to an author agent
- **Comments** — structured messages between agents, including feedback reports
- **Labels** — machine-readable state signals on issues and PRs

The Git Monitor watches these events and translates them into orchestrator actions. This means all agent communication is automatically auditable, versioned, and persistent.

### 2.5 Supervisor

The Supervisor is a special agent with full visibility into the project repository, the DAG state, and all agent health contracts. It is the only agent that can contact the human operator. Its job is to detect and resolve problems that individual agents cannot resolve themselves — infinite loops in PR reviews, stalled agents, credential failures, tasks that are beyond any agent's capability.

---

## 3. Architecture Overview

```
┌─────────────────────────────────────────────────────────────────────┐
│                        CALORON DAEMON (Rust)                        │
│                                                                     │
│  ┌─────────────┐   ┌──────────────┐   ┌──────────────────────────┐ │
│  │  PO Agent   │   │  DAG Engine  │   │      Git Monitor         │ │
│  │  (kickoff)  │──▶│  (static,    │──▶│  (event loop over        │ │
│  │             │   │   per sprint)│   │   project repo)          │ │
│  └─────────────┘   └──────────────┘   └────────────┬─────────────┘ │
│                                                    │               │
│                                       ┌────────────▼─────────────┐ │
│                                       │     Agent Spawner        │ │
│                                       │  (Nix envs, ephemeral)   │ │
│                                       └────────────┬─────────────┘ │
│                                                    │               │
│  ┌─────────────────────────────────────────────────▼─────────────┐ │
│  │                    SUPERVISOR AGENT                           │ │
│  │  ┌─────────────┐  ┌──────────────┐  ┌──────────────────────┐ │ │
│  │  │   Health    │  │   Conflict   │  │   Human Escalation   │ │ │
│  │  │  Monitor    │  │   Resolver   │  │       Gateway        │ │ │
│  │  └─────────────┘  └──────────────┘  └──────────────────────┘ │ │
│  └───────────────────────────────────────────────────────────────┘ │
│                                                                     │
│  ┌───────────────────────────────────────────────────────────────┐ │
│  │                     RETRO ENGINE                              │ │
│  │  ┌─────────────┐  ┌──────────────┐  ┌──────────────────────┐ │ │
│  │  │  Feedback   │  │   Pattern    │  │   DAG Improvement    │ │ │
│  │  │  Collector  │  │   Detector   │  │    Suggestions       │ │ │
│  │  └─────────────┘  └──────────────┘  └──────────────────────┘ │ │
│  └───────────────────────────────────────────────────────────────┘ │
└─────────────────────────────────────────────────────────────────────┘
         │                        │                        │
         ▼                        ▼                        ▼
  ┌─────────────┐        ┌────────────────┐      ┌─────────────────┐
  │  project    │        │  caloron-meta  │      │    Noether      │
  │  repo       │        │  repo          │      │  (computation)  │
  │  (git)      │        │  (git)         │      │                 │
  └─────────────┘        └────────────────┘      └─────────────────┘
```

### Component Responsibilities

| Component | Language | Responsibility |
|---|---|---|
| Caloron Daemon | Rust | Main process; owns all other components |
| PO Agent | LLM (via harness) | Kickoff; DAG generation; sprint scope definition |
| DAG Engine | Rust | Load, validate, and evaluate the sprint DAG |
| Git Monitor | Rust | Event loop; translates git events to DAG actions |
| Agent Spawner | Rust | Create and destroy Nix-based agent environments |
| Supervisor Agent | LLM (via harness) | Health monitoring; conflict resolution; escalation |
| Retro Engine | Rust + LLM | Feedback aggregation; pattern detection |
| Noether Client | Rust | Calls Noether for verified computation stages |

---

## 4. The Git Protocol

Git is the communication bus between agents. This section defines the exact semantics of each Git event and what the orchestrator does in response.

### 4.1 Event Types and Handlers

#### `issue.opened`
Triggered when a new issue is created in the project repository.

**Orchestrator action:**
1. Parse issue labels to determine task type (`caloron:task`, `caloron:bug`, `caloron:research`, etc.)
2. Look up the DAG to determine which agent role should handle this task type
3. If the appropriate agent is available and the DAG dependency is satisfied, assign the issue to the agent and spawn an agent instance if not already running
4. Add label `caloron:assigned` and comment `@agent-{role} has been assigned this task`
5. Reset stall timer for the assigned agent

#### `issue.labeled`
**Orchestrator action:** Re-evaluate DAG assignment. A label change may indicate a task type change that requires a different agent.

#### `pr.opened`
Triggered when an agent opens a pull request.

**Orchestrator action:**
1. Parse PR description for the linked issue (`Closes #N`)
2. Look up the DAG to determine which agent role is the designated reviewer for PRs from the author role
3. Assign review to the reviewer agent
4. Add label `caloron:review-pending`
5. Reset stall timer for the reviewer agent

#### `pr.review_submitted` with `state: approved`
**Orchestrator action:**
1. Check if all required reviewers have approved (as defined in DAG)
2. If yes, add label `caloron:merge-ready`
3. If auto-merge is configured, trigger merge
4. Mark the linked issue as `caloron:done` in DAG state
5. Trigger any DAG-dependent tasks that were waiting on this completion

#### `pr.review_submitted` with `state: changes_requested`
**Orchestrator action:**
1. Remove `caloron:review-pending` label
2. Add `caloron:changes-requested` label
3. Re-notify the author agent: create a comment mentioning `@agent-{author-role}` with the review summary
4. Reset stall timer for author agent
5. Increment the review cycle counter for this PR (used by supervisor to detect infinite loops)

#### `pr.merged`
**Orchestrator action:**
1. Mark task as `completed` in DAG state
2. Record completion time and token cost from the closing feedback comment
3. Identify all DAG tasks that had this task as a dependency
4. For each newly unblocked task, trigger `issue.opened` flow for the next task

#### `comment.created` matching `@agent-{role}`
**Orchestrator action:**
1. Extract the mentioned agent role
2. If the agent is not running, spawn it with the relevant context (issue number, comment content)
3. If running, inject the message into the agent's input queue
4. Reset stall timer for the mentioned agent

#### `comment.created` matching `caloron_feedback:` YAML block
**Orchestrator action:**
1. Parse the structured feedback block
2. Store in retro buffer for end-of-sprint processing
3. Do not trigger any agent actions — feedback comments are for the retro engine only

### 4.2 The Structured Feedback Comment

When an agent completes a task (closes an issue or merges a PR), it posts a structured feedback comment. This is mandatory — the Caloron harness wrapper enforces it.

```yaml
---
caloron_feedback:
  task_id: "issue-42"
  agent_role: "backend-developer"
  task_clarity: 4        # 1-10: how clear was the task description?
  blockers:
    - "Error response format was not specified in the issue"
    - "Dependency on issue #38 was not in the DAG — discovered at runtime"
  tools_used:
    - "github_mcp"
    - "noether:http_post_json"
    - "noether:json_validate"
  tokens_consumed: 14200
  time_to_complete_min: 47
  self_assessment: "completed"   # completed | partial | blocked | failed
  notes: "Had to make assumptions about the pagination strategy"
---
```

The feedback comment is the primary input to the Retro Engine. Every field is required. If an agent fails to produce a feedback comment, the Supervisor marks it as a health event.

### 4.3 Label Taxonomy

All Caloron-managed labels use the `caloron:` prefix to avoid collision with project labels.

| Label | Set by | Meaning |
|---|---|---|
| `caloron:task` | PO Agent | This issue is a Caloron-managed task |
| `caloron:assigned` | Orchestrator | Task has been assigned to an agent |
| `caloron:in-progress` | Agent | Agent has started working |
| `caloron:blocked` | Supervisor | Task is blocked, waiting for intervention |
| `caloron:review-pending` | Orchestrator | PR is waiting for review |
| `caloron:changes-requested` | Orchestrator | PR has requested changes |
| `caloron:merge-ready` | Orchestrator | PR is approved, ready to merge |
| `caloron:done` | Orchestrator | Task is complete |
| `caloron:escalated` | Supervisor | Issue has been escalated to human |
| `caloron:stalled` | Supervisor | Agent has not shown activity beyond threshold |

---

## 5. Agent Model

### 5.1 Agent Definition

An agent is defined as a YAML file in the `caloron-meta` repository under `agents/`. It is completely declarative — no code, only configuration.

```yaml
# agents/backend-developer.yaml
name: backend-developer
version: "1.0"
description: "Implements backend features, writes tests, and opens PRs for review"

llm:
  model: claude-sonnet-4          # or any supported model
  max_tokens: 8192
  temperature: 0.2                # low temperature for code generation

system_prompt: |
  You are a senior backend developer working on this project.
  You receive tasks as GitHub issues assigned to you.
  
  Your workflow:
  1. Read the assigned issue carefully
  2. Explore the codebase to understand the context
  3. Implement the required changes
  4. Write tests for your changes
  5. Open a pull request with a clear description
  6. When complete, post a caloron_feedback comment on the issue
  
  Always check existing code patterns before implementing.
  Never make assumptions about external API formats — ask via issue comment if unclear.
  
  You have access to the following tools: {tools}

tools:
  - github_mcp                    # read/write issues, PRs, comments
  - noether                       # verified computation stages
  - bash                          # run tests, linters

mcps:
  - url: "https://github.mcp.claude.com/mcp"
    name: "github"
  - url: "http://localhost:8080/mcp"   # local Noether instance
    name: "noether"

nix:
  packages:
    - nodejs_20
    - python311
    - rustc
    - cargo
  env:
    NODE_ENV: "test"

stall_threshold_minutes: 20       # alert supervisor if no git activity for 20 min
max_review_cycles: 3              # supervisor intervenes after 3 review cycles on same PR
```

### 5.2 Agent Lifecycle

```
DEFINED       → agent.yaml exists in caloron-meta
    ↓
SPAWNING      → Nix env being built, git worktree being created
    ↓
IDLE          → running, waiting for task assignment
    ↓
WORKING       → assigned to an issue, actively making git events
    ↓
STALLED       → no git activity for > stall_threshold_minutes
    ↓            (supervisor intervenes)
BLOCKED       → supervisor has determined the agent cannot proceed alone
    ↓            (supervisor escalates or reassigns)
COMPLETING    → agent has posted feedback comment, wrapping up
    ↓
DESTROYED     → end of sprint, Nix env torn down, worktree removed
```

### 5.3 Agent Context Window Management

Each agent has its own context window. An agent cannot read another agent's context. This is by design — context isolation prevents agents from being confused by each other's internal reasoning.

What an agent CAN see:
- Its own system prompt
- The git history of its assigned worktree
- Issues and PRs it has been mentioned in or assigned to
- Comments directed at its role via `@agent-{role}`
- Any files it has read via its tools

What an agent CANNOT see:
- Other agents' context windows or internal reasoning
- The Supervisor's internal state
- The DAG structure (agents are not aware they are being orchestrated)
- Other agents' worktrees

This last point is important: **agents should not know they are agents in a multi-agent system.** They receive tasks as GitHub issues and deliver results as pull requests. The orchestration is invisible to them.

### 5.4 The Harness

The harness is the thin wrapper that runs the LLM in the agent's Nix environment. It is responsible for:

1. Injecting the system prompt with the correct tool list
2. Pointing the LLM at the correct git worktree
3. Enforcing the feedback comment requirement on task completion
4. Reporting heartbeats to the Caloron daemon every 60 seconds
5. Capturing token usage and forwarding to the health monitor

The harness is harness-agnostic by design — it wraps Claude Code, Gemini CLI, or any ACLI-compatible tool. The interface between the harness and the Caloron daemon is a simple Unix socket:

```
// Heartbeat message (every 60 seconds)
{ "type": "heartbeat", "agent_role": "backend-developer", "task_id": "issue-42", "tokens_used": 4200 }

// Status update
{ "type": "status", "agent_role": "backend-developer", "status": "working", "detail": "implementing auth middleware" }

// Error report
{ "type": "error", "agent_role": "backend-developer", "error_type": "credentials", "detail": "GitHub token 401", "count": 3 }

// Completion signal
{ "type": "completed", "agent_role": "backend-developer", "task_id": "issue-42" }
```

---

## 6. The DAG Engine

### 6.1 The DAG Format

The sprint DAG is stored as `dag.json` in the `caloron-meta` repository. It is generated by the PO Agent at kickoff and is immutable for the duration of the sprint.

```json
{
  "sprint": {
    "id": "sprint-2026-04-w2",
    "goal": "Implement user authentication and session management",
    "start": "2026-04-08T09:00:00Z",
    "max_duration_hours": 72
  },
  "agents": [
    {
      "id": "po",
      "role": "product-owner",
      "definition": "agents/product-owner.yaml"
    },
    {
      "id": "backend-1",
      "role": "backend-developer",
      "definition": "agents/backend-developer.yaml"
    },
    {
      "id": "backend-2",
      "role": "backend-developer",
      "definition": "agents/backend-developer.yaml"
    },
    {
      "id": "reviewer-1",
      "role": "senior-reviewer",
      "definition": "agents/senior-reviewer.yaml"
    },
    {
      "id": "qa-1",
      "role": "qa-engineer",
      "definition": "agents/qa-engineer.yaml"
    },
    {
      "id": "supervisor",
      "role": "supervisor",
      "definition": "agents/supervisor.yaml"
    }
  ],
  "tasks": [
    {
      "id": "task-1",
      "title": "Implement JWT token generation and validation",
      "assigned_to": "backend-1",
      "issue_template": "tasks/jwt-implementation.md",
      "depends_on": [],
      "reviewed_by": "reviewer-1"
    },
    {
      "id": "task-2",
      "title": "Implement session store with Redis",
      "assigned_to": "backend-2",
      "issue_template": "tasks/session-store.md",
      "depends_on": [],
      "reviewed_by": "reviewer-1"
    },
    {
      "id": "task-3",
      "title": "Integration tests for auth flow",
      "assigned_to": "qa-1",
      "issue_template": "tasks/auth-integration-tests.md",
      "depends_on": ["task-1", "task-2"],
      "reviewed_by": "reviewer-1"
    }
  ],
  "review_policy": {
    "required_approvals": 1,
    "auto_merge": true,
    "max_review_cycles": 3
  },
  "escalation": {
    "stall_threshold_minutes": 20,
    "supervisor_id": "supervisor",
    "human_contact": "github_issue"
  }
}
```

### 6.2 DAG State Machine

The DAG Engine maintains the runtime state of each task. This state is separate from the Git labels (which are the external representation) and is stored in memory during a sprint, persisted to `caloron-meta/state/sprint-{id}.json` after each transition.

```
PENDING     → task exists in DAG, dependencies not yet satisfied
    ↓         (when all depends_on tasks reach DONE)
READY       → dependencies satisfied, waiting for agent to be available
    ↓         (when agent is spawned and issue is created)
IN_PROGRESS → issue created and assigned, agent is working
    ↓         (when PR is opened)
IN_REVIEW   → PR opened, waiting for reviewer
    ↓         (when PR is approved)
DONE        → PR merged, task complete
    
    ↓ at any point if supervisor intervenes
BLOCKED     → supervisor has paused this task
    ↓
CANCELLED   → task will not be completed in this sprint
```

### 6.3 Dependency Resolution

When a task transitions to `DONE`, the DAG Engine evaluates all `PENDING` tasks to determine if any are now unblocked:

```rust
fn evaluate_unblocked(&self, completed_task_id: &str) -> Vec<TaskId> {
    self.tasks
        .iter()
        .filter(|task| task.status == TaskStatus::Pending)
        .filter(|task| {
            task.depends_on.iter().all(|dep_id| {
                self.get_task(dep_id)
                    .map(|t| t.status == TaskStatus::Done)
                    .unwrap_or(false)
            })
        })
        .map(|task| task.id.clone())
        .collect()
}
```

Each newly unblocked task transitions to `READY` and the Git Monitor creates the corresponding GitHub issue.

---

## 7. The Supervisor Agent

The Supervisor is the most critical component of Caloron v2. It is the direct solution to the primary failure mode of v1: agents stalled with no visibility and no intervention.

### 7.1 Supervisor Responsibilities

The Supervisor has three distinct responsibilities that are handled by separate sub-components:

**Health Monitor** — continuously evaluates the health contract of every running agent and detects stall conditions.

**Conflict Resolver** — detects and resolves multi-agent conflicts: infinite PR review loops, contradictory requirements between agents, tasks that are beyond any agent's capability.

**Human Escalation Gateway** — the only path by which information reaches the human operator. The Supervisor decides when a problem exceeds its own resolution capability and how to present it to the human concisely.

### 7.2 The Health Contract

Every running agent has a health contract maintained by the Health Monitor:

```rust
pub struct AgentHealth {
    pub agent_id: String,
    pub role: String,
    pub current_task_id: Option<String>,
    pub last_git_event: DateTime<Utc>,      // last push, comment, or PR action
    pub last_heartbeat: DateTime<Utc>,      // last daemon socket message
    pub consecutive_errors: u32,
    pub error_types: Vec<ErrorType>,
    pub review_cycles: HashMap<String, u32>, // pr_id → cycle count
    pub status: AgentStatus,
    pub stall_threshold: Duration,
}

pub enum AgentStatus {
    Idle,
    Working,
    Stalled { since: DateTime<Utc>, reason: StallReason },
    Blocked { reason: String },
    Failed { error: String },
}

pub enum StallReason {
    NoGitActivity,
    NoHeartbeat,
    RepeatedErrors(ErrorType),
    ReviewLoopDetected,
}

pub enum ErrorType {
    CredentialsFailure,
    RateLimited,
    ToolUnavailable,
    UnknownError,
}
```

### 7.3 Stall Detection

The Health Monitor runs a check loop every 60 seconds. For each agent, it evaluates:

```rust
fn evaluate_health(&self, agent: &AgentHealth) -> HealthVerdict {
    let now = Utc::now();

    // Check 1: heartbeat timeout (process may be dead)
    if now - agent.last_heartbeat > Duration::minutes(5) {
        return HealthVerdict::ProcessDead;
    }

    // Check 2: no git activity beyond threshold
    if now - agent.last_git_event > agent.stall_threshold {
        return HealthVerdict::Stalled(StallReason::NoGitActivity);
    }

    // Check 3: repeated identical errors
    if agent.consecutive_errors >= 3 {
        let error_type = agent.error_types.last().unwrap();
        return HealthVerdict::Stalled(StallReason::RepeatedErrors(error_type.clone()));
    }

    // Check 4: review loop on a PR
    for (pr_id, cycles) in &agent.review_cycles {
        if *cycles >= self.config.max_review_cycles {
            return HealthVerdict::ReviewLoopDetected(pr_id.clone());
        }
    }

    HealthVerdict::Healthy
}
```

### 7.4 Supervisor Intervention Playbook

When the Health Monitor returns a non-healthy verdict, the Supervisor follows a structured playbook. Each intervention type has a defined action and an escalation condition.

#### Stall: No Git Activity

**Action 1 — Probe (immediate):**
Post a comment on the agent's current issue:
```
@agent-{role} You have had no activity for {N} minutes on this task. 
Please respond with your current status. If you are blocked, describe what you need.
```

**Action 2 — Restart (after 10 minutes with no response to probe):**
Destroy and respawn the agent with the same task context. The agent resumes from the git state — no work is lost.

**Action 3 — Reassign (after second stall on same task):**
If the agent stalls again after restart, the task may be beyond its capability. The Supervisor reassigns the task to a different agent role if one is available in the DAG, or marks the task as `BLOCKED`.

**Escalation condition:** Task marked `BLOCKED` with no reassignment option available.

#### Stall: Repeated Credentials Error

**Action (immediate) — Escalate to human:**
This cannot be resolved by the Supervisor. Post a GitHub issue tagged `caloron:escalated`:
```
🚨 Human intervention required

Agent: {role}
Problem: Credentials failure (3 consecutive 401 errors)
Tool: {tool_name}
Task: #{issue_number}

The agent cannot proceed until credentials are resolved.
Please check {tool_name} token configuration and comment "resolved" when fixed.
```

The Supervisor pauses the affected agent and marks the task as `BLOCKED` until the human resolves it.

#### Review Loop Detected

A PR review loop occurs when the same PR has gone through more than `max_review_cycles` review cycles without being approved or closed.

**Action 1 — Analyze the loop:**
The Supervisor reads the full PR review thread and uses its LLM capability to understand why the loop is occurring. Common causes:
- Reviewer and author have contradictory requirements
- The issue specification was ambiguous and each agent interpreted it differently
- The reviewer is applying standards not mentioned in the issue

**Action 2 — Mediate:**
The Supervisor posts a structured mediation comment on the PR:
```
@agent-{reviewer} @agent-{author}

This PR has been through {N} review cycles without resolution. 
I have analyzed the review thread and identified the core disagreement:

{supervisor_analysis}

Resolution: {proposed_resolution}

Both agents should acknowledge this resolution with a 👍 reaction. 
If either disagrees, I will escalate to the human operator.
```

**Action 3 — Escalate if mediation fails:**
If the loop continues after mediation, escalate to human with the full context and the Supervisor's analysis.

#### Task Beyond Agent Capability

Detected when an agent has attempted the same task multiple times (via restart) and consistently fails or stalls at the same point.

**Action — Escalate with diagnosis:**
```
🚨 Task escalation required

Task: #{issue_number} — {title}
Agent: {role} (attempted {N} times)
Failure pattern: {supervisor_analysis}

This task appears to require capabilities beyond the current agent configuration.
Options:
1. Clarify the task specification (comment with clarification)
2. Assign to a human developer (comment "assign-human")
3. Break down into smaller subtasks (comment "break-down")
```

### 7.5 Human Escalation Protocol

When the Supervisor escalates to a human, it creates a GitHub issue in the project repository tagged `caloron:escalated`. The issue contains:

1. A clear one-sentence description of the problem
2. What the Supervisor already tried
3. The exact information the human needs to provide
4. The specific action the human should take (comment format)

The Supervisor monitors the escalated issue for a human response. When the human responds with the expected comment format, the Supervisor resumes the paused workflow.

The human can also respond with `caloron:take-over` on any issue to indicate they are handling it directly. The Supervisor removes the issue from DAG tracking and marks the task as `HUMAN_ASSIGNED`.

---

## 8. The Nix Execution Layer

### 8.1 Why Nix Instead of Docker

Docker was the primary source of pain in v1. The specific problems:

- Agent configuration changes (new tools, updated prompts) required image rebuilds or container restarts
- Credentials had to be injected at container creation time — if they changed, the container had to be recreated
- Container networking between agents added complexity that was not necessary (agents communicate via Git, not via network)
- Docker Desktop on macOS introduced additional overhead and occasional instability

Nix solves all of these without replacing Docker's isolation guarantees:

- Configuration changes are instant — a Nix env is built from a derivation, not from a running container
- Credentials are injected as environment variables at process start time, not at container build time
- No networking between agents is needed — Nix processes on the same machine share the Nix store but not their runtime environment
- Nix runs natively on macOS and Linux with identical semantics

### 8.2 Agent Environment as Nix Derivation

Each agent definition in YAML is compiled by the Agent Spawner into a Nix shell expression:

```nix
# Generated from agents/backend-developer.yaml
{ pkgs ? import <nixpkgs> {} }:

pkgs.mkShell {
  name = "caloron-agent-backend-developer";
  
  buildInputs = with pkgs; [
    nodejs_20
    python311
    rustc
    cargo
    git
    # Caloron harness
    caloron-harness
    # Claude Code (or configured harness)
    claude-code
  ];
  
  shellHook = ''
    export CALORON_AGENT_ROLE="backend-developer"
    export CALORON_DAEMON_SOCKET="${daemonSocket}"
    export CALORON_WORKTREE="${worktreePath}"
    export CALORON_TASK_ID="${taskId}"
    # Credentials injected at spawn time, not build time
    export GITHUB_TOKEN="${githubToken}"
    export ANTHROPIC_API_KEY="${anthropicKey}"
  '';
}
```

The Nix store caches the build result. The second time an agent of the same type is spawned, it starts immediately — there is no rebuild.

### 8.3 Git Worktrees

Each agent gets a dedicated git worktree of the project repository. This means:

- All agents work on the same git history but in separate filesystem paths
- Agents cannot accidentally see each other's uncommitted work
- Switching between tasks does not require stashing or discarding work
- Worktrees are cheap — they share the git object store

```bash
# Agent Spawner creates worktree at spawn time
git worktree add \
  .caloron/worktrees/backend-developer-sprint-2026-04-w2 \
  -b agent/backend-developer/sprint-2026-04-w2

# Worktree removed at agent destruction
git worktree remove .caloron/worktrees/backend-developer-sprint-2026-04-w2
```

### 8.4 Agent Spawning Sequence

```
1. Agent Spawner receives spawn request from DAG Engine
   Input: { agent_definition, task_id, credentials }

2. Generate Nix shell expression from agent YAML definition

3. Check Nix store cache — if cached, skip to step 5

4. Build Nix environment
   nix-shell --pure generated-shell.nix --run "echo ready"

5. Create git worktree at .caloron/worktrees/{role}-{sprint-id}

6. Write task context to worktree
   - Current issue content
   - Relevant files (if specified in task)
   - Agent system prompt

7. Start harness process in Nix environment
   nix-shell generated-shell.nix --run "caloron-harness start"

8. Register agent in Health Monitor with health contract

9. Report spawn success to DAG Engine
```

### 8.5 Agent Destruction Sequence

```
1. Agent signals completion via daemon socket

2. Harness ensures feedback comment is posted to issue
   (enforced — harness will not exit without this)

3. Harness exits cleanly

4. Agent Spawner removes git worktree
   git worktree remove .caloron/worktrees/{role}-{sprint-id}

5. Health Monitor unregisters agent

6. DAG Engine transitions task to DONE

7. Nix environment is not explicitly removed — cached in Nix store
   for reuse in next sprint
```

---

## 9. The Retro Engine

### 9.1 Purpose

The Retro Engine runs at the end of each sprint. Its input is the collection of structured feedback comments posted by agents throughout the sprint. Its output is a retro report that the PO Agent uses to improve the next sprint.

This is the learning mechanism of Caloron. Without it, every sprint starts from the same baseline. With it, the system improves systematically over time.

### 9.2 Feedback Collection

At the end of a sprint, the Retro Engine fetches all issues in the project repository that were worked on during the sprint, extracts `caloron_feedback:` YAML blocks from comments, and builds a structured dataset:

```rust
pub struct SprintFeedback {
    pub sprint_id: String,
    pub tasks: Vec<TaskFeedback>,
    pub summary: SprintSummary,
}

pub struct TaskFeedback {
    pub task_id: String,
    pub agent_role: String,
    pub task_clarity: u8,               // 1-10
    pub blockers: Vec<String>,
    pub tools_used: Vec<String>,
    pub tokens_consumed: u64,
    pub time_to_complete_min: u32,
    pub self_assessment: SelfAssessment,
    pub notes: Option<String>,
    pub review_cycles: u32,             // from DAG state
    pub supervisor_interventions: u32,  // from supervisor log
}
```

### 9.3 Pattern Detection

The Retro Engine runs the following analyses on the collected feedback:

**Clarity Analysis**
Tasks with `task_clarity < 5` are flagged. The Retro Engine groups them by issue template and identifies common patterns in the blockers list. If multiple agents report similar blockers on similar task types, it generates a template improvement suggestion.

**Dependency Discovery**
Tasks where an agent reported "discovered dependency at runtime" indicate that the DAG was missing an explicit dependency. These are flagged as DAG improvement opportunities for the next sprint.

**Tool Gap Analysis**
If an agent reported being blocked because a required tool was unavailable, the Retro Engine flags this as a missing capability that should be added to the agent definition.

**Review Loop Analysis**
PRs that went through more than 2 review cycles are analyzed for root causes. Common causes (ambiguous requirements, conflicting standards, unclear acceptance criteria) are summarized and fed back into task template improvements.

**Token and Time Efficiency**
Tasks that took significantly more tokens or time than similar tasks in previous sprints are flagged for investigation. The Retro Engine does not automatically conclude why — it surfaces the anomaly for human review.

### 9.4 Retro Report Format

The Retro Engine produces a markdown report stored in `caloron-meta/retro/sprint-{id}.md`:

```markdown
# Sprint Retro — sprint-2026-04-w2

## Summary
- Tasks completed: 8/10
- Tasks blocked: 1
- Tasks cancelled: 1
- Average task clarity: 6.2/10
- Total tokens consumed: 284,000
- Supervisor interventions: 3

## 🔴 Critical Issues

### Credentials failure — GitHub MCP (backend-developer)
The backend-developer agent hit repeated 401 errors on the GitHub MCP.
Root cause: token scope was missing `workflow` permission.
Fix: Add `workflow` scope to GitHub token in agent configuration.

## 🟡 Clarity Issues (tasks scoring < 5/10)

### task-3: Integration tests for auth flow (clarity: 3/10)
Reported blockers:
- "Error response format not specified" (2/3 agents reported this)
- "Redis configuration environment not documented"

Suggested template improvements:
1. Add "Error response format" section to auth task template
2. Add Redis configuration reference to QA task template

## 🟢 What Worked Well

- task-1 and task-2 completed in parallel with zero conflicts
- Reviewer agent consistently provided actionable feedback
- Noether stages `http_post_json` and `jwt_validate` were reused 6 times

## DAG Improvements for Next Sprint

- Add explicit dependency: task-3 should depend on Redis setup task
  (discovered at runtime by qa-1 agent)
- Consider splitting task-2 into session-store-setup and session-store-api
  (size was 3x larger than estimated)

## Agent Configuration Changes

- backend-developer: add `workflow` scope to GitHub token
- qa-engineer: add Redis MCP to tools list
```

---

## 10. Noether Integration

Noether is Caloron's computation runtime. Where Caloron manages *which agents do what and when*, Noether manages *how computations are performed verifiably*.

### 10.1 When Agents Use Noether

Agents call Noether when they need to perform computation that benefits from:

- **Reproducibility** — the same input should always produce the same output
- **Reuse** — a computation that has been done before should not be redone
- **Verification** — the output type needs to be guaranteed before passing to the next stage

Typical use cases in a software development context:

- Parsing and validating API specifications
- Running static analysis on generated code
- Generating structured documentation from code
- Searching and comparing code patterns
- Any data transformation with a defined input/output type

### 10.2 Noether as an MCP

From the agent's perspective, Noether is just another MCP. The agent calls `noether compose "parse this OpenAPI spec and return a structured summary"` and receives a verified result. The agent does not know about stages, type signatures, or Nix derivations.

```yaml
# In agent definition
mcps:
  - url: "http://localhost:8080/mcp"
    name: "noether"
```

The Noether MCP server is started by the Caloron daemon as a local service and is available to all agents during the sprint.

### 10.3 What Caloron Learns from Noether

The structured feedback comment includes `tools_used`, which lists Noether stages used by each agent. The Retro Engine analyzes this to:

- Identify the most commonly used stages (high reuse = high value stages)
- Detect tasks where no Noether stages were used but could have been (token efficiency improvement opportunity)
- Track token savings over time as the Noether store matures

---

## 11. The PO Agent and Kickoff

### 11.1 Kickoff Flow

The kickoff is the first phase of every sprint. It is interactive — the PO Agent collaborates with the human to define the sprint scope before generating the DAG.

```
Human → caloron kickoff "implement user authentication"
           ↓
        PO Agent spawned
           ↓
        PO Agent analyzes current repository state:
          - open issues
          - recent commits
          - existing code structure
           ↓
        PO Agent engages human in dialogue:
          "I see the project has a User model but no auth layer.
           For authentication, should I prioritize:
           1. JWT-based stateless auth
           2. Session-based auth with Redis
           3. OAuth integration
           Which approach fits your architecture?"
           ↓
        Human responds
           ↓
        PO Agent generates draft DAG
           ↓
        PO Agent presents DAG summary to human:
          "I propose a sprint with 3 backend tasks, 1 QA task,
           and 1 review task. Estimated duration: 48 hours.
           The critical path is: JWT impl → Session store → Integration tests.
           Do you want to proceed, modify, or cancel?"
           ↓
        Human approves
           ↓
        PO Agent writes dag.json to caloron-meta
        PO Agent creates issues in project repo
        PO Agent posts sprint start comment
           ↓
        Caloron daemon loads DAG and starts Git Monitor
```

### 11.2 PO Agent System Prompt

```
You are the Product Owner agent for a software development sprint managed by Caloron.

Your responsibilities:
1. Analyze the current state of the project repository
2. Collaborate with the human operator to define clear sprint goals
3. Generate a DAG (Directed Acyclic Graph) that describes the tasks, agents, and dependencies
4. Create well-specified GitHub issues for each task

When creating tasks, always specify:
- Clear acceptance criteria
- Expected inputs and outputs
- Dependencies on other tasks
- Which agent role should handle it
- Estimated complexity (S/M/L)

When creating issues, always include:
- A "Definition of Done" section
- An "Error handling" section for tasks involving external APIs
- A "Dependencies" section linking to other issues

Your DAG must be valid JSON matching the schema in caloron-meta/schema/dag.schema.json.
Never start the sprint until the human has explicitly approved the DAG.

Output the DAG as a JSON code block when ready for approval.
```

---

## 12. Repository Structure

Caloron v2 involves three distinct repositories.

### 12.1 The Caloron Daemon Repository

```
caloron/
├── Cargo.toml
├── Cargo.lock
├── flake.nix                    # Nix flake for reproducible builds
├── src/
│   ├── main.rs                  # Daemon entry point
│   ├── config.rs                # Configuration loading
│   ├── daemon/
│   │   ├── mod.rs
│   │   ├── server.rs            # Unix socket server
│   │   └── state.rs             # Global daemon state
│   ├── dag/
│   │   ├── mod.rs
│   │   ├── engine.rs            # DAG loading and evaluation
│   │   ├── state.rs             # Runtime DAG state machine
│   │   └── types.rs             # DAG data types
│   ├── git/
│   │   ├── mod.rs
│   │   ├── monitor.rs           # Git event loop
│   │   ├── events.rs            # Event type definitions
│   │   └── worktree.rs          # Git worktree management
│   ├── agent/
│   │   ├── mod.rs
│   │   ├── definition.rs        # Agent YAML parsing
│   │   ├── spawner.rs           # Nix env + worktree creation
│   │   ├── harness.rs           # Harness process management
│   │   └── health.rs            # Health contract types
│   ├── supervisor/
│   │   ├── mod.rs
│   │   ├── health_monitor.rs    # Health check loop
│   │   ├── conflict_resolver.rs # Loop detection and mediation
│   │   └── escalation.rs        # Human escalation gateway
│   ├── retro/
│   │   ├── mod.rs
│   │   ├── collector.rs         # Feedback comment parser
│   │   ├── analyzer.rs          # Pattern detection
│   │   └── report.rs            # Report generation
│   ├── noether/
│   │   ├── mod.rs
│   │   └── client.rs            # Noether MCP client
│   └── nix/
│       ├── mod.rs
│       ├── generator.rs         # Nix expression generator
│       └── builder.rs           # Nix build execution
├── tests/
│   ├── integration/
│   └── unit/
└── caloron-harness/             # The agent harness (separate binary)
    ├── Cargo.toml
    └── src/
        ├── main.rs
        ├── heartbeat.rs
        └── feedback_enforcer.rs
```

### 12.2 The Caloron Metadata Repository (`caloron-meta`)

```
caloron-meta/
├── README.md
├── schema/
│   └── dag.schema.json          # JSON Schema for DAG validation
├── agents/                      # Agent definition files
│   ├── product-owner.yaml
│   ├── backend-developer.yaml
│   ├── senior-reviewer.yaml
│   ├── qa-engineer.yaml
│   ├── frontend-developer.yaml
│   └── supervisor.yaml
├── tasks/                       # Issue templates
│   ├── feature-implementation.md
│   ├── bug-fix.md
│   ├── code-review.md
│   └── integration-testing.md
├── state/                       # Runtime state (gitignored in prod)
│   └── sprint-{id}.json
├── retro/                       # Retro reports
│   └── sprint-{id}.md
└── sprints/                     # Historical sprint DAGs
    └── sprint-{id}/
        ├── dag.json
        └── summary.md
```

### 12.3 The Project Repository

The project repository is a normal git repository. Caloron adds:

```
project-repo/
├── ... normal project files ...
├── .caloron/
│   ├── .gitignore               # ignores worktrees/
│   └── worktrees/               # git worktrees (gitignored)
│       ├── backend-developer-sprint-2026-04-w2/
│       └── qa-engineer-sprint-2026-04-w2/
└── .github/
    └── labels.yml               # Caloron label definitions (auto-created)
```

---

## 13. Data Models

### 13.1 Core Rust Types

```rust
// src/dag/types.rs

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Sprint {
    pub id: String,
    pub goal: String,
    pub start: DateTime<Utc>,
    pub max_duration_hours: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentNode {
    pub id: String,
    pub role: String,
    pub definition_path: PathBuf,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Task {
    pub id: String,
    pub title: String,
    pub assigned_to: String,         // agent id
    pub issue_template: PathBuf,
    pub depends_on: Vec<String>,     // task ids
    pub reviewed_by: Option<String>, // agent id
    pub github_issue_number: Option<u64>, // set after issue creation
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum TaskStatus {
    Pending,
    Ready,
    InProgress,
    InReview,
    Done,
    Blocked { reason: String },
    Cancelled { reason: String },
    HumanAssigned,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DagState {
    pub sprint: Sprint,
    pub tasks: HashMap<String, (Task, TaskStatus)>,
    pub agents: HashMap<String, AgentNode>,
    pub last_updated: DateTime<Utc>,
}
```

```rust
// src/agent/health.rs

#[derive(Debug, Clone)]
pub struct AgentHealth {
    pub agent_id: String,
    pub role: String,
    pub current_task_id: Option<String>,
    pub last_git_event: DateTime<Utc>,
    pub last_heartbeat: DateTime<Utc>,
    pub consecutive_errors: u32,
    pub error_types: Vec<ErrorType>,
    pub review_cycles: HashMap<String, u32>,
    pub status: AgentStatus,
    pub stall_threshold: Duration,
    pub spawn_time: DateTime<Utc>,
}

#[derive(Debug, Clone, PartialEq)]
pub enum AgentStatus {
    Idle,
    Working,
    Stalled { since: DateTime<Utc>, reason: StallReason },
    Blocked { reason: String },
    Failed { error: String },
    Completing,
    Destroyed,
}

#[derive(Debug, Clone, PartialEq)]
pub enum StallReason {
    NoGitActivity,
    NoHeartbeat,
    RepeatedErrors(ErrorType),
    ReviewLoopDetected { pr_id: String },
}

#[derive(Debug, Clone, PartialEq)]
pub enum ErrorType {
    CredentialsFailure { tool: String },
    RateLimited { tool: String },
    ToolUnavailable { tool: String },
    Unknown,
}

#[derive(Debug, Clone)]
pub enum HealthVerdict {
    Healthy,
    ProcessDead,
    Stalled(StallReason),
    ReviewLoopDetected(String),
}
```

```rust
// src/git/events.rs

#[derive(Debug, Clone)]
pub enum GitEvent {
    IssueOpened { number: u64, title: String, labels: Vec<String> },
    IssueLabeled { number: u64, label: String },
    IssueClosed { number: u64, closer: String },
    PrOpened { number: u64, title: String, linked_issue: Option<u64>, author: String },
    PrReviewSubmitted { pr_number: u64, reviewer: String, state: ReviewState, body: String },
    PrMerged { number: u64 },
    CommentCreated { issue_number: u64, body: String, author: String },
    PushReceived { branch: String, author: String, commit_sha: String },
}

#[derive(Debug, Clone, PartialEq)]
pub enum ReviewState {
    Approved,
    ChangesRequested,
    Commented,
}
```

### 13.2 Configuration

```toml
# caloron.toml — project-level configuration

[project]
name = "my-project"
repo = "owner/repo"
meta_repo = "owner/caloron-meta"

[github]
token_env = "GITHUB_TOKEN"         # env var name, not the token itself
polling_interval_seconds = 30      # how often to poll for git events
                                    # (webhook support planned for v2.1)

[noether]
enabled = true
endpoint = "http://localhost:8080"

[supervisor]
stall_default_threshold_minutes = 20
max_review_cycles = 3
escalation_method = "github_issue"  # or "slack" (planned)

[retro]
enabled = true
auto_run = true                     # run retro automatically at sprint end
output_format = "markdown"

[nix]
enabled = true
extra_nixpkgs_config = ""           # additional nixpkgs configuration
cache_url = ""                      # optional binary cache

[llm]
default_model = "claude-sonnet-4"
default_max_tokens = 8192
api_key_env = "ANTHROPIC_API_KEY"
```

---

## 14. Implementation Roadmap

### Phase Overview

| Phase | Name | Duration | Outcome |
|---|---|---|---|
| 0 | Foundation | 2 weeks | Rust project structure, Nix integration, Git event model |
| 1 | Agent Lifecycle | 3 weeks | Agent spawning/destruction, harness, health contracts |
| 2 | Supervisor | 3 weeks | Health monitor, stall detection, intervention playbook |
| 3 | DAG Engine | 2 weeks | DAG loading, state machine, dependency resolution |
| 4 | Git Monitor | 2 weeks | Event loop, label management, issue/PR handlers |
| 5 | PO Agent + Kickoff | 2 weeks | Kickoff flow, DAG generation, issue creation |
| 6 | Retro Engine | 2 weeks | Feedback collection, pattern analysis, report generation |
| 7 | Noether Integration | 1 week | Noether MCP client, feedback integration |
| 8 | Hardening | 2 weeks | Error handling, recovery, observability, load testing |

**Total: ~19 weeks**

---

### Phase 0 — Foundation (Weeks 1–2)

**Goal:** a compiling Rust project with the core data models, Nix build, and a working connection to GitHub.

#### 0.1 Rust project setup

- Initialize Cargo workspace with crates: `caloron-daemon`, `caloron-harness`, `caloron-types`
- `caloron-types` contains all shared data types (DAG, health, events) — no business logic
- `caloron-daemon` contains the main daemon binary
- `caloron-harness` contains the agent harness binary
- Write `flake.nix` for reproducible builds of both binaries

Key dependencies:
```toml
[dependencies]
tokio = { version = "1", features = ["full"] }    # async runtime
serde = { version = "1", features = ["derive"] }  # serialization
serde_json = "1"                                   # JSON
serde_yaml = "0.9"                                 # YAML for agent definitions
chrono = { version = "0.4", features = ["serde"] } # timestamps
octocrab = "0.39"                                  # GitHub API client
tracing = "0.1"                                    # structured logging
tracing-subscriber = "0.3"
anyhow = "1"                                       # error handling
clap = { version = "4", features = ["derive"] }    # CLI
```

#### 0.2 Core data types

- Implement all types in `caloron-types/src/`: `Sprint`, `AgentNode`, `Task`, `TaskStatus`, `DagState`, `AgentHealth`, `AgentStatus`, `GitEvent`, `ReviewState`, `ErrorType`
- All types must implement `Serialize`, `Deserialize`, `Debug`, `Clone`
- Write unit tests for all state machine transitions

#### 0.3 GitHub client

- Implement `GitHubClient` wrapper around `octocrab`
- Methods needed: `create_issue`, `add_label`, `create_comment`, `list_events`, `assign_reviewer`
- Implement polling loop: fetch events newer than last poll timestamp
- Handle rate limiting gracefully: exponential backoff with jitter

#### 0.4 Nix shell generator

- Implement `NixGenerator` that takes an `AgentDefinition` and produces a `shell.nix` string
- Write snapshot tests for generated Nix expressions
- Verify generated expressions are valid by running `nix-shell --dry-run`

#### 0.5 Configuration loading

- Implement `Config` struct and TOML loading from `caloron.toml`
- Implement validation: check required env vars exist, GitHub token is valid, meta repo is accessible

---

### Phase 1 — Agent Lifecycle (Weeks 3–5)

**Goal:** the daemon can spawn and destroy agents as Nix environments with git worktrees.

#### 1.1 Agent definition parser

- Implement YAML loading for `AgentDefinition`
- Validate required fields: `name`, `llm`, `system_prompt`, `tools`
- Validate tool names against known tool registry
- Write tests with valid and invalid agent definitions

#### 1.2 Git worktree management

- Implement `WorktreeManager` with methods: `create`, `remove`, `list`
- Handle edge cases: worktree already exists (sprint resume), dirty worktree on removal
- Write integration tests that actually create and remove worktrees in a temp git repo

#### 1.3 Agent spawner

- Implement the full spawn sequence (see section 8.4)
- Generate Nix shell expression from agent definition
- Build Nix environment (async, with timeout)
- Create git worktree
- Start harness process
- Register health contract

- Implement the full destruction sequence (see section 8.5)

#### 1.4 The harness binary

- Implement `caloron-harness` as a thin wrapper around the configured LLM CLI
- Heartbeat loop: send heartbeat to daemon socket every 60 seconds
- Error capture: detect repeated identical errors and report to daemon
- Feedback enforcer: on exit, verify feedback comment exists; if not, prompt LLM to generate one before allowing exit
- Unix socket client: connect to daemon socket and send/receive messages

#### 1.5 Daemon socket server

- Implement Unix socket server in daemon
- Handle heartbeat messages: update `last_heartbeat` in health contract
- Handle status messages: update agent status
- Handle error messages: increment error counter, categorize error type
- Handle completion messages: trigger task completion flow

---

### Phase 2 — Supervisor (Weeks 6–8)

**Goal:** the Supervisor can detect stalled agents and execute the intervention playbook. This is the highest-priority component — it directly fixes the v1 failure mode.

#### 2.1 Health monitor

- Implement the health check loop (runs every 60 seconds)
- Implement `evaluate_health()` function for each agent
- Implement all `HealthVerdict` variants
- Write unit tests for each stall detection condition with time-mocked inputs

#### 2.2 Stall intervention — No Git Activity

- Implement probe comment posting
- Implement restart flow: destroy agent, respawn with same task context
- Implement reassignment flow: find alternative agent role in DAG, create new assignment
- Track intervention history per task to avoid infinite retry loops

#### 2.3 Stall intervention — Credentials Failure

- Implement escalation issue creation with structured format
- Implement monitoring loop for human response
- Implement resume flow when human posts resolution comment

#### 2.4 Review loop detection and mediation

- Implement review cycle counter per PR
- Implement `analyze_review_loop()`: read PR thread, extract disagreement using LLM
- Implement mediation comment posting
- Implement escalation flow if mediation fails

#### 2.5 Supervisor agent harness

- The Supervisor runs as a special agent with access to:
  - Full project repository (read-only for most operations)
  - `caloron-meta` repository (read-write for logging decisions)
  - Daemon socket with elevated permissions (can spawn/destroy agents)
- Implement Supervisor-specific tools: `inspect_agent_health`, `post_mediation`, `escalate_to_human`, `restart_agent`, `reassign_task`

---

### Phase 3 — DAG Engine (Weeks 9–10)

**Goal:** the daemon can load a DAG, track task state, and resolve dependencies correctly.

#### 3.1 DAG loader and validator

- Implement JSON loading for `DagState` from `dag.json`
- Validate against JSON Schema
- Validate semantic constraints: no cycles, all agent references valid, all task references valid
- Write tests with valid DAGs, cyclic DAGs, and DAGs with invalid references

#### 3.2 State machine

- Implement all state transitions with their preconditions
- Implement `evaluate_unblocked()` for dependency resolution
- Implement state persistence: write `state/sprint-{id}.json` after each transition
- Implement state recovery: reload state from file on daemon restart (sprint resume)

#### 3.3 DAG query API

- `get_task_by_issue_number(n: u64) → Option<Task>`
- `get_agent_for_role(role: &str) → Option<AgentNode>`
- `get_reviewer_for_task(task_id: &str) → Option<AgentNode>`
- `get_unblocked_tasks() → Vec<Task>`
- `get_tasks_in_status(status: TaskStatus) → Vec<Task>`
- `is_sprint_complete() → bool`

---

### Phase 4 — Git Monitor (Weeks 11–12)

**Goal:** the daemon responds correctly to all Git events and drives the DAG state machine.

#### 4.1 Event polling loop

- Implement polling loop with configurable interval
- Implement event deduplication: track processed event IDs to avoid re-processing
- Implement event ordering: process events in chronological order
- Handle GitHub API pagination for events

#### 4.2 Event handlers

Implement a handler for each event type defined in `GitEvent`:

- `handle_issue_opened`
- `handle_issue_labeled`
- `handle_issue_closed`
- `handle_pr_opened`
- `handle_pr_review_submitted`
- `handle_pr_merged`
- `handle_comment_created` (including feedback comment detection)
- `handle_push_received`

Each handler must be idempotent — calling it twice with the same event must produce the same result.

#### 4.3 Label management

- Implement `LabelManager` that ensures Caloron labels exist in the repository
- Create missing labels on first run
- Implement atomic label transitions: remove old status label, add new status label in a single operation

#### 4.4 Integration tests

- Use a real GitHub repository (or GitHub API mock) for integration tests
- Test the full flow: create issue → spawn agent → open PR → approve → merge → task done
- Test stall detection: create issue, assign agent, do not generate any activity, verify supervisor probe is sent

---

### Phase 5 — PO Agent and Kickoff (Weeks 13–14)

**Goal:** a human can run `caloron kickoff "goal"` and the PO Agent generates a valid DAG after interactive dialogue.

#### 5.1 Kickoff CLI command

- Implement `caloron kickoff <goal>` command
- Spawn PO Agent with kickoff system prompt
- Open interactive terminal session for human-PO dialogue
- Monitor PO Agent output for DAG JSON code block
- On DAG detection: validate, present summary, ask for human approval
- On approval: write `dag.json` to meta repo, create GitHub issues, start daemon

#### 5.2 PO Agent tools

The PO Agent needs specific tools for kickoff:
- `read_repository_state` — summarize open issues, recent commits, file structure
- `write_dag` — write validated DAG JSON to meta repo
- `create_issues_from_dag` — create all GitHub issues from DAG task definitions
- `list_available_agent_types` — return available agent definitions from meta repo

#### 5.3 Issue template rendering

- Implement template rendering: fill `{task_title}`, `{dependencies}`, `{acceptance_criteria}` placeholders
- Validate rendered issues before creation (minimum required sections present)

---

### Phase 6 — Retro Engine (Weeks 15–16)

**Goal:** at the end of a sprint, the Retro Engine produces a useful retro report automatically.

#### 6.1 Feedback collector

- Implement YAML parser for `caloron_feedback:` blocks in issue comments
- Fetch all sprint issues from GitHub
- Extract feedback from closing comments
- Build `SprintFeedback` struct

#### 6.2 Pattern analyzer

- Implement clarity analysis: group low-clarity tasks by template type
- Implement dependency discovery: extract runtime dependencies from blocker lists
- Implement tool gap analysis: identify unavailable tools from error reports
- Implement review loop analysis: correlate PR cycle counts with root causes

#### 6.3 Report generator

- Implement markdown report generation
- Implement suggestions extraction: actionable items for next sprint
- Write report to `caloron-meta/retro/sprint-{id}.md`
- Optionally post summary to project repository as a discussion

#### 6.4 Retro CLI command

- Implement `caloron retro` command (can run manually or automatically at sprint end)
- Support `--sprint-id` flag for retroactive analysis of past sprints

---

### Phase 7 — Noether Integration (Week 17)

**Goal:** agents can use Noether stages via MCP, and token savings are tracked in feedback.

#### 7.1 Noether MCP server startup

- Implement `NoetherService` that starts the Noether daemon as a subprocess
- Health check: verify Noether is responding before sprint starts
- Graceful shutdown at sprint end

#### 7.2 Noether client

- Implement `NoetherClient` with methods: `compose`, `search_stages`, `get_trace`
- Use `compose` in the Retro Engine to analyze feedback patterns (dogfooding)

#### 7.3 Feedback integration

- Parse `noether_stages_used` from feedback comments
- Track stage reuse rates in retro analysis
- Add "token savings via Noether" section to retro report

---

### Phase 8 — Hardening (Weeks 18–19)

**Goal:** the system is production-ready, handles errors gracefully, and provides enough observability to debug any issue.

#### 8.1 Error handling and recovery

- Implement daemon restart recovery: reload DAG state from `state/sprint-{id}.json`
- Implement agent crash recovery: detect harness process death, trigger Health Monitor stall flow
- Implement GitHub API failure handling: circuit breaker with fallback to cached state
- All error paths must log structured events (never panic in production)

#### 8.2 Observability

- Implement structured JSON logging via `tracing` with sprint_id, task_id, agent_role fields on every log line
- Implement `caloron status` CLI command: show current sprint state, all agent health contracts, recent events
- Implement `caloron logs <agent-role>` CLI command: tail agent harness logs
- Implement `caloron trace <task-id>` CLI command: show full event history for a task

#### 8.3 Credential management

- Never log credentials
- Validate all required credentials at daemon startup, fail fast with clear error messages
- Support credential rotation without daemon restart (re-read from env on next use)

#### 8.4 Load testing

- Test with 10 concurrent agents
- Verify GitHub API rate limiting is handled without dropping events
- Verify Health Monitor performance at 10 agents with 60-second check loop

---

## 15. Configuration Reference

### Environment Variables

| Variable | Required | Description |
|---|---|---|
| `GITHUB_TOKEN` | Yes | GitHub personal access token with `repo` and `workflow` scopes |
| `ANTHROPIC_API_KEY` | Yes (if using Claude) | Anthropic API key |
| `CALORON_META_REPO` | Yes | Path or URL of the caloron-meta repository |
| `CALORON_LOG_LEVEL` | No | `trace`, `debug`, `info` (default), `warn`, `error` |
| `CALORON_NOETHER_ENDPOINT` | No | Noether MCP endpoint (default: `http://localhost:8080`) |

### CLI Commands

| Command | Description |
|---|---|
| `caloron kickoff <goal>` | Start interactive kickoff with PO Agent |
| `caloron start` | Start the daemon with existing DAG |
| `caloron stop` | Gracefully stop the current sprint |
| `caloron status` | Show current sprint state and agent health |
| `caloron logs <role>` | Tail logs for a specific agent role |
| `caloron trace <task-id>` | Show full event history for a task |
| `caloron retro` | Run retro for the completed sprint |
| `caloron retro --sprint-id <id>` | Run retro for a specific past sprint |
| `caloron agent list` | List available agent definitions |
| `caloron agent validate <file>` | Validate an agent definition YAML |

---

## 16. Observability and Debugging

### 16.1 Structured Logging

Every log line includes the following fields:

```json
{
  "timestamp": "2026-04-08T10:23:45.123Z",
  "level": "INFO",
  "sprint_id": "sprint-2026-04-w2",
  "component": "git_monitor",
  "task_id": "task-1",
  "agent_role": "backend-developer",
  "event": "pr_opened",
  "pr_number": 42,
  "message": "PR opened by backend-developer, assigning reviewer"
}
```

### 16.2 The `caloron status` Output

```
Sprint: sprint-2026-04-w2
Goal: Implement user authentication
Started: 2026-04-08 09:00 UTC (6h 23m ago)

Tasks:
  ✅ task-1  JWT implementation          DONE       (backend-1, 47min, 14.2k tokens)
  ✅ task-2  Session store               DONE       (backend-2, 52min, 16.8k tokens)
  🔄 task-3  Integration tests          IN_REVIEW  (qa-1 → reviewer-1)
  ⏳ task-4  E2E test suite              PENDING    (depends on task-3)

Agents:
  backend-1     IDLE        last activity: 2h ago
  backend-2     IDLE        last activity: 1h 30m ago
  reviewer-1    WORKING     on PR #47, review cycle 1/3
  qa-1          IN_REVIEW   waiting for PR #47 feedback
  supervisor    MONITORING  last intervention: 3h ago (probe sent to backend-2)

Supervisor events (last 24h):
  10:14  Probe sent to backend-2 (no activity for 20min)
  10:18  backend-2 responded, resumed working
  
Noether: connected (127 stages in store, 6 reused this sprint)
```

### 16.3 Common Debugging Scenarios

**Agent stalled with no apparent reason**

```bash
caloron logs backend-developer    # check harness logs for errors
caloron trace task-1              # see full event timeline for the task
```

Look for: repeated identical LLM outputs (hallucination loop), tool errors not reported to daemon, heartbeat gaps.

**Supervisor escalating repeatedly for same issue**

Check `caloron-meta/retro/` for patterns — if the same task type consistently triggers escalation, it indicates a systemic issue with the task template or the agent definition.

**DAG not unblocking tasks after dependency completion**

```bash
caloron trace task-3    # verify task-1 and task-2 actually reached DONE status
```

The most common cause: the PR was merged but the linked issue was not closed, so the DAG did not receive the `issue.closed` event.

---

## 17. Security Model

### 17.1 Credential Isolation

Each agent's Nix environment receives only the credentials it needs, declared explicitly in its agent definition:

```yaml
credentials:
  - GITHUB_TOKEN           # all agents need this for git operations
  - ANTHROPIC_API_KEY      # LLM access
  # backend-developer also needs:
  - DATABASE_URL           # only agents that touch the database
```

An agent that is not configured with `DATABASE_URL` cannot access the database, even if the environment variable is set on the host. Nix's pure shell mode ensures only declared variables are available.

### 17.2 Git Permissions

Agents operate with a scoped GitHub token. The token has repository access but not organization-level access. For production use, consider using GitHub App tokens with per-repository scope.

Agents can:
- Create and comment on issues
- Open pull requests
- Push to branches prefixed with `agent/`

Agents cannot:
- Merge PRs directly (the orchestrator does this after approval)
- Delete branches or repositories
- Modify GitHub Actions workflows
- Access other repositories

### 17.3 Nix Sandbox

All agent processes run in Nix's pure shell mode. This provides:
- No access to host environment variables not explicitly passed
- No access to host filesystem outside the declared worktree path
- Reproducible environment: the agent cannot install packages at runtime

---

## 18. Open Questions and Deferred Decisions

| Question | Impact | Deferred to |
|---|---|---|
| Should Caloron use GitHub webhooks instead of polling? | Latency and reliability | Phase 4 — polling first, webhooks in v2.1 |
| How does Caloron handle multiple simultaneous sprints on the same repository? | Multi-team support | Post-Phase 8 |
| Should agent definitions be versioned in caloron-meta or in the project repo? | Repository structure | Phase 5 design review |
| What is the right stall threshold for different agent types? (reviewers are naturally slower) | Supervisor accuracy | Phase 2 — configurable per agent definition |
| Should the Supervisor have write access to the project repository directly, or only via the Git Protocol? | Security boundary | Phase 2 design review |
| How does sprint cancellation handle partially-merged PRs? | Data integrity | Phase 3 |
| Should retro suggestions be automatically applied to agent definitions, or always require human approval? | Automation risk | Phase 6 — human approval required in v1 |
| How does Caloron handle private repositories with self-hosted GitHub runners? | Enterprise deployment | Post-Phase 8 |

---

*Caloron — Agents that collaborate like engineers, not like scripts.*
