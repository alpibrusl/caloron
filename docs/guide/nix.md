# Nix Environments

Caloron uses Nix to provide reproducible, isolated environments for each agent.

## Why Nix

| Concern | Docker (v1) | Nix (v2) |
|---------|------------|----------|
| Config changes | Rebuild container | Instant (re-evaluate derivation) |
| Credentials | Injected at build time | Injected at spawn time via file |
| Agent networking | Container networking needed | Not needed (agents use Git) |
| Cache | Layer cache, can be stale | Content-addressed Nix store |
| macOS/Linux | Docker Desktop overhead | Native on both |

## How It Works

When an agent is spawned:

1. Caloron generates a standalone `flake.nix` at `.caloron/nix/{agent-name}/flake.nix`
2. Runs `nix develop .#agent-{name} --command echo ready` to build and cache the environment
3. Starts the harness inside the Nix env: `nix develop .#agent-{name} --impure --command caloron-harness start`

### Generated Flake

```nix
{
  description = "Caloron agent environment: backend-developer";

  inputs = {
    nixpkgs.url = "github:NixOS/nixpkgs/nixpkgs-unstable";
  };

  outputs = { self, nixpkgs }:
    let
      supportedSystems = [ "x86_64-linux" "aarch64-linux" "x86_64-darwin" "aarch64-darwin" ];
      forAllSystems = nixpkgs.lib.genAttrs supportedSystems;
    in {
      devShells = forAllSystems (system:
        let
          pkgs = import nixpkgs { inherit system; };
        in {
          "agent-backend-developer" = pkgs.mkShell {
            name = "caloron-agent-backend-developer";

            nativeBuildInputs = with pkgs; [
              git
              nodejs_20
              rustc
              cargo
            ];

            shellHook = ''
              export CALORON_AGENT_ROLE="backend-developer"
              export CALORON_DAEMON_SOCKET="/run/caloron/daemon.sock"
              export CALORON_WORKTREE="/project/.caloron/worktrees/backend-1-sprint-1"
              export CALORON_TASK_ID="issue-42"
              export CALORON_SECRETS_FILE="/run/caloron/secrets/backend-1.env"
              export NODE_ENV="test"
            '';
          };
        });
    };
}
```

### What Nix Provides

- **Tool isolation**: Only packages declared in `nativeBuildInputs` are available
- **Reproducibility**: Same `flake.nix` produces identical environments everywhere
- **Caching**: The Nix store deduplicates. Second spawn of the same agent type is instant.

### What Nix Does NOT Provide

- **Filesystem sandboxing**: Agents can access the host filesystem (worktree, daemon socket, secrets). This is intentional — the `--impure` flag is required.
- **Network isolation**: Agents need to reach GitHub and LLM APIs.

## Disabling Nix

Set `nix.enabled = false` in `caloron.toml` to run agents directly without Nix isolation. This is useful for development and testing. The harness receives environment variables directly instead of via the Nix shellHook.

## Preview Without Building

```bash
# Show the generated Nix expression (with nix.enabled = false)
caloron agent build examples/agents/backend-developer.yaml
```

## Worktrees

Each agent gets a dedicated git worktree:

```
.caloron/worktrees/
  backend-1-sprint-2026-04-w2/    # backend-1's isolated copy
  qa-1-sprint-2026-04-w2/         # qa-1's isolated copy
```

Worktrees share the git object store but have separate working directories. Agents cannot see each other's uncommitted work.

Branch naming: `agent/{agent-id}/{sprint-id}`
