# Agent Definitions

Agents are defined as YAML files. Each file is a complete, declarative description of an agent's capabilities.

## Format

```yaml
name: backend-developer
version: "1.0"
description: "Implements backend features, writes tests, and opens PRs"

llm:
  model: default          # Model alias (resolved via caloron.toml)
  max_tokens: 8192
  temperature: 0.2        # Low for code generation

system_prompt: |
  You are a senior backend developer working on this project.
  You receive tasks as GitHub issues assigned to you.
  ...

tools:
  - github_mcp
  - noether
  - bash

mcps:
  - url: "https://github.mcp.claude.com/mcp"
    name: "github"

nix:
  packages:
    - nodejs_20
    - python311
    - rustc
    - cargo
  env:
    NODE_ENV: "test"

credentials:
  - GITHUB_TOKEN
  - ANTHROPIC_API_KEY

stall_threshold_minutes: 20    # Supervisor alert threshold
max_review_cycles: 3           # Max PR review rounds
```

## Fields

### Required

| Field | Description |
|-------|-------------|
| `name` | Unique identifier for the agent type |
| `version` | Semantic version |
| `llm.model` | Model name or alias |
| `system_prompt` | Instructions for the LLM |
| `tools` | List of available tools |

### Optional

| Field | Default | Description |
|-------|---------|-------------|
| `description` | `""` | Human-readable description |
| `llm.max_tokens` | `8192` | Max output tokens |
| `llm.temperature` | `0.2` | LLM temperature (0.0-2.0) |
| `mcps` | `[]` | MCP server connections |
| `nix.packages` | `[]` | Nix packages to include |
| `nix.env` | `{}` | Extra environment variables |
| `credentials` | `[]` | Env vars to inject via secrets file |
| `stall_threshold_minutes` | `20` | Minutes without git activity before stall alert |
| `max_review_cycles` | `3` | PR review cycles before supervisor intervenes |

## Known Tools

| Tool | Description |
|------|-------------|
| `github_mcp` | Read/write issues, PRs, comments |
| `noether` | Verified computation via Noether stages |
| `bash` | Run shell commands, tests, linters |
| `browser` | Web browsing |
| `filesystem` | File system operations |

Unknown tools generate a warning during validation but are not rejected.

## Nix Packages

The `nix.packages` list maps directly to nixpkgs attribute names. Common examples:

```yaml
nix:
  packages:
    - nodejs_20       # Node.js 20.x
    - python311       # Python 3.11
    - rustc           # Rust compiler
    - cargo           # Cargo build tool
    - go              # Go compiler
    - jdk21           # Java 21
    - postgresql_16   # PostgreSQL client
```

## Validation

```bash
caloron agent validate agents/my-agent.yaml
```

The validator checks:

- All required fields are present
- Tool names are in the known registry (warns if not)
- MCP configs have name and URL
- Nix package names are syntactically valid
- Temperature is in range [0.0, 2.0]
- Stall threshold is >= 5 minutes (warns if lower)
- Credentials list is non-empty (warns if empty)

## Example: Reviewer Agent

```yaml
name: senior-reviewer
version: "1.0"
description: "Reviews PRs for correctness and code quality"

llm:
  model: reviewer         # Uses the stronger model alias
  max_tokens: 8192
  temperature: 0.1        # Very low — consistent reviews

system_prompt: |
  You are a senior code reviewer. You receive pull requests to review.
  
  Review criteria:
  1. Correctness — does the code do what the issue asks?
  2. Tests — are there adequate tests?
  3. Code quality — follows existing patterns
  4. Security — no obvious vulnerabilities

  If changes are needed, be specific.
  If the PR is good, approve it.

tools:
  - github_mcp
  - bash

nix:
  packages:
    - nodejs_20
    - rustc
    - cargo

credentials:
  - GITHUB_TOKEN
  - ANTHROPIC_API_KEY

stall_threshold_minutes: 30   # Reviewers are naturally slower
max_review_cycles: 3
```
