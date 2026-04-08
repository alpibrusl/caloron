use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::time::Duration;

use anyhow::{Context, Result, bail};
use tokio::process::{Child, Command};

use caloron_types::agent::{AgentDefinition, AgentHealth, AgentStatus};
use caloron_types::config::CaloronConfig;

use super::worktree::WorktreeManager;
use crate::nix::builder::{NixBuilder, NixEnv};
use crate::nix::generator::SpawnParams;

/// Manages the lifecycle of agent processes.
pub struct AgentSpawner {
    config: CaloronConfig,
    worktree_manager: WorktreeManager,
    nix_builder: NixBuilder,
    secrets_dir: PathBuf,
    socket_path: PathBuf,
    /// Running agent processes indexed by agent_id.
    processes: HashMap<String, AgentProcess>,
}

/// A running agent process.
pub struct AgentProcess {
    pub agent_id: String,
    pub role: String,
    pub task_id: String,
    pub sprint_id: String,
    pub worktree_path: PathBuf,
    pub nix_env: NixEnv,
    pub child: Child,
    pub pid: u32,
}

impl AgentSpawner {
    pub fn new(
        config: CaloronConfig,
        repo_root: PathBuf,
        socket_path: PathBuf,
    ) -> Self {
        let caloron_dir = repo_root.join(".caloron");
        let nix_enabled = config.nix.enabled;
        let secrets_dir = PathBuf::from("/run/caloron/secrets");

        Self {
            config,
            worktree_manager: WorktreeManager::new(repo_root),
            nix_builder: NixBuilder::new(&caloron_dir, nix_enabled),
            secrets_dir,
            socket_path,
            processes: HashMap::new(),
        }
    }

    /// Full agent spawn sequence (Section 8.4 of docs).
    ///
    /// 1. Create git worktree
    /// 2. Inject secrets via temporary file (R3)
    /// 3. Generate Nix flake and build environment
    /// 4. Start harness inside the Nix env (or directly if Nix disabled)
    /// 5. Register health contract
    pub async fn spawn(
        &mut self,
        agent_id: &str,
        agent_def: &AgentDefinition,
        task_id: &str,
        sprint_id: &str,
        credentials: &HashMap<String, String>,
    ) -> Result<AgentHealth> {
        tracing::info!(
            agent_id,
            role = agent_def.name,
            task_id,
            "Spawning agent"
        );

        // Step 1: Create git worktree
        let worktree_path = self
            .worktree_manager
            .create(agent_id, sprint_id)
            .context("Failed to create worktree")?;

        // Step 2: Inject secrets via temporary file (Addendum R3)
        let secrets_file = self
            .inject_secrets(agent_id, credentials)
            .context("Failed to inject secrets")?;

        // Step 3: Generate Nix spawn params
        let params = SpawnParams {
            daemon_socket: self.socket_path.to_string_lossy().to_string(),
            worktree_path: worktree_path.to_string_lossy().to_string(),
            task_id: task_id.to_string(),
            secrets_file_path: secrets_file.to_string_lossy().to_string(),
        };

        // Step 4: Build Nix environment (writes flake.nix, runs nix develop to cache)
        let nix_env = self
            .nix_builder
            .build_env(agent_def, &params)
            .await
            .context("Failed to build Nix environment")?;

        if nix_env.nix_used {
            tracing::info!(
                agent_id,
                flake = %nix_env.flake_path.display(),
                "Nix environment ready"
            );
        } else {
            tracing::info!(agent_id, "Running without Nix isolation");
        }

        // Step 5: Start harness process inside the Nix env
        let child = self
            .start_harness(agent_id, agent_def, &worktree_path, &params, &nix_env)
            .await
            .context("Failed to start harness")?;

        let pid = child.id().unwrap_or(0);

        tracing::info!(agent_id, pid, nix = nix_env.nix_used, "Agent process started");

        // Step 6: Register process
        self.processes.insert(
            agent_id.to_string(),
            AgentProcess {
                agent_id: agent_id.to_string(),
                role: agent_def.name.clone(),
                task_id: task_id.to_string(),
                sprint_id: sprint_id.to_string(),
                worktree_path: worktree_path.clone(),
                nix_env,
                child,
                pid,
            },
        );

        // Step 7: Create health contract
        let stall_threshold =
            Duration::from_secs(agent_def.stall_threshold_minutes as u64 * 60);
        let mut health =
            AgentHealth::new(agent_id.to_string(), agent_def.name.clone(), stall_threshold);
        health.current_task_id = Some(task_id.to_string());
        health.status = AgentStatus::Working;

        Ok(health)
    }

    /// Full agent destruction sequence (Section 8.5 of docs).
    pub async fn destroy(&mut self, agent_id: &str, sprint_id: &str) -> Result<()> {
        tracing::info!(agent_id, "Destroying agent");

        // Step 1: Stop the harness process
        if let Some(mut process) = self.processes.remove(agent_id) {
            if let Err(e) = process.child.kill().await {
                tracing::warn!(agent_id, error = %e, "Failed to kill agent process");
            }

            // Clean up Nix flake directory (env stays cached in Nix store)
            if process.nix_env.nix_used {
                let _ = self.nix_builder.cleanup(&process.role);
            }
        }

        // Step 2: Remove worktree
        self.worktree_manager
            .remove(agent_id, sprint_id)
            .context("Failed to remove worktree")?;

        // Step 3: Clean up secrets file
        let secrets_file = self.secrets_dir.join(format!("{agent_id}.env"));
        if secrets_file.exists() {
            let _ = std::fs::remove_file(&secrets_file);
        }

        Ok(())
    }

    /// Destroy agent but preserve worktree (for crash debugging per Addendum H3).
    pub async fn destroy_preserve_worktree(&mut self, agent_id: &str, sprint_id: &str) -> Result<()> {
        tracing::info!(agent_id, "Destroying agent (preserving worktree for debugging)");

        if let Some(mut process) = self.processes.remove(agent_id) {
            if let Err(e) = process.child.kill().await {
                tracing::warn!(agent_id, error = %e, "Failed to kill agent process");
            }
            // Don't clean up Nix flake dir — useful for debugging
        }

        self.worktree_manager
            .mark_cancelled(agent_id, sprint_id)
            .context("Failed to mark worktree as cancelled")?;

        let secrets_file = self.secrets_dir.join(format!("{agent_id}.env"));
        if secrets_file.exists() {
            let _ = std::fs::remove_file(&secrets_file);
        }

        Ok(())
    }

    /// Restart an agent with the same task context.
    pub async fn restart(
        &mut self,
        agent_id: &str,
        agent_def: &AgentDefinition,
        task_id: &str,
        sprint_id: &str,
        credentials: &HashMap<String, String>,
    ) -> Result<AgentHealth> {
        tracing::info!(agent_id, task_id, "Restarting agent");

        if let Some(mut process) = self.processes.remove(agent_id) {
            let _ = process.child.kill().await;
        }

        // Re-spawn (reuses existing worktree and Nix store cache)
        self.spawn(agent_id, agent_def, task_id, sprint_id, credentials)
            .await
    }

    /// Check if an agent process is still running.
    pub async fn is_running(&mut self, agent_id: &str) -> bool {
        if let Some(process) = self.processes.get_mut(agent_id) {
            match process.child.try_wait() {
                Ok(None) => true,
                Ok(Some(_)) => false,
                Err(_) => false,
            }
        } else {
            false
        }
    }

    /// Wait for an agent process to exit and return exit status.
    pub async fn wait(&mut self, agent_id: &str) -> Result<Option<std::process::ExitStatus>> {
        if let Some(process) = self.processes.get_mut(agent_id) {
            let status = process.child.wait().await?;
            Ok(Some(status))
        } else {
            Ok(None)
        }
    }

    /// Inject secrets into a temporary file (Addendum R3).
    fn inject_secrets(
        &self,
        agent_id: &str,
        credentials: &HashMap<String, String>,
    ) -> Result<PathBuf> {
        std::fs::create_dir_all(&self.secrets_dir)
            .context("Failed to create secrets directory")?;

        let secrets_file = self.secrets_dir.join(format!("{agent_id}.env"));

        let mut content = String::new();
        for (key, value) in credentials {
            content.push_str(&format!("{key}={value}\n"));
        }

        std::fs::write(&secrets_file, &content)
            .context("Failed to write secrets file")?;

        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&secrets_file, std::fs::Permissions::from_mode(0o600))
                .context("Failed to set secrets file permissions")?;
        }

        Ok(secrets_file)
    }

    /// Start the harness process, either inside a Nix environment or directly.
    async fn start_harness(
        &self,
        agent_id: &str,
        agent_def: &AgentDefinition,
        worktree_path: &Path,
        params: &SpawnParams,
        nix_env: &NixEnv,
    ) -> Result<Child> {
        let extra_env: Vec<(&str, &str)> = vec![
            ("CALORON_AGENT_ID", agent_id),
            ("CALORON_SYSTEM_PROMPT", &agent_def.system_prompt),
            ("CALORON_LLM_MODEL", &agent_def.llm.model),
        ];

        if nix_env.nix_used {
            // Run inside Nix: `nix develop .#agent-{name} --impure --command caloron-harness start`
            // The shellHook in the flake sets CALORON_AGENT_ROLE, DAEMON_SOCKET, etc.
            self.nix_builder
                .run_in_env(nix_env, "caloron-harness", &["start"], worktree_path, &extra_env)
                .await
        } else {
            // Fallback: run directly with env vars
            let mut cmd = Command::new("caloron-harness");
            cmd.arg("start")
                .env("CALORON_AGENT_ROLE", &agent_def.name)
                .env("CALORON_AGENT_ID", agent_id)
                .env("CALORON_DAEMON_SOCKET", &params.daemon_socket)
                .env("CALORON_WORKTREE", &params.worktree_path)
                .env("CALORON_TASK_ID", &params.task_id)
                .env("CALORON_SECRETS_FILE", &params.secrets_file_path)
                .env("CALORON_SYSTEM_PROMPT", &agent_def.system_prompt)
                .env("CALORON_LLM_MODEL", &agent_def.llm.model)
                .env("CALORON_LLM_MAX_TOKENS", agent_def.llm.max_tokens.to_string())
                .env("CALORON_LLM_TEMPERATURE", agent_def.llm.temperature.to_string())
                .current_dir(worktree_path)
                .stdin(Stdio::null())
                .stdout(Stdio::piped())
                .stderr(Stdio::piped());

            // Custom env from agent definition
            for (key, value) in &agent_def.nix.env {
                cmd.env(key, value);
            }

            cmd.spawn().context("Failed to spawn harness process")
        }
    }

    /// Get a reference to the worktree manager.
    pub fn worktree_manager(&self) -> &WorktreeManager {
        &self.worktree_manager
    }

    /// Get all running agent IDs.
    pub fn running_agents(&self) -> Vec<String> {
        self.processes.keys().cloned().collect()
    }
}
