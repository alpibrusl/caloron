use std::fmt::Write;

use caloron_types::agent::AgentDefinition;

/// Generates Nix flake devShell expressions from agent definitions.
pub struct NixGenerator;

/// Parameters injected at spawn time (not known at generation time).
pub struct SpawnParams {
    pub daemon_socket: String,
    pub worktree_path: String,
    pub task_id: String,
    pub secrets_file_path: String,
}

impl NixGenerator {
    /// Create default spawn params for preview/build commands.
    pub fn default_params(agent_name: &str) -> SpawnParams {
        SpawnParams {
            daemon_socket: "/run/caloron/daemon.sock".into(),
            worktree_path: format!(".caloron/worktrees/{agent_name}"),
            task_id: "(none)".into(),
            secrets_file_path: format!("/run/caloron/secrets/{agent_name}.env"),
        }
    }

    /// Generate a Nix flake devShell expression for an agent.
    pub fn generate_devshell(agent: &AgentDefinition, params: &SpawnParams) -> String {
        let mut out = String::new();

        writeln!(out, "# Generated from agents/{}.yaml", agent.name).unwrap();
        writeln!(out, "{{ pkgs, ... }}:").unwrap();
        writeln!(out).unwrap();
        writeln!(out, "pkgs.mkShell {{").unwrap();
        writeln!(out, "  name = \"caloron-agent-{}\";", agent.name).unwrap();
        writeln!(out).unwrap();

        // nativeBuildInputs (not buildInputs — per Addendum E1)
        writeln!(out, "  nativeBuildInputs = with pkgs; [").unwrap();
        writeln!(out, "    git").unwrap();
        writeln!(out, "    caloron-harness").unwrap();
        for pkg in &agent.nix.packages {
            writeln!(out, "    {pkg}").unwrap();
        }
        writeln!(out, "  ];").unwrap();
        writeln!(out).unwrap();

        // shellHook — non-secret config only (secrets via file, per Addendum R3)
        writeln!(out, "  shellHook = ''").unwrap();
        writeln!(
            out,
            "    export CALORON_AGENT_ROLE=\"{}\"",
            agent.name
        )
        .unwrap();
        writeln!(
            out,
            "    export CALORON_DAEMON_SOCKET=\"{}\"",
            params.daemon_socket
        )
        .unwrap();
        writeln!(
            out,
            "    export CALORON_WORKTREE=\"{}\"",
            params.worktree_path
        )
        .unwrap();
        writeln!(
            out,
            "    export CALORON_TASK_ID=\"{}\"",
            params.task_id
        )
        .unwrap();
        writeln!(
            out,
            "    export CALORON_SECRETS_FILE=\"{}\"",
            params.secrets_file_path
        )
        .unwrap();

        // Custom env vars from agent definition
        for (key, value) in &agent.nix.env {
            writeln!(out, "    export {key}=\"{value}\"").unwrap();
        }

        writeln!(out, "  '';").unwrap();
        writeln!(out, "}}").unwrap();

        out
    }

    /// Generate a complete flake.nix that wraps one or more agent devShells.
    pub fn generate_agent_flake(agents: &[(&AgentDefinition, &SpawnParams)]) -> String {
        let mut out = String::new();

        writeln!(out, "{{").unwrap();
        writeln!(out, "  description = \"Caloron agent environments\";").unwrap();
        writeln!(out).unwrap();
        writeln!(out, "  inputs = {{").unwrap();
        writeln!(
            out,
            "    nixpkgs.url = \"github:NixOS/nixpkgs/nixpkgs-unstable\";"
        )
        .unwrap();
        writeln!(out, "  }};").unwrap();
        writeln!(out).unwrap();
        writeln!(
            out,
            "  outputs = {{ self, nixpkgs }}: let"
        )
        .unwrap();
        writeln!(
            out,
            "    supportedSystems = [ \"x86_64-linux\" \"aarch64-linux\" \"x86_64-darwin\" \"aarch64-darwin\" ];"
        )
        .unwrap();
        writeln!(
            out,
            "    forAllSystems = nixpkgs.lib.genAttrs supportedSystems;"
        )
        .unwrap();
        writeln!(out, "  in {{").unwrap();
        writeln!(
            out,
            "    devShells = forAllSystems (system: let"
        )
        .unwrap();
        writeln!(
            out,
            "      pkgs = import nixpkgs {{ inherit system; }};"
        )
        .unwrap();
        writeln!(out, "    in {{").unwrap();

        for (agent, params) in agents {
            writeln!(out).unwrap();
            writeln!(
                out,
                "      \"agent-{}\" = {};",
                agent.name,
                Self::generate_devshell_inline(agent, params)
            )
            .unwrap();
        }

        writeln!(out, "    }});").unwrap();
        writeln!(out, "  }};").unwrap();
        writeln!(out, "}}").unwrap();

        out
    }

    /// Generate an inline devShell expression (for embedding in a flake).
    fn generate_devshell_inline(agent: &AgentDefinition, params: &SpawnParams) -> String {
        let mut out = String::new();

        write!(out, "pkgs.mkShell {{\n").unwrap();
        writeln!(out, "        name = \"caloron-agent-{}\";", agent.name).unwrap();
        writeln!(out, "        nativeBuildInputs = with pkgs; [").unwrap();
        writeln!(out, "          git").unwrap();
        for pkg in &agent.nix.packages {
            writeln!(out, "          {pkg}").unwrap();
        }
        writeln!(out, "        ];").unwrap();
        writeln!(out, "        shellHook = ''").unwrap();
        writeln!(
            out,
            "          export CALORON_AGENT_ROLE=\"{}\"",
            agent.name
        )
        .unwrap();
        writeln!(
            out,
            "          export CALORON_DAEMON_SOCKET=\"{}\"",
            params.daemon_socket
        )
        .unwrap();
        writeln!(
            out,
            "          export CALORON_WORKTREE=\"{}\"",
            params.worktree_path
        )
        .unwrap();
        writeln!(
            out,
            "          export CALORON_TASK_ID=\"{}\"",
            params.task_id
        )
        .unwrap();
        writeln!(
            out,
            "          export CALORON_SECRETS_FILE=\"{}\"",
            params.secrets_file_path
        )
        .unwrap();
        for (key, value) in &agent.nix.env {
            writeln!(out, "          export {key}=\"{value}\"").unwrap();
        }
        writeln!(out, "        '';").unwrap();
        write!(out, "      }}").unwrap();

        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    fn test_agent() -> AgentDefinition {
        AgentDefinition {
            name: "backend-developer".into(),
            version: "1.0".into(),
            description: "Implements backend features".into(),
            llm: caloron_types::agent::LlmConfig {
                model: "claude-sonnet-4-6".into(),
                max_tokens: 8192,
                temperature: 0.2,
            },
            system_prompt: "You are a backend developer.".into(),
            tools: vec!["github_mcp".into(), "bash".into()],
            mcps: vec![],
            nix: caloron_types::agent::NixConfig {
                packages: vec!["nodejs_20".into(), "rustc".into(), "cargo".into()],
                env: HashMap::from([("NODE_ENV".into(), "test".into())]),
            },
            credentials: vec!["GITHUB_TOKEN".into()],
            stall_threshold_minutes: 20,
            max_review_cycles: 3,
        }
    }

    fn test_params() -> SpawnParams {
        SpawnParams {
            daemon_socket: "/run/caloron/daemon.sock".into(),
            worktree_path: "/project/.caloron/worktrees/backend-developer-sprint-1".into(),
            task_id: "issue-42".into(),
            secrets_file_path: "/run/caloron/secrets/backend-1.env".into(),
        }
    }

    #[test]
    fn test_generate_devshell_uses_native_build_inputs() {
        let nix = NixGenerator::generate_devshell(&test_agent(), &test_params());
        assert!(nix.contains("nativeBuildInputs"), "Should use nativeBuildInputs, not buildInputs");
        assert!(!nix.contains("buildInputs ="), "Should not contain bare buildInputs");
    }

    #[test]
    fn test_generate_devshell_includes_packages() {
        let nix = NixGenerator::generate_devshell(&test_agent(), &test_params());
        assert!(nix.contains("nodejs_20"));
        assert!(nix.contains("rustc"));
        assert!(nix.contains("cargo"));
        assert!(nix.contains("git"));
    }

    #[test]
    fn test_generate_devshell_secrets_via_file() {
        let nix = NixGenerator::generate_devshell(&test_agent(), &test_params());
        assert!(nix.contains("CALORON_SECRETS_FILE"));
        // Should NOT contain actual secret env vars
        assert!(!nix.contains("GITHUB_TOKEN"));
        assert!(!nix.contains("ANTHROPIC_API_KEY"));
    }

    #[test]
    fn test_generate_devshell_custom_env() {
        let nix = NixGenerator::generate_devshell(&test_agent(), &test_params());
        assert!(nix.contains("NODE_ENV=\"test\""));
    }

    #[test]
    fn test_generate_devshell_spawn_params() {
        let nix = NixGenerator::generate_devshell(&test_agent(), &test_params());
        assert!(nix.contains("CALORON_AGENT_ROLE=\"backend-developer\""));
        assert!(nix.contains("CALORON_DAEMON_SOCKET=\"/run/caloron/daemon.sock\""));
        assert!(nix.contains("CALORON_TASK_ID=\"issue-42\""));
    }
}
