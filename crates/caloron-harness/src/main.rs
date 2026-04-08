mod heartbeat;
mod secrets;

use std::process::Stdio;

use clap::{Parser, Subcommand};

#[derive(Parser)]
#[command(name = "caloron-harness", version, about = "Caloron agent harness")]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Start the agent harness
    Start,
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_env("CALORON_LOG_LEVEL")
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info")),
        )
        .init();

    let cli = Cli::parse();

    match cli.command {
        Command::Start => run_harness().await,
    }
}

async fn run_harness() -> anyhow::Result<()> {
    let agent_role = std::env::var("CALORON_AGENT_ROLE").unwrap_or_else(|_| "unknown".into());
    let agent_id = std::env::var("CALORON_AGENT_ID").unwrap_or_else(|_| agent_role.clone());
    let socket_path = std::env::var("CALORON_DAEMON_SOCKET")
        .unwrap_or_else(|_| "/run/caloron/daemon.sock".into());
    let task_id = std::env::var("CALORON_TASK_ID").ok();
    let system_prompt = std::env::var("CALORON_SYSTEM_PROMPT").unwrap_or_default();
    let task_description = std::env::var("CALORON_TASK_DESCRIPTION").unwrap_or_default();
    let worktree = std::env::var("CALORON_WORKTREE").unwrap_or_else(|_| ".".into());

    // Which framework/command to use
    let framework_cmd = std::env::var("CALORON_FRAMEWORK_CMD")
        .unwrap_or_else(|_| "claude".into());
    let framework_args = std::env::var("CALORON_FRAMEWORK_ARGS")
        .unwrap_or_else(|_| "--dangerously-skip-permissions".into());

    tracing::info!(
        agent_role,
        agent_id,
        ?task_id,
        framework = framework_cmd,
        worktree,
        "Harness starting"
    );

    // Step 1: Load and delete secrets file (Addendum R3)
    let secrets = secrets::load_and_delete_secrets();
    for (key, value) in &secrets {
        // SAFETY: single-threaded at this point.
        unsafe { std::env::set_var(key, value) };
    }
    tracing::info!(count = secrets.len(), "Secrets loaded");

    // Step 2: Start heartbeat loop
    let heartbeat_handle = heartbeat::start_heartbeat_loop(
        socket_path.clone(),
        agent_role.clone(),
        task_id.clone(),
    );

    // Step 3: Build the prompt for the LLM
    let prompt = build_prompt(&agent_role, &system_prompt, &task_description, &task_id);

    tracing::info!(
        agent_role,
        prompt_len = prompt.len(),
        "Invoking LLM framework"
    );

    // Step 4: Run the LLM
    let exit_status = run_framework(&framework_cmd, &framework_args, &prompt, &worktree).await;

    match &exit_status {
        Ok(status) => {
            if status.success() {
                tracing::info!(agent_role, "LLM completed successfully");
            } else {
                tracing::warn!(agent_role, code = ?status.code(), "LLM exited with error");
            }
        }
        Err(e) => {
            tracing::error!(agent_role, error = %e, "Failed to run LLM");
        }
    }

    // Step 5: Commit any uncommitted work
    commit_work(&worktree, &agent_id, &task_id).await;

    // Step 6: Shutdown
    heartbeat_handle.abort();
    tracing::info!(agent_role, "Harness complete");

    Ok(())
}

/// Build the prompt that the LLM receives.
fn build_prompt(
    agent_role: &str,
    system_prompt: &str,
    task_description: &str,
    task_id: &Option<String>,
) -> String {
    let mut prompt = String::new();

    if !system_prompt.is_empty() {
        prompt.push_str(system_prompt);
        prompt.push_str("\n\n");
    }

    if !task_description.is_empty() {
        prompt.push_str("## Your Task\n\n");
        prompt.push_str(task_description);
        prompt.push_str("\n\n");
    } else if let Some(tid) = task_id {
        prompt.push_str(&format!("## Your Task\n\nComplete task {tid}.\n\n"));
    }

    prompt.push_str("## Instructions\n\n");
    prompt.push_str("1. Read the existing code to understand the project structure.\n");
    prompt.push_str("2. Implement the required changes.\n");
    prompt.push_str("3. Write tests for your changes.\n");
    prompt.push_str("4. Make sure all tests pass.\n");
    prompt.push_str("5. Commit your work with a clear message.\n");

    prompt
}

/// Run the LLM framework (Claude Code, Gemini CLI, etc.)
async fn run_framework(
    cmd: &str,
    args_str: &str,
    prompt: &str,
    worktree: &str,
) -> Result<std::process::ExitStatus, std::io::Error> {
    let mut command = tokio::process::Command::new(cmd);

    // Parse args string into individual args
    let args: Vec<&str> = args_str.split_whitespace().collect();
    command.args(&args);

    // Add the prompt via -p flag
    command.arg("-p").arg(prompt);

    // Set working directory to the worktree
    command.current_dir(worktree);

    // Inherit stdio so we can see what the agent is doing
    command.stdout(Stdio::inherit());
    command.stderr(Stdio::inherit());
    command.stdin(Stdio::null());

    let mut child = command.spawn()?;
    child.wait().await
}

/// Commit any uncommitted work in the worktree.
async fn commit_work(worktree: &str, agent_id: &str, task_id: &Option<String>) {
    let msg = task_id
        .as_ref()
        .map(|tid| format!("[{agent_id}] Complete {tid}"))
        .unwrap_or_else(|| format!("[{agent_id}] Work completed"));

    // Stage all changes
    let _ = tokio::process::Command::new("git")
        .args(["add", "-A"])
        .current_dir(worktree)
        .output()
        .await;

    // Check if there's anything to commit
    let status = tokio::process::Command::new("git")
        .args(["diff", "--cached", "--quiet"])
        .current_dir(worktree)
        .status()
        .await;

    if status.is_ok_and(|s| !s.success()) {
        // There are staged changes — commit
        let result = tokio::process::Command::new("git")
            .args(["commit", "-m", &msg])
            .current_dir(worktree)
            .output()
            .await;

        match result {
            Ok(output) if output.status.success() => {
                tracing::info!(agent_id, "Committed work");
            }
            Ok(output) => {
                let stderr = String::from_utf8_lossy(&output.stderr);
                tracing::warn!(agent_id, stderr = %stderr, "Commit failed");
            }
            Err(e) => {
                tracing::warn!(agent_id, error = %e, "Could not run git commit");
            }
        }
    } else {
        tracing::debug!(agent_id, "No changes to commit");
    }
}
