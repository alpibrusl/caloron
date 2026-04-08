# Label Taxonomy

All Caloron-managed labels use the `caloron:` prefix to avoid collision with project labels.

## Labels

| Label | Set by | Meaning |
|-------|--------|---------|
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
| `caloron:sprint-cancelled` | Orchestrator | Sprint was cancelled; PR preserved for next sprint |

## Automatic Setup

On first run, the daemon creates any missing labels in the repository with the color `#0366d6`.
