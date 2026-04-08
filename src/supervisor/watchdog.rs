use std::time::Duration;

use caloron_types::agent::{SupervisorWatchdog, WatchdogVerdict};

use crate::daemon::state::DaemonState;
use super::escalation::{EscalationGateway, EscalationIssue};
use crate::git::GitHubClient;

/// Runs the supervisor watchdog loop at the daemon level.
/// This is NOT part of the supervisor — it monitors the supervisor from outside.
pub async fn run_watchdog_loop(
    state: DaemonState,
    github: GitHubClient,
    sprint_id: String,
    check_interval: Duration,
) {
    let mut watchdog = SupervisorWatchdog::default();

    loop {
        tokio::time::sleep(check_interval).await;

        let verdict = watchdog.check();

        match verdict {
            WatchdogVerdict::Healthy => {
                tracing::trace!("Supervisor watchdog: healthy");
            }

            WatchdogVerdict::RestartSupervisor => {
                tracing::warn!(
                    restart_count = watchdog.restart_count + 1,
                    "Supervisor watchdog: restarting supervisor"
                );
                watchdog.record_restart();

                // TODO: Actually restart the supervisor process via AgentSpawner.
                // For now, log the intent. The restart mechanism will be
                // wired in when the supervisor runs as a real agent process.
                tracing::info!("Supervisor restart requested");
            }

            WatchdogVerdict::EscalateToHuman => {
                tracing::error!(
                    restart_count = watchdog.restart_count,
                    "Supervisor watchdog: max restarts exceeded, escalating to human"
                );

                let agents_count = state.all_agent_health().await.len();

                let escalation = EscalationIssue::SupervisorDown {
                    sprint_id: sprint_id.clone(),
                    running_agents: agents_count,
                    restart_attempts: watchdog.restart_count,
                };

                match EscalationGateway::escalate(&github, &escalation).await {
                    Ok(issue) => {
                        tracing::info!(
                            issue_number = issue,
                            "Created supervisor-down escalation issue"
                        );
                    }
                    Err(e) => {
                        tracing::error!(
                            error = %e,
                            "Failed to create escalation issue — supervisor is down AND escalation failed"
                        );
                    }
                }

                // Reset watchdog after escalation so we don't spam issues
                watchdog = SupervisorWatchdog::default();
                watchdog.restart_count = 0;

                // Wait longer before next check after escalation
                tokio::time::sleep(Duration::from_secs(300)).await;
            }
        }
    }
}

/// Called by the daemon when it receives a supervisor heartbeat.
/// Updates the watchdog state.
pub fn record_supervisor_heartbeat(watchdog: &mut SupervisorWatchdog) {
    watchdog.record_heartbeat();
    tracing::trace!("Supervisor heartbeat recorded");
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Utc;

    #[test]
    fn test_watchdog_healthy_after_heartbeat() {
        let mut watchdog = SupervisorWatchdog::default();
        watchdog.record_heartbeat();
        assert_eq!(watchdog.check(), WatchdogVerdict::Healthy);
    }

    #[test]
    fn test_watchdog_restart_after_missed_heartbeats() {
        let watchdog = SupervisorWatchdog {
            last_heartbeat: Utc::now() - chrono::Duration::minutes(5),
            restart_count: 0,
            ..Default::default()
        };
        assert_eq!(watchdog.check(), WatchdogVerdict::RestartSupervisor);
    }

    #[test]
    fn test_watchdog_escalate_after_max_restarts() {
        let watchdog = SupervisorWatchdog {
            last_heartbeat: Utc::now() - chrono::Duration::minutes(5),
            restart_count: 3,
            max_restarts: 3,
            ..Default::default()
        };
        assert_eq!(watchdog.check(), WatchdogVerdict::EscalateToHuman);
    }

    #[test]
    fn test_record_restart_increments() {
        let mut watchdog = SupervisorWatchdog::default();
        assert_eq!(watchdog.restart_count, 0);
        watchdog.record_restart();
        assert_eq!(watchdog.restart_count, 1);
        watchdog.record_restart();
        assert_eq!(watchdog.restart_count, 2);
    }
}
