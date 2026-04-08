# CLI Reference

## `caloron kickoff <goal>`

Start an interactive kickoff with the PO Agent.

```bash
caloron kickoff "implement user authentication"
```

The PO Agent analyzes the repository, asks clarifying questions, generates a DAG, and waits for human approval before creating issues.

## `caloron start`

Start the daemon with an existing DAG.

```bash
caloron start --dag dag.json
```

| Option | Default | Description |
|--------|---------|-------------|
| `--dag` | `dag.json` | Path to the DAG JSON file |

Requires `caloron.toml` and the `GITHUB_TOKEN` environment variable.

## `caloron stop`

Gracefully cancel the current sprint.

```bash
caloron stop
```

- In-progress tasks get cancellation comments
- Open PRs are labeled `caloron:sprint-cancelled` (not closed)
- Agents are destroyed, worktrees preserved
- Partial retro runs on completed tasks

## `caloron status`

Show current sprint state and task progress.

```bash
caloron status
```

Reads from the most recent state file in `state/`.

## `caloron retro`

Run the retro engine for a completed sprint.

```bash
caloron retro
caloron retro --sprint-id sprint-2026-04-w2
```

| Option | Default | Description |
|--------|---------|-------------|
| `--sprint-id` | Latest | Specific sprint to analyze |

Generates a markdown report in `retro/sprint-{id}.md`.

## `caloron logs <role>`

Tail logs for a specific agent.

```bash
caloron logs backend-developer
```

## `caloron trace <task-id>`

Show full event history for a task.

```bash
caloron trace task-1
```

## `caloron agent validate <file>`

Validate an agent definition YAML file.

```bash
caloron agent validate agents/backend-developer.yaml
```

Checks required fields, tool names, Nix packages, temperature range, and credentials.

## `caloron agent build <file>`

Build the Nix environment for an agent definition.

```bash
caloron agent build agents/backend-developer.yaml
```

With `nix.enabled = true`: writes `flake.nix` and builds via `nix develop`.
With `nix.enabled = false`: prints the generated devShell expression.

## `caloron agent list`

List available agent definitions from the meta repository.

```bash
caloron agent list
```
