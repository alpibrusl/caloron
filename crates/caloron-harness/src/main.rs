mod heartbeat;
mod secrets;

use std::collections::HashMap;

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
    // Read config from environment
    let agent_role = std::env::var("CALORON_AGENT_ROLE")
        .unwrap_or_else(|_| "unknown".into());
    let agent_id = std::env::var("CALORON_AGENT_ID")
        .unwrap_or_else(|_| agent_role.clone());
    let socket_path = std::env::var("CALORON_DAEMON_SOCKET")
        .unwrap_or_else(|_| "/run/caloron/daemon.sock".into());
    let task_id = std::env::var("CALORON_TASK_ID").ok();

    tracing::info!(
        agent_role,
        agent_id,
        ?task_id,
        "Harness starting"
    );

    // Step 1: Load and delete secrets file (Addendum R3)
    let secrets = secrets::load_and_delete_secrets();
    for (key, value) in &secrets {
        // Set as env vars for tools that need them.
        // SAFETY: The harness is single-threaded at this point (before spawning async tasks).
        unsafe { std::env::set_var(key, value) };
    }
    tracing::info!(count = secrets.len(), "Secrets loaded");

    // Step 2: Start heartbeat loop
    let heartbeat_handle = heartbeat::start_heartbeat_loop(
        socket_path.clone(),
        agent_role.clone(),
        task_id.clone(),
    );

    // Step 3: Run the LLM harness
    // In production, this invokes claude-code or another ACLI-compatible tool.
    // For now, we log that we're ready and wait for the process to be managed externally.
    tracing::info!(agent_role, "Harness ready — waiting for LLM process");

    // The actual LLM invocation will be implemented when we integrate with claude-code.
    // The harness wraps the LLM process and ensures:
    // - Heartbeats continue while the LLM is running
    // - Errors are captured and reported
    // - Feedback comment is enforced on exit
    //
    // For Phase 1, the harness runs until killed by the spawner.
    // This is sufficient for the D1 demo where we verify the lifecycle.
    tokio::signal::ctrl_c().await?;

    // Step 4: Shutdown
    heartbeat_handle.abort();
    tracing::info!(agent_role, "Harness shutting down");

    // Step 5: Verify feedback comment exists (enforcement)
    // TODO: Check GitHub for feedback comment on the task issue
    // If missing, prompt LLM to generate one before exiting

    Ok(())
}
