use std::time::Duration;

use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::net::UnixStream;
use tokio::task::JoinHandle;

use caloron_types::agent::HarnessMessage;

/// Start a background heartbeat loop that sends periodic heartbeats to the daemon.
/// Returns a handle that can be used to abort the loop.
pub fn start_heartbeat_loop(
    socket_path: String,
    agent_role: String,
    task_id: Option<String>,
) -> JoinHandle<()> {
    tokio::spawn(async move {
        let mut tokens_total: u64 = 0;
        let mut consecutive_failures = 0u32;

        loop {
            tokio::time::sleep(Duration::from_secs(60)).await;

            match send_heartbeat(&socket_path, &agent_role, &task_id, tokens_total).await {
                Ok(_) => {
                    if consecutive_failures > 0 {
                        tracing::info!("Heartbeat connection restored");
                        consecutive_failures = 0;
                    }
                    tracing::trace!(agent_role, "Heartbeat sent");
                }
                Err(e) => {
                    consecutive_failures += 1;
                    tracing::warn!(
                        error = %e,
                        consecutive_failures,
                        "Failed to send heartbeat"
                    );
                    // Don't crash the agent if daemon is temporarily unreachable
                    if consecutive_failures >= 10 {
                        tracing::error!(
                            "Heartbeat failed 10 consecutive times — daemon may be down"
                        );
                    }
                }
            }
        }
    })
}

/// Send a single heartbeat message to the daemon via Unix socket.
async fn send_heartbeat(
    socket_path: &str,
    agent_role: &str,
    task_id: &Option<String>,
    tokens_used: u64,
) -> anyhow::Result<()> {
    let stream = UnixStream::connect(socket_path).await?;
    let (reader, mut writer) = stream.into_split();
    let mut reader = BufReader::new(reader);

    let msg = HarnessMessage::Heartbeat {
        agent_role: agent_role.to_string(),
        task_id: task_id.clone(),
        tokens_used,
    };

    let json = serde_json::to_string(&msg)?;
    writer.write_all(json.as_bytes()).await?;
    writer.write_all(b"\n").await?;

    // Read response
    let mut response = String::new();
    reader.read_line(&mut response).await?;

    Ok(())
}

/// Send a status update to the daemon.
pub async fn send_status(
    socket_path: &str,
    agent_role: &str,
    status: &str,
    detail: &str,
) -> anyhow::Result<()> {
    let stream = UnixStream::connect(socket_path).await?;
    let (reader, mut writer) = stream.into_split();
    let mut reader = BufReader::new(reader);

    let msg = HarnessMessage::Status {
        agent_role: agent_role.to_string(),
        status: status.to_string(),
        detail: detail.to_string(),
    };

    let json = serde_json::to_string(&msg)?;
    writer.write_all(json.as_bytes()).await?;
    writer.write_all(b"\n").await?;

    let mut response = String::new();
    reader.read_line(&mut response).await?;

    Ok(())
}

/// Send a completion signal to the daemon.
pub async fn send_completed(
    socket_path: &str,
    agent_role: &str,
    task_id: &str,
) -> anyhow::Result<()> {
    let stream = UnixStream::connect(socket_path).await?;
    let (reader, mut writer) = stream.into_split();
    let mut reader = BufReader::new(reader);

    let msg = HarnessMessage::Completed {
        agent_role: agent_role.to_string(),
        task_id: task_id.to_string(),
    };

    let json = serde_json::to_string(&msg)?;
    writer.write_all(json.as_bytes()).await?;
    writer.write_all(b"\n").await?;

    let mut response = String::new();
    reader.read_line(&mut response).await?;

    Ok(())
}

/// Send an error report to the daemon.
pub async fn send_error(
    socket_path: &str,
    agent_role: &str,
    error_type: &str,
    detail: &str,
    count: u32,
) -> anyhow::Result<()> {
    let stream = UnixStream::connect(socket_path).await?;
    let (reader, mut writer) = stream.into_split();
    let mut reader = BufReader::new(reader);

    let msg = HarnessMessage::Error {
        agent_role: agent_role.to_string(),
        error_type: error_type.to_string(),
        detail: detail.to_string(),
        count,
    };

    let json = serde_json::to_string(&msg)?;
    writer.write_all(json.as_bytes()).await?;
    writer.write_all(b"\n").await?;

    let mut response = String::new();
    reader.read_line(&mut response).await?;

    Ok(())
}
