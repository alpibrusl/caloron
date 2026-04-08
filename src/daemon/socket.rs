use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::net::{UnixListener, UnixStream};

use caloron_types::agent::HarnessMessage;

use super::state::DaemonState;

/// Unix socket server for harness-to-daemon communication.
/// Each harness connects and sends JSON-encoded HarnessMessages, one per line.
pub struct DaemonSocket {
    socket_path: PathBuf,
    state: DaemonState,
}

impl DaemonSocket {
    pub fn new(socket_path: PathBuf, state: DaemonState) -> Self {
        Self { socket_path, state }
    }

    /// Start listening for harness connections.
    /// Each connection is handled in a separate task.
    pub async fn listen(&self) -> Result<()> {
        // Clean up stale socket file
        if self.socket_path.exists() {
            std::fs::remove_file(&self.socket_path)
                .context("Failed to remove stale socket file")?;
        }

        // Ensure parent directory exists
        if let Some(parent) = self.socket_path.parent() {
            std::fs::create_dir_all(parent)
                .context("Failed to create socket directory")?;
        }

        let listener = UnixListener::bind(&self.socket_path)
            .context("Failed to bind Unix socket")?;

        tracing::info!(
            path = %self.socket_path.display(),
            "Daemon socket listening"
        );

        loop {
            match listener.accept().await {
                Ok((stream, _addr)) => {
                    let state = self.state.clone();
                    tokio::spawn(async move {
                        if let Err(e) = handle_connection(stream, state).await {
                            tracing::error!(error = %e, "Connection handler error");
                        }
                    });
                }
                Err(e) => {
                    tracing::error!(error = %e, "Failed to accept connection");
                }
            }
        }
    }

    /// Get the socket path (for passing to agents).
    pub fn path(&self) -> &Path {
        &self.socket_path
    }
}

/// Handle a single harness connection.
/// Reads newline-delimited JSON messages and processes each one.
async fn handle_connection(stream: UnixStream, state: DaemonState) -> Result<()> {
    let (reader, mut writer) = stream.into_split();
    let mut reader = BufReader::new(reader);
    let mut line = String::new();

    loop {
        line.clear();
        let bytes_read = reader.read_line(&mut line).await?;
        if bytes_read == 0 {
            // Connection closed
            break;
        }

        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }

        match serde_json::from_str::<HarnessMessage>(trimmed) {
            Ok(msg) => {
                let response = handle_message(msg, &state).await;
                let response_json = serde_json::to_string(&response).unwrap();
                writer
                    .write_all(response_json.as_bytes())
                    .await
                    .context("Failed to write response")?;
                writer.write_all(b"\n").await?;
            }
            Err(e) => {
                tracing::warn!(error = %e, line = trimmed, "Failed to parse harness message");
                writer
                    .write_all(b"{\"status\":\"error\",\"message\":\"invalid JSON\"}\n")
                    .await?;
            }
        }
    }

    Ok(())
}

/// Process a single harness message and update daemon state.
async fn handle_message(
    msg: HarnessMessage,
    state: &DaemonState,
) -> serde_json::Value {
    match msg {
        HarnessMessage::Heartbeat {
            agent_role,
            task_id,
            tokens_used,
        } => {
            tracing::trace!(
                agent_role,
                ?task_id,
                tokens_used,
                "Heartbeat received"
            );

            state
                .update_agent(&agent_role, |health| {
                    health.record_heartbeat();
                })
                .await;

            serde_json::json!({ "status": "ok" })
        }

        HarnessMessage::Status {
            agent_role,
            status,
            detail,
        } => {
            tracing::info!(agent_role, status, detail, "Agent status update");

            state
                .update_agent(&agent_role, |health| {
                    health.record_git_event();
                    health.clear_errors();
                })
                .await;

            serde_json::json!({ "status": "ok" })
        }

        HarnessMessage::Error {
            agent_role,
            error_type,
            detail,
            count,
        } => {
            tracing::warn!(
                agent_role,
                error_type,
                detail,
                count,
                "Agent error reported"
            );

            let error = parse_error_type(&error_type, &detail);
            state
                .update_agent(&agent_role, |health| {
                    health.record_error(error);
                })
                .await;

            serde_json::json!({ "status": "ok" })
        }

        HarnessMessage::Completed {
            agent_role,
            task_id,
        } => {
            tracing::info!(agent_role, task_id, "Agent completed task");

            state
                .update_agent(&agent_role, |health| {
                    health.status = caloron_types::agent::AgentStatus::Completing;
                    health.current_task_id = None;
                })
                .await;

            serde_json::json!({ "status": "ok" })
        }
    }
}

fn parse_error_type(error_type: &str, detail: &str) -> caloron_types::agent::ErrorType {
    match error_type {
        "credentials" => caloron_types::agent::ErrorType::CredentialsFailure {
            tool: detail.to_string(),
        },
        "rate_limited" => caloron_types::agent::ErrorType::RateLimited {
            tool: detail.to_string(),
        },
        "tool_unavailable" => caloron_types::agent::ErrorType::ToolUnavailable {
            tool: detail.to_string(),
        },
        _ => caloron_types::agent::ErrorType::Unknown,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use caloron_types::agent::AgentHealth;
    use std::time::Duration;

    fn test_state() -> DaemonState {
        let config: caloron_types::config::CaloronConfig = toml::from_str(
            "[project]\nname=\"t\"\nrepo=\"o/r\"\nmeta_repo=\"o/m\"\n[github]\n[llm]\n",
        )
        .unwrap();
        DaemonState::new(config)
    }

    #[tokio::test]
    async fn test_handle_heartbeat() {
        let state = test_state();
        state
            .register_agent(AgentHealth::new(
                "backend-developer".into(),
                "backend-developer".into(),
                Duration::from_secs(1200),
            ))
            .await;

        let msg = HarnessMessage::Heartbeat {
            agent_role: "backend-developer".into(),
            task_id: Some("issue-42".into()),
            tokens_used: 1000,
        };

        let response = handle_message(msg, &state).await;
        assert_eq!(response["status"], "ok");
    }

    #[tokio::test]
    async fn test_handle_error_increments_count() {
        let state = test_state();
        state
            .register_agent(AgentHealth::new(
                "backend-1".into(),
                "backend-developer".into(),
                Duration::from_secs(1200),
            ))
            .await;

        let msg = HarnessMessage::Error {
            agent_role: "backend-1".into(),
            error_type: "credentials".into(),
            detail: "GitHub token 401".into(),
            count: 1,
        };

        handle_message(msg, &state).await;

        let health = state.get_agent_health("backend-1").await.unwrap();
        assert_eq!(health.consecutive_errors, 1);
    }

    #[tokio::test]
    async fn test_handle_completed() {
        let state = test_state();
        state
            .register_agent(AgentHealth::new(
                "backend-1".into(),
                "backend-developer".into(),
                Duration::from_secs(1200),
            ))
            .await;

        let msg = HarnessMessage::Completed {
            agent_role: "backend-1".into(),
            task_id: "issue-42".into(),
        };

        handle_message(msg, &state).await;

        let health = state.get_agent_health("backend-1").await.unwrap();
        assert!(matches!(
            health.status,
            caloron_types::agent::AgentStatus::Completing
        ));
    }

    #[tokio::test]
    async fn test_unix_socket_roundtrip() {
        let dir = tempfile::tempdir().unwrap();
        let socket_path = dir.path().join("test.sock");

        let state = test_state();
        state
            .register_agent(AgentHealth::new(
                "test-agent".into(),
                "test".into(),
                Duration::from_secs(1200),
            ))
            .await;

        let server = DaemonSocket::new(socket_path.clone(), state);

        // Spawn server in background
        let server_handle = tokio::spawn(async move {
            server.listen().await.unwrap();
        });

        // Give server time to bind
        tokio::time::sleep(Duration::from_millis(50)).await;

        // Connect as client
        let stream = UnixStream::connect(&socket_path).await.unwrap();
        let (reader, mut writer) = stream.into_split();
        let mut reader = BufReader::new(reader);

        // Send heartbeat
        let msg = serde_json::json!({
            "type": "heartbeat",
            "agent_role": "test-agent",
            "task_id": "issue-1",
            "tokens_used": 500
        });
        writer
            .write_all(format!("{}\n", msg).as_bytes())
            .await
            .unwrap();

        // Read response
        let mut response = String::new();
        reader.read_line(&mut response).await.unwrap();
        let resp: serde_json::Value = serde_json::from_str(&response).unwrap();
        assert_eq!(resp["status"], "ok");

        server_handle.abort();
    }
}
