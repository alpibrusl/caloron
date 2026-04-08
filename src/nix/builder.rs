use std::path::{Path, PathBuf};
use std::process::Stdio;

use anyhow::{Context, Result, bail};
use tokio::process::Command;

use caloron_types::agent::AgentDefinition;

use super::generator::{NixGenerator, SpawnParams};

/// Manages Nix environment builds and process execution.
///
/// Responsible for:
/// 1. Writing generated flake.nix to the agent's working directory
/// 2. Building the Nix environment (caching via the Nix store)
/// 3. Running commands inside the Nix environment with `nix develop`
pub struct NixBuilder {
    /// Directory where agent flakes are generated (.caloron/nix/)
    flake_dir: PathBuf,
    /// Whether Nix is enabled (falls back to direct execution if false)
    enabled: bool,
}

/// Result of building a Nix environment.
#[derive(Debug)]
pub struct NixEnv {
    /// Path to the flake directory for this agent
    pub flake_path: PathBuf,
    /// The devShell attribute name (e.g., "agent-backend-developer")
    pub shell_attr: String,
    /// Whether the build used Nix or fell back to direct mode
    pub nix_used: bool,
}

impl NixBuilder {
    pub fn new(caloron_dir: &Path, enabled: bool) -> Self {
        Self {
            flake_dir: caloron_dir.join("nix"),
            enabled,
        }
    }

    /// Check if Nix is available on the system.
    pub async fn is_nix_available() -> bool {
        Command::new("nix")
            .args(["--version"])
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
            .await
            .is_ok_and(|s| s.success())
    }

    /// Write a flake.nix for an agent and build the environment.
    ///
    /// Steps:
    /// 1. Generate the flake.nix from the agent definition
    /// 2. Write it to .caloron/nix/{agent_name}/flake.nix
    /// 3. Run `nix develop --build` to pre-build (caches in Nix store)
    pub async fn build_env(
        &self,
        agent_def: &AgentDefinition,
        params: &SpawnParams,
    ) -> Result<NixEnv> {
        let shell_attr = format!("agent-{}", agent_def.name);

        if !self.enabled {
            tracing::debug!(agent = agent_def.name, "Nix disabled — using direct execution");
            return Ok(NixEnv {
                flake_path: PathBuf::new(),
                shell_attr,
                nix_used: false,
            });
        }

        let agent_flake_dir = self.flake_dir.join(&agent_def.name);
        std::fs::create_dir_all(&agent_flake_dir)
            .context("Failed to create agent flake directory")?;

        // Step 1: Generate flake.nix
        let flake_content = generate_standalone_flake(agent_def, params);
        let flake_path = agent_flake_dir.join("flake.nix");
        std::fs::write(&flake_path, &flake_content)
            .with_context(|| format!("Failed to write {}", flake_path.display()))?;

        tracing::info!(
            agent = agent_def.name,
            path = %flake_path.display(),
            "Generated flake.nix"
        );

        // Step 2: Build the environment (pre-cache)
        tracing::info!(agent = agent_def.name, "Building Nix environment...");

        let output = Command::new("nix")
            .args([
                "develop",
                &format!(".#{shell_attr}"),
                "--command",
                "echo",
                "ready",
            ])
            .current_dir(&agent_flake_dir)
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .output()
            .await
            .context("Failed to run nix develop")?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            bail!(
                "Nix environment build failed for agent {}: {stderr}",
                agent_def.name
            );
        }

        tracing::info!(agent = agent_def.name, "Nix environment built successfully");

        Ok(NixEnv {
            flake_path: agent_flake_dir,
            shell_attr,
            nix_used: true,
        })
    }

    /// Start a command inside the Nix environment.
    ///
    /// Runs: `nix develop .#agent-{name} --command <cmd> <args...>`
    /// Uses `--impure` to allow access to the secrets file and daemon socket.
    pub async fn run_in_env(
        &self,
        env: &NixEnv,
        cmd: &str,
        args: &[&str],
        working_dir: &Path,
        extra_env: &[(&str, &str)],
    ) -> Result<tokio::process::Child> {
        if !env.nix_used {
            // Fallback: run directly without Nix
            return self
                .run_direct(cmd, args, working_dir, extra_env)
                .await;
        }

        let mut command = Command::new("nix");
        command
            .arg("develop")
            .arg(&format!(".#{}", env.shell_attr))
            // --impure allows the shellHook env vars and access to host paths
            // (daemon socket, secrets file, worktree). This is intentional:
            // the Nix env provides tool isolation, not filesystem sandboxing.
            .arg("--impure")
            .arg("--command")
            .arg(cmd)
            .args(args)
            .current_dir(&env.flake_path)
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());

        // Set CALORON_WORKTREE so the harness knows where to work,
        // since nix develop runs from the flake dir, not the worktree.
        for (key, value) in extra_env {
            command.env(key, value);
        }
        // Override the CWD for the inner command via env var
        // (the actual cwd is the flake dir for nix to find flake.nix)
        command.env("CALORON_WORKING_DIR", working_dir);

        let child = command.spawn().with_context(|| {
            format!(
                "Failed to spawn `nix develop .#{} --command {cmd}`",
                env.shell_attr
            )
        })?;

        Ok(child)
    }

    /// Fallback: run a command directly without Nix isolation.
    async fn run_direct(
        &self,
        cmd: &str,
        args: &[&str],
        working_dir: &Path,
        extra_env: &[(&str, &str)],
    ) -> Result<tokio::process::Child> {
        let mut command = Command::new(cmd);
        command
            .args(args)
            .current_dir(working_dir)
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());

        for (key, value) in extra_env {
            command.env(key, value);
        }

        let child = command
            .spawn()
            .with_context(|| format!("Failed to spawn {cmd}"))?;

        Ok(child)
    }

    /// Clean up the flake directory for an agent.
    pub fn cleanup(&self, agent_name: &str) -> Result<()> {
        let agent_flake_dir = self.flake_dir.join(agent_name);
        if agent_flake_dir.exists() {
            std::fs::remove_dir_all(&agent_flake_dir)
                .with_context(|| format!("Failed to clean up {}", agent_flake_dir.display()))?;
        }
        Ok(())
    }
}

/// Generate a standalone flake.nix for a single agent.
/// This is a complete, self-contained flake that can be built with `nix develop`.
fn generate_standalone_flake(agent: &AgentDefinition, params: &SpawnParams) -> String {
    let mut out = String::new();

    out.push_str("{\n");
    out.push_str(&format!(
        "  description = \"Caloron agent environment: {}\";\n\n",
        agent.name
    ));

    // Inputs
    out.push_str("  inputs = {\n");
    out.push_str("    nixpkgs.url = \"github:NixOS/nixpkgs/nixpkgs-unstable\";\n");
    out.push_str("  };\n\n");

    // Outputs
    out.push_str("  outputs = { self, nixpkgs }:\n");
    out.push_str("    let\n");
    out.push_str("      supportedSystems = [ \"x86_64-linux\" \"aarch64-linux\" \"x86_64-darwin\" \"aarch64-darwin\" ];\n");
    out.push_str("      forAllSystems = nixpkgs.lib.genAttrs supportedSystems;\n");
    out.push_str("    in {\n");
    out.push_str("      devShells = forAllSystems (system:\n");
    out.push_str("        let\n");
    out.push_str("          pkgs = import nixpkgs { inherit system; };\n");
    out.push_str("        in {\n");

    // The devShell
    out.push_str(&format!(
        "          \"agent-{}\" = pkgs.mkShell {{\n",
        agent.name
    ));
    out.push_str(&format!(
        "            name = \"caloron-agent-{}\";\n\n",
        agent.name
    ));

    // nativeBuildInputs
    out.push_str("            nativeBuildInputs = with pkgs; [\n");
    out.push_str("              git\n");
    for pkg in &agent.nix.packages {
        out.push_str(&format!("              {pkg}\n"));
    }
    out.push_str("            ];\n\n");

    // shellHook — non-secret env vars only (secrets via file per R3)
    out.push_str("            shellHook = ''\n");
    out.push_str(&format!(
        "              export CALORON_AGENT_ROLE=\"{}\"\n",
        agent.name
    ));
    out.push_str(&format!(
        "              export CALORON_DAEMON_SOCKET=\"{}\"\n",
        params.daemon_socket
    ));
    out.push_str(&format!(
        "              export CALORON_WORKTREE=\"{}\"\n",
        params.worktree_path
    ));
    out.push_str(&format!(
        "              export CALORON_TASK_ID=\"{}\"\n",
        params.task_id
    ));
    out.push_str(&format!(
        "              export CALORON_SECRETS_FILE=\"{}\"\n",
        params.secrets_file_path
    ));

    for (key, value) in &agent.nix.env {
        out.push_str(&format!("              export {key}=\"{value}\"\n"));
    }

    out.push_str("            '';\n");
    out.push_str("          };\n");
    out.push_str("        });\n");
    out.push_str("    };\n");
    out.push_str("}\n");

    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use caloron_types::agent::{AgentDefinition, LlmConfig, NixConfig};
    use std::collections::HashMap;

    fn test_agent() -> AgentDefinition {
        AgentDefinition {
            name: "test-agent".into(),
            version: "1.0".into(),
            description: "Test".into(),
            llm: LlmConfig {
                model: "test".into(),
                max_tokens: 8192,
                temperature: 0.2,
            },
            system_prompt: "Test prompt".into(),
            tools: vec!["bash".into()],
            mcps: vec![],
            nix: NixConfig {
                packages: vec!["python311".into(), "nodejs_20".into()],
                env: HashMap::from([("MY_VAR".into(), "hello".into())]),
            },
            credentials: vec!["GITHUB_TOKEN".into()],
            stall_threshold_minutes: 20,
            max_review_cycles: 3,
        }
    }

    fn test_params() -> SpawnParams {
        SpawnParams {
            daemon_socket: "/tmp/test.sock".into(),
            worktree_path: "/tmp/worktree".into(),
            task_id: "issue-1".into(),
            secrets_file_path: "/tmp/secrets.env".into(),
        }
    }

    #[test]
    fn test_standalone_flake_structure() {
        let flake = generate_standalone_flake(&test_agent(), &test_params());

        // Should be a valid flake structure
        assert!(flake.contains("description ="));
        assert!(flake.contains("inputs ="));
        assert!(flake.contains("nixpkgs.url ="));
        assert!(flake.contains("outputs ="));
        assert!(flake.contains("devShells ="));
        assert!(flake.contains("\"agent-test-agent\""));
    }

    #[test]
    fn test_standalone_flake_packages() {
        let flake = generate_standalone_flake(&test_agent(), &test_params());

        assert!(flake.contains("nativeBuildInputs"));
        assert!(flake.contains("python311"));
        assert!(flake.contains("nodejs_20"));
        assert!(flake.contains("git"));
    }

    #[test]
    fn test_standalone_flake_env_vars() {
        let flake = generate_standalone_flake(&test_agent(), &test_params());

        assert!(flake.contains("CALORON_AGENT_ROLE=\"test-agent\""));
        assert!(flake.contains("CALORON_DAEMON_SOCKET=\"/tmp/test.sock\""));
        assert!(flake.contains("CALORON_WORKTREE=\"/tmp/worktree\""));
        assert!(flake.contains("CALORON_TASK_ID=\"issue-1\""));
        assert!(flake.contains("CALORON_SECRETS_FILE=\"/tmp/secrets.env\""));
        assert!(flake.contains("MY_VAR=\"hello\""));
    }

    #[test]
    fn test_standalone_flake_no_secrets_in_env() {
        let flake = generate_standalone_flake(&test_agent(), &test_params());

        // Secrets should NOT be in the flake — only the path to the secrets file
        assert!(!flake.contains("GITHUB_TOKEN="));
        assert!(!flake.contains("ANTHROPIC_API_KEY="));
    }

    #[test]
    fn test_build_env_disabled_returns_fallback() {
        let dir = tempfile::tempdir().unwrap();
        let builder = NixBuilder::new(dir.path(), false);

        let rt = tokio::runtime::Runtime::new().unwrap();
        let env = rt
            .block_on(builder.build_env(&test_agent(), &test_params()))
            .unwrap();

        assert!(!env.nix_used);
        assert_eq!(env.shell_attr, "agent-test-agent");
    }

    #[test]
    fn test_write_flake_to_disk() {
        let dir = tempfile::tempdir().unwrap();
        let builder = NixBuilder::new(dir.path(), true);

        // We can't actually run `nix develop` in tests without Nix,
        // but we can verify the file is written correctly.
        let agent_dir = dir.path().join("test-agent");
        std::fs::create_dir_all(&agent_dir).unwrap();

        let flake = generate_standalone_flake(&test_agent(), &test_params());
        let flake_path = agent_dir.join("flake.nix");
        std::fs::write(&flake_path, &flake).unwrap();

        assert!(flake_path.exists());
        let content = std::fs::read_to_string(&flake_path).unwrap();
        assert!(content.contains("agent-test-agent"));
    }

    #[test]
    fn test_cleanup() {
        let dir = tempfile::tempdir().unwrap();
        let builder = NixBuilder::new(dir.path(), true);

        // Create a fake agent flake dir at the path the builder expects
        let agent_dir = dir.path().join("nix").join("test-agent");
        std::fs::create_dir_all(&agent_dir).unwrap();
        std::fs::write(agent_dir.join("flake.nix"), "fake").unwrap();

        assert!(agent_dir.exists());
        builder.cleanup("test-agent").unwrap();
        assert!(!agent_dir.exists());
    }

    #[tokio::test]
    async fn test_nix_available_check() {
        // This test just verifies the function doesn't panic.
        // The result depends on whether nix is installed.
        let _available = NixBuilder::is_nix_available().await;
    }

    #[tokio::test]
    async fn test_run_direct_fallback() {
        let dir = tempfile::tempdir().unwrap();
        let builder = NixBuilder::new(dir.path(), false);

        let env = NixEnv {
            flake_path: PathBuf::new(),
            shell_attr: "test".into(),
            nix_used: false,
        };

        let mut child = builder
            .run_in_env(&env, "echo", &["hello"], dir.path(), &[])
            .await
            .unwrap();

        let status = child.wait().await.unwrap();
        assert!(status.success());
    }
}
