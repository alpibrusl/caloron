# Configuration

Caloron is configured via `caloron.toml` in the project root.

## Full Reference

```toml
[project]
name = "my-project"
repo = "owner/repo"              # GitHub owner/repo
meta_repo = "owner/caloron-meta" # Agent definitions repository

[github]
token_env = "GITHUB_TOKEN"       # Env var holding the token (not the token itself)
polling_interval_seconds = 5     # Event polling interval (default: 5)
webhook_enabled = false           # Use webhooks instead of polling
webhook_port = 9443               # Webhook listener port
webhook_secret_env = "CALORON_WEBHOOK_SECRET"

[noether]
enabled = true                    # Enable Noether integration
endpoint = ""                     # Remote registry URL (optional)
binary = "noether"                # Path to noether binary

[supervisor]
stall_default_threshold_minutes = 20  # Default stall detection threshold
max_review_cycles = 3                 # Max PR review cycles before intervention
escalation_method = "github_issue"    # How to contact humans

[retro]
enabled = true
auto_run = true                   # Run retro automatically at sprint end
output_format = "markdown"

[nix]
enabled = true                    # Enable Nix environment isolation

[llm]
api_key_env = "ANTHROPIC_API_KEY"

[llm.aliases]                     # Model alias system
default = "claude-sonnet-4-6"     # Agents referencing "default" get this model
fast = "claude-haiku-4-5"
strong = "claude-opus-4-6"
reviewer = "claude-opus-4-6"
```

## Model Aliases

Agent definitions reference model aliases instead of concrete model IDs:

```yaml
# In agent definition
llm:
  model: default  # Resolved via [llm.aliases] at spawn time
```

A single change to `[llm.aliases]` updates all agents using that alias.

## Environment Variables

| Variable | Required | Description |
|----------|----------|-------------|
| `GITHUB_TOKEN` | Yes | GitHub token with `repo` and `workflow` scopes |
| `ANTHROPIC_API_KEY` | Yes | Anthropic API key for LLM agents |
| `CALORON_LOG_LEVEL` | No | `trace`, `debug`, `info` (default), `warn`, `error` |
| `CALORON_WEBHOOK_SECRET` | If webhooks | Webhook signature verification secret |

## Secrets Handling

Caloron never stores secrets in Nix expressions or environment variables visible in `/proc`. Instead:

1. Secrets are written to a temporary file (`/run/caloron/secrets/{agent}.env`) with mode `0600`
2. The file path is passed to the agent as `CALORON_SECRETS_FILE`
3. The harness reads the file on startup and deletes it immediately
