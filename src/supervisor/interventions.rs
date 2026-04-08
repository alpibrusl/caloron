use std::collections::HashMap;

use chrono::{DateTime, Utc};

use caloron_types::agent::{ErrorType, HealthVerdict, StallReason};

use super::health_monitor::HealthCheckResult;

/// Tracks intervention history to avoid infinite retry loops.
#[derive(Debug, Clone, Default)]
pub struct InterventionTracker {
    /// History per task: task_id -> list of interventions
    history: HashMap<String, Vec<Intervention>>,
}

#[derive(Debug, Clone)]
pub struct Intervention {
    pub timestamp: DateTime<Utc>,
    pub action: InterventionAction,
    pub agent_id: String,
    pub task_id: String,
}

/// Actions the supervisor can take in response to health issues.
#[derive(Debug, Clone, PartialEq)]
pub enum InterventionAction {
    /// Post a probe comment asking the agent for status.
    Probe,
    /// Restart the agent with the same task context.
    Restart,
    /// Reassign the task to a different agent.
    Reassign { new_agent_id: String },
    /// Escalate to human with structured issue.
    EscalateCredentials { tool: String },
    /// Escalate a review loop with analysis.
    EscalateReviewLoop { pr_id: String },
    /// Escalate because task is beyond agent capability.
    EscalateCapability,
    /// Block the task (no further automatic action).
    Block { reason: String },
}

/// Decides what intervention to take based on the health verdict and history.
pub struct InterventionDecider;

impl InterventionDecider {
    /// Given a health check result, decide what to do.
    pub fn decide(
        result: &HealthCheckResult,
        task_id: &str,
        tracker: &InterventionTracker,
    ) -> InterventionAction {
        let history = tracker.history_for_task(task_id);

        match &result.verdict {
            HealthVerdict::Healthy => unreachable!("Should not be called for healthy agents"),

            HealthVerdict::ProcessDead => {
                // Always restart on process death
                let restart_count = history
                    .iter()
                    .filter(|i| matches!(i.action, InterventionAction::Restart))
                    .count();

                if restart_count >= 2 {
                    InterventionAction::EscalateCapability
                } else {
                    InterventionAction::Restart
                }
            }

            HealthVerdict::Stalled(StallReason::NoGitActivity) => {
                let probe_count = history
                    .iter()
                    .filter(|i| matches!(i.action, InterventionAction::Probe))
                    .count();
                let restart_count = history
                    .iter()
                    .filter(|i| matches!(i.action, InterventionAction::Restart))
                    .count();

                if probe_count == 0 {
                    // First stall: probe
                    InterventionAction::Probe
                } else if restart_count == 0 {
                    // Probe didn't help: restart
                    InterventionAction::Restart
                } else {
                    // Restart didn't help: escalate
                    InterventionAction::EscalateCapability
                }
            }

            HealthVerdict::Stalled(StallReason::NoHeartbeat) => {
                // Heartbeat gap but not dead yet — probe first
                let probe_count = history
                    .iter()
                    .filter(|i| matches!(i.action, InterventionAction::Probe))
                    .count();

                if probe_count == 0 {
                    InterventionAction::Probe
                } else {
                    InterventionAction::Restart
                }
            }

            HealthVerdict::Stalled(StallReason::RepeatedErrors(error_type)) => {
                match error_type {
                    ErrorType::CredentialsFailure { tool } => {
                        // Credentials errors cannot be fixed by the supervisor
                        InterventionAction::EscalateCredentials {
                            tool: tool.clone(),
                        }
                    }
                    ErrorType::RateLimited { .. } => {
                        // Rate limiting: just wait. Block the task temporarily.
                        InterventionAction::Block {
                            reason: "Rate limited — waiting for cooldown".into(),
                        }
                    }
                    ErrorType::ToolUnavailable { tool } => {
                        InterventionAction::Block {
                            reason: format!("Tool '{tool}' unavailable"),
                        }
                    }
                    ErrorType::Unknown => {
                        let restart_count = history
                            .iter()
                            .filter(|i| matches!(i.action, InterventionAction::Restart))
                            .count();
                        if restart_count == 0 {
                            InterventionAction::Restart
                        } else {
                            InterventionAction::EscalateCapability
                        }
                    }
                }
            }

            HealthVerdict::Stalled(StallReason::ReviewLoopDetected { pr_id })
            | HealthVerdict::ReviewLoopDetected(pr_id) => {
                InterventionAction::EscalateReviewLoop {
                    pr_id: pr_id.clone(),
                }
            }
        }
    }
}

impl InterventionTracker {
    pub fn new() -> Self {
        Self::default()
    }

    /// Record an intervention.
    pub fn record(
        &mut self,
        task_id: &str,
        agent_id: &str,
        action: InterventionAction,
    ) {
        let intervention = Intervention {
            timestamp: Utc::now(),
            action,
            agent_id: agent_id.to_string(),
            task_id: task_id.to_string(),
        };
        self.history
            .entry(task_id.to_string())
            .or_default()
            .push(intervention);
    }

    /// Get intervention history for a task.
    pub fn history_for_task(&self, task_id: &str) -> &[Intervention] {
        self.history
            .get(task_id)
            .map(|v| v.as_slice())
            .unwrap_or(&[])
    }

    /// Count total interventions for a task.
    pub fn intervention_count(&self, task_id: &str) -> usize {
        self.history_for_task(task_id).len()
    }
}

/// Generate a probe comment for a stalled agent.
pub fn probe_comment(agent_id: &str, minutes_stalled: u64) -> String {
    format!(
        "@caloron-agent-{agent_id} You have had no activity for {minutes_stalled} minutes on this task. \
         Please respond with your current status. If you are blocked, describe what you need."
    )
}

/// Generate a mediation comment for a review loop.
pub fn mediation_comment(
    reviewer_id: &str,
    author_id: &str,
    pr_id: &str,
    cycles: u32,
    analysis: &str,
) -> String {
    format!(
        "@caloron-agent-{reviewer_id} @caloron-agent-{author_id}\n\n\
         This PR has been through {cycles} review cycles without resolution. \
         I have analyzed the review thread and identified the core disagreement:\n\n\
         {analysis}\n\n\
         Both agents should acknowledge this resolution with a thumbs-up reaction. \
         If either disagrees, I will escalate to the human operator."
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use caloron_types::agent::HealthVerdict;

    fn stall_result(agent_id: &str) -> HealthCheckResult {
        HealthCheckResult {
            agent_id: agent_id.into(),
            verdict: HealthVerdict::Stalled(StallReason::NoGitActivity),
            details: "test".into(),
        }
    }

    fn cred_result(agent_id: &str) -> HealthCheckResult {
        HealthCheckResult {
            agent_id: agent_id.into(),
            verdict: HealthVerdict::Stalled(StallReason::RepeatedErrors(
                ErrorType::CredentialsFailure {
                    tool: "github".into(),
                },
            )),
            details: "test".into(),
        }
    }

    #[test]
    fn test_first_stall_probes() {
        let tracker = InterventionTracker::new();
        let action = InterventionDecider::decide(&stall_result("a"), "task-1", &tracker);
        assert_eq!(action, InterventionAction::Probe);
    }

    #[test]
    fn test_second_stall_restarts() {
        let mut tracker = InterventionTracker::new();
        tracker.record("task-1", "a", InterventionAction::Probe);

        let action = InterventionDecider::decide(&stall_result("a"), "task-1", &tracker);
        assert_eq!(action, InterventionAction::Restart);
    }

    #[test]
    fn test_third_stall_escalates() {
        let mut tracker = InterventionTracker::new();
        tracker.record("task-1", "a", InterventionAction::Probe);
        tracker.record("task-1", "a", InterventionAction::Restart);

        let action = InterventionDecider::decide(&stall_result("a"), "task-1", &tracker);
        assert_eq!(action, InterventionAction::EscalateCapability);
    }

    #[test]
    fn test_credentials_always_escalates() {
        let tracker = InterventionTracker::new();
        let action = InterventionDecider::decide(&cred_result("a"), "task-1", &tracker);
        assert!(matches!(
            action,
            InterventionAction::EscalateCredentials { .. }
        ));
    }

    #[test]
    fn test_process_dead_restarts_then_escalates() {
        let result = HealthCheckResult {
            agent_id: "a".into(),
            verdict: HealthVerdict::ProcessDead,
            details: "test".into(),
        };

        let tracker = InterventionTracker::new();
        let action = InterventionDecider::decide(&result, "task-1", &tracker);
        assert_eq!(action, InterventionAction::Restart);

        let mut tracker = InterventionTracker::new();
        tracker.record("task-1", "a", InterventionAction::Restart);
        tracker.record("task-1", "a", InterventionAction::Restart);

        let action = InterventionDecider::decide(&result, "task-1", &tracker);
        assert_eq!(action, InterventionAction::EscalateCapability);
    }

    #[test]
    fn test_review_loop_escalates_immediately() {
        let result = HealthCheckResult {
            agent_id: "a".into(),
            verdict: HealthVerdict::ReviewLoopDetected("pr-42".into()),
            details: "test".into(),
        };
        let tracker = InterventionTracker::new();
        let action = InterventionDecider::decide(&result, "task-1", &tracker);
        assert_eq!(
            action,
            InterventionAction::EscalateReviewLoop {
                pr_id: "pr-42".into()
            }
        );
    }

    #[test]
    fn test_rate_limited_blocks() {
        let result = HealthCheckResult {
            agent_id: "a".into(),
            verdict: HealthVerdict::Stalled(StallReason::RepeatedErrors(
                ErrorType::RateLimited {
                    tool: "github".into(),
                },
            )),
            details: "test".into(),
        };
        let tracker = InterventionTracker::new();
        let action = InterventionDecider::decide(&result, "task-1", &tracker);
        assert!(matches!(action, InterventionAction::Block { .. }));
    }

    #[test]
    fn test_probe_comment_format() {
        let comment = probe_comment("backend-1", 20);
        assert!(comment.contains("@caloron-agent-backend-1"));
        assert!(comment.contains("20 minutes"));
    }

    #[test]
    fn test_intervention_tracker_counts() {
        let mut tracker = InterventionTracker::new();
        assert_eq!(tracker.intervention_count("task-1"), 0);

        tracker.record("task-1", "a", InterventionAction::Probe);
        tracker.record("task-1", "a", InterventionAction::Restart);
        assert_eq!(tracker.intervention_count("task-1"), 2);
        assert_eq!(tracker.intervention_count("task-2"), 0);
    }
}
