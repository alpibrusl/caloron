use std::collections::HashMap;
use std::time::Duration;

use chrono::{DateTime, Utc};

use caloron_types::agent::{
    AgentHealth, AgentStatus, ErrorType, HealthVerdict, StallReason,
};

/// Configuration for the health monitor.
#[derive(Debug, Clone)]
pub struct HealthMonitorConfig {
    /// How often to run health checks.
    pub check_interval: Duration,
    /// How long without a heartbeat before declaring process dead.
    pub heartbeat_timeout: Duration,
    /// How many consecutive errors trigger a stall.
    pub max_consecutive_errors: u32,
    /// How many review cycles trigger loop detection.
    pub max_review_cycles: u32,
}

impl Default for HealthMonitorConfig {
    fn default() -> Self {
        Self {
            check_interval: Duration::from_secs(60),
            heartbeat_timeout: Duration::from_secs(300), // 5 minutes
            max_consecutive_errors: 3,
            max_review_cycles: 3,
        }
    }
}

/// Evaluates the health of all running agents and returns verdicts.
pub struct HealthMonitor {
    config: HealthMonitorConfig,
}

/// A health check result for a single agent.
#[derive(Debug, Clone)]
pub struct HealthCheckResult {
    pub agent_id: String,
    pub verdict: HealthVerdict,
    pub details: String,
}

impl HealthMonitor {
    pub fn new(config: HealthMonitorConfig) -> Self {
        Self { config }
    }

    /// Evaluate health for all agents, returning non-healthy results.
    pub fn check_all(&self, agents: &HashMap<String, AgentHealth>) -> Vec<HealthCheckResult> {
        let now = Utc::now();
        agents
            .iter()
            .filter(|(_, health)| {
                // Only check agents that are actively working
                matches!(
                    health.status,
                    AgentStatus::Working | AgentStatus::Idle
                )
            })
            .filter_map(|(id, health)| {
                let verdict = self.evaluate_health(health, now);
                if verdict != HealthVerdict::Healthy {
                    let details = describe_verdict(&verdict, health);
                    Some(HealthCheckResult {
                        agent_id: id.clone(),
                        verdict,
                        details,
                    })
                } else {
                    None
                }
            })
            .collect()
    }

    /// Evaluate the health of a single agent.
    /// Checks are ordered by severity: process death > repeated errors > review loops > stall.
    pub fn evaluate_health(&self, agent: &AgentHealth, now: DateTime<Utc>) -> HealthVerdict {
        // Check 1: heartbeat timeout (process may be dead)
        let heartbeat_elapsed = now
            .signed_duration_since(agent.last_heartbeat)
            .to_std()
            .unwrap_or(Duration::ZERO);

        if heartbeat_elapsed > self.config.heartbeat_timeout {
            return HealthVerdict::ProcessDead;
        }

        // Check 2: repeated identical errors
        if agent.consecutive_errors >= self.config.max_consecutive_errors {
            let error_type = agent
                .error_types
                .last()
                .cloned()
                .unwrap_or(ErrorType::Unknown);
            return HealthVerdict::Stalled(StallReason::RepeatedErrors(error_type));
        }

        // Check 3: review loop on any PR
        for (pr_id, cycles) in &agent.review_cycles {
            if *cycles >= self.config.max_review_cycles {
                return HealthVerdict::ReviewLoopDetected(pr_id.clone());
            }
        }

        // Check 4: no git activity beyond threshold
        let git_elapsed = now
            .signed_duration_since(agent.last_git_event)
            .to_std()
            .unwrap_or(Duration::ZERO);

        if git_elapsed > agent.stall_threshold {
            return HealthVerdict::Stalled(StallReason::NoGitActivity);
        }

        // Check 5: no heartbeat but within timeout (early warning)
        let expected_heartbeat = Duration::from_secs(120); // 2 heartbeat intervals
        if heartbeat_elapsed > expected_heartbeat {
            return HealthVerdict::Stalled(StallReason::NoHeartbeat);
        }

        HealthVerdict::Healthy
    }
}

fn describe_verdict(verdict: &HealthVerdict, agent: &AgentHealth) -> String {
    match verdict {
        HealthVerdict::Healthy => "Healthy".into(),
        HealthVerdict::ProcessDead => format!(
            "Process dead: no heartbeat for {}s",
            Utc::now()
                .signed_duration_since(agent.last_heartbeat)
                .num_seconds()
        ),
        HealthVerdict::Stalled(StallReason::NoGitActivity) => format!(
            "Stalled: no git activity for {}s (threshold: {}s)",
            Utc::now()
                .signed_duration_since(agent.last_git_event)
                .num_seconds(),
            agent.stall_threshold.as_secs()
        ),
        HealthVerdict::Stalled(StallReason::NoHeartbeat) => format!(
            "Stalled: no heartbeat for {}s",
            Utc::now()
                .signed_duration_since(agent.last_heartbeat)
                .num_seconds()
        ),
        HealthVerdict::Stalled(StallReason::RepeatedErrors(err)) => format!(
            "Stalled: {} consecutive errors, last: {:?}",
            agent.consecutive_errors, err
        ),
        HealthVerdict::Stalled(StallReason::ReviewLoopDetected { pr_id }) => format!(
            "Stalled: review loop on PR {pr_id}"
        ),
        HealthVerdict::ReviewLoopDetected(pr_id) => format!(
            "Review loop detected on PR {pr_id}"
        ),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_healthy_agent(id: &str) -> (String, AgentHealth) {
        let mut health = AgentHealth::new(id.into(), "developer".into(), Duration::from_secs(1200));
        health.status = AgentStatus::Working;
        (id.into(), health)
    }

    fn monitor() -> HealthMonitor {
        HealthMonitor::new(HealthMonitorConfig::default())
    }

    #[test]
    fn test_healthy_agent() {
        let agent = AgentHealth::new("test".into(), "dev".into(), Duration::from_secs(1200));
        let verdict = monitor().evaluate_health(&agent, Utc::now());
        assert_eq!(verdict, HealthVerdict::Healthy);
    }

    #[test]
    fn test_process_dead_no_heartbeat() {
        let mut agent = AgentHealth::new("test".into(), "dev".into(), Duration::from_secs(1200));
        agent.last_heartbeat = Utc::now() - chrono::Duration::minutes(10);

        let verdict = monitor().evaluate_health(&agent, Utc::now());
        assert_eq!(verdict, HealthVerdict::ProcessDead);
    }

    #[test]
    fn test_stall_no_git_activity() {
        let mut agent = AgentHealth::new("test".into(), "dev".into(), Duration::from_secs(600));
        // 600s threshold, 15 min without activity
        agent.last_git_event = Utc::now() - chrono::Duration::minutes(15);

        let verdict = monitor().evaluate_health(&agent, Utc::now());
        assert_eq!(verdict, HealthVerdict::Stalled(StallReason::NoGitActivity));
    }

    #[test]
    fn test_stall_repeated_errors() {
        let mut agent = AgentHealth::new("test".into(), "dev".into(), Duration::from_secs(1200));
        for _ in 0..3 {
            agent.record_error(ErrorType::CredentialsFailure {
                tool: "github".into(),
            });
        }

        let verdict = monitor().evaluate_health(&agent, Utc::now());
        assert!(matches!(
            verdict,
            HealthVerdict::Stalled(StallReason::RepeatedErrors(_))
        ));
    }

    #[test]
    fn test_review_loop_detected() {
        let mut agent = AgentHealth::new("test".into(), "dev".into(), Duration::from_secs(1200));
        agent.review_cycles.insert("pr-42".into(), 3);

        let verdict = monitor().evaluate_health(&agent, Utc::now());
        assert_eq!(verdict, HealthVerdict::ReviewLoopDetected("pr-42".into()));
    }

    #[test]
    fn test_stall_no_heartbeat_early_warning() {
        let mut agent = AgentHealth::new("test".into(), "dev".into(), Duration::from_secs(1200));
        // More than 2 heartbeat intervals but less than timeout
        agent.last_heartbeat = Utc::now() - chrono::Duration::minutes(3);

        let verdict = monitor().evaluate_health(&agent, Utc::now());
        assert_eq!(verdict, HealthVerdict::Stalled(StallReason::NoHeartbeat));
    }

    #[test]
    fn test_check_severity_order() {
        // Process dead takes priority over stall
        let mut agent = AgentHealth::new("test".into(), "dev".into(), Duration::from_secs(600));
        agent.last_heartbeat = Utc::now() - chrono::Duration::minutes(10);
        agent.last_git_event = Utc::now() - chrono::Duration::minutes(20);

        let verdict = monitor().evaluate_health(&agent, Utc::now());
        assert_eq!(verdict, HealthVerdict::ProcessDead);
    }

    #[test]
    fn test_errors_take_priority_over_stall() {
        let mut agent = AgentHealth::new("test".into(), "dev".into(), Duration::from_secs(600));
        agent.last_git_event = Utc::now() - chrono::Duration::minutes(15);
        for _ in 0..3 {
            agent.record_error(ErrorType::RateLimited {
                tool: "github".into(),
            });
        }

        let verdict = monitor().evaluate_health(&agent, Utc::now());
        // Errors checked before git activity
        assert!(matches!(
            verdict,
            HealthVerdict::Stalled(StallReason::RepeatedErrors(_))
        ));
    }

    #[test]
    fn test_check_all_skips_non_working_agents() {
        let mut agents = HashMap::new();

        let (id, mut agent) = make_healthy_agent("working-1");
        agent.last_git_event = Utc::now() - chrono::Duration::minutes(30);
        agents.insert(id, agent);

        let (id, mut agent) = make_healthy_agent("destroyed-1");
        agent.status = AgentStatus::Destroyed;
        agent.last_git_event = Utc::now() - chrono::Duration::minutes(30);
        agents.insert(id, agent);

        let results = monitor().check_all(&agents);
        // Only the working agent should be checked
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].agent_id, "working-1");
    }

    #[test]
    fn test_check_all_returns_empty_for_healthy() {
        let mut agents = HashMap::new();
        agents.insert("a".into(), {
            let mut h = AgentHealth::new("a".into(), "dev".into(), Duration::from_secs(1200));
            h.status = AgentStatus::Working;
            h
        });

        let results = monitor().check_all(&agents);
        assert!(results.is_empty());
    }

    #[test]
    fn test_cleared_errors_restore_health() {
        let mut agent = AgentHealth::new("test".into(), "dev".into(), Duration::from_secs(1200));
        for _ in 0..3 {
            agent.record_error(ErrorType::Unknown);
        }
        assert!(matches!(
            monitor().evaluate_health(&agent, Utc::now()),
            HealthVerdict::Stalled(_)
        ));

        agent.clear_errors();
        assert_eq!(
            monitor().evaluate_health(&agent, Utc::now()),
            HealthVerdict::Healthy
        );
    }
}
