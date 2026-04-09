# Scalability

## Current Architecture: Single Machine

Today, Caloron runs on a single machine:

```
┌─────────────────────────────────────┐
│  Single machine                     │
│                                     │
│  Orchestrator (Python process)      │
│    ├── PO Agent (Claude subprocess) │
│    ├── Agent 1 (Claude subprocess)  │
│    ├── Agent 2 (Claude subprocess)  │
│    ├── Reviewer (Claude subprocess) │
│    └── bubblewrap sandboxes         │
│                                     │
│  Gitea (Docker container)           │
│  Learnings (local JSON file)        │
│  Worktrees (local filesystem)       │
└─────────────────────────────────────┘
```

This works for sprints with 2-5 agents. Beyond that, you hit:

- **CPU/memory limits** — each Claude Code subprocess uses ~200MB RAM
- **Sequential bottleneck** — agents on the same dependency level run one at a time
- **Single point of failure** — if the machine goes down, the sprint stops

## What Can Scale Without Changes

Some parts are already network-ready:

| Component | Current | Scales to |
|---|---|---|
| Git hosting | Local Gitea Docker | GitHub.com, Gitea cluster, GitLab |
| LLM API | Claude Pro (local CLI) | Any API-based LLM (Anthropic API, Vertex, Bedrock) |
| Supervisor | In-process timeout | Works anywhere (just HTTP calls to Gitea) |
| Retro | Reads from Gitea comments | Works against any Git hosting |

## Path to Multi-Machine

### Level 1: Parallel Agents on One Machine

The DAG already supports parallel tasks. The orchestrator runs them sequentially today, but could run independent tasks concurrently:

```python
# Current (sequential):
for task in ready_tasks:
    run_agent(task)

# Level 1 (parallel on one machine):
import asyncio
await asyncio.gather(*[run_agent(task) for task in ready_tasks])
```

Effort: ~20 lines. Gains: 2-3x speedup on DAGs with parallel branches.

### Level 2: Distributed Agent Workers

Separate the orchestrator from the agent execution:

```
┌─────────────┐      ┌──────────────┐
│ Coordinator │─────→│ Worker 1     │
│             │      │ (bubblewrap) │
│ Orchestrator│─────→│ Worker 2     │
│ Supervisor  │      │ (bubblewrap) │
│ Retro       │─────→│ Worker 3     │
└─────────────┘      └──────────────┘
       │
       ▼
┌─────────────┐
│ Gitea/GitHub│
│ (shared)    │
└─────────────┘
```

Each worker needs:
- Claude Code (or API key)
- bubblewrap
- git
- Network access to Gitea and the coordinator

Communication: the orchestrator posts a task to a job queue (Redis, RabbitMQ, or simple HTTP). Workers poll for tasks, execute them, push results to Gitea.

Effort: ~200 lines (queue + worker process). Gains: N machines = N concurrent agents.

### Level 3: Kubernetes Deployment

```yaml
# Agent worker as a K8s Job
apiVersion: batch/v1
kind: Job
metadata:
  name: caloron-agent-task-1
spec:
  template:
    spec:
      containers:
      - name: agent
        image: caloron-agent:latest
        env:
        - name: CALORON_TASK_ID
          value: "task-1"
        - name: CALORON_GITEA_URL
          value: "https://gitea.internal"
        securityContext:
          readOnlyRootFilesystem: true
          runAsNonRoot: true
```

Each agent is a K8s Job with:
- Resource limits (CPU, memory)
- Network policies (only Gitea + LLM API)
- Read-only root filesystem (better than bubblewrap)
- Automatic retry on failure

The orchestrator becomes a K8s controller that creates Jobs from the DAG.

Effort: Significant (~1000 lines). Gains: auto-scaling, resource isolation, monitoring.

### Level 4: caloron-noether (Native Distribution)

The [caloron-noether](https://github.com/alpibrusl/caloron-noether) variant is designed for distribution:

- **noether-scheduler** already supports remote execution via registry
- **KV store** can be backed by networked storage
- **Stages** are stateless — can run on any machine with the Noether runtime
- **The shell** (heartbeat + spawn) can run per-worker

This is the natural scaling path if you're using Noether.

## What's Hard to Distribute

| Challenge | Why |
|---|---|
| **Git worktrees** | Shared filesystem needed, or each worker clones independently (slow) |
| **bubblewrap** | Linux-only, requires unprivileged user namespaces per worker |
| **Claude Pro subscription** | Per-user, not per-machine — API keys scale better |
| **Learnings store** | Currently local JSON — needs shared storage (Postgres, S3) |
| **PO Agent** | Interactive on first run — needs human approval before distributing |

## Recommendations

| Sprint size | Recommendation |
|---|---|
| 1-5 agents | Single machine (current) |
| 5-15 agents | Level 1: parallel on one machine |
| 15-50 agents | Level 2: distributed workers + Redis queue |
| 50+ agents | Level 3: Kubernetes + Anthropic API |

For most use cases (OTA features, electromobility services), Level 1-2 is sufficient. A 10-task sprint with 5 parallel agents on a single machine completes in minutes.
