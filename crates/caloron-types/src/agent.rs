use std::collections::HashMap;
use std::time::Duration;

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

/// An agent definition as loaded from YAML in caloron-meta/agents/.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentDefinition {
    pub name: String,
    pub version: String,
    pub description: String,
    pub llm: LlmConfig,
    pub system_prompt: String,
    pub tools: Vec<String>,
    #[serde(default)]
    pub mcps: Vec<McpConfig>,
    #[serde(default)]
    pub nix: NixConfig,
    #[serde(default)]
    pub credentials: Vec<String>,
    #[serde(default = "default_stall_threshold")]
    pub stall_threshold_minutes: u32,
    #[serde(default = "default_max_review_cycles")]
    pub max_review_cycles: u32,
}

fn default_stall_threshold() -> u32 {
    20
}

fn default_max_review_cycles() -> u32 {
    3
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LlmConfig {
    /// Model name or alias (resolved via caloron.toml [llm.aliases])
    pub model: String,
    #[serde(default = "default_max_tokens")]
    pub max_tokens: u32,
    #[serde(default = "default_temperature")]
    pub temperature: f32,
}

fn default_max_tokens() -> u32 {
    8192
}

fn default_temperature() -> f32 {
    0.2
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct McpConfig {
    pub url: String,
    pub name: String,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct NixConfig {
    #[serde(default)]
    pub packages: Vec<String>,
    #[serde(default)]
    pub env: HashMap<String, String>,
}

/// Runtime health state for a running agent.
#[derive(Debug, Clone)]
pub struct AgentHealth {
    pub agent_id: String,
    pub role: String,
    pub current_task_id: Option<String>,
    pub last_git_event: DateTime<Utc>,
    pub last_heartbeat: DateTime<Utc>,
    pub consecutive_errors: u32,
    pub error_types: Vec<ErrorType>,
    pub review_cycles: HashMap<String, u32>,
    pub status: AgentStatus,
    pub stall_threshold: Duration,
    pub spawn_time: DateTime<Utc>,
}

impl AgentHealth {
    pub fn new(agent_id: String, role: String, stall_threshold: Duration) -> Self {
        let now = Utc::now();
        Self {
            agent_id,
            role,
            current_task_id: None,
            last_git_event: now,
            last_heartbeat: now,
            consecutive_errors: 0,
            error_types: Vec::new(),
            review_cycles: HashMap::new(),
            status: AgentStatus::Idle,
            stall_threshold,
            spawn_time: now,
        }
    }

    pub fn record_heartbeat(&mut self) {
        self.last_heartbeat = Utc::now();
    }

    pub fn record_git_event(&mut self) {
        self.last_git_event = Utc::now();
    }

    pub fn record_error(&mut self, error_type: ErrorType) {
        self.consecutive_errors += 1;
        self.error_types.push(error_type);
    }

    pub fn clear_errors(&mut self) {
        self.consecutive_errors = 0;
    }

    pub fn increment_review_cycle(&mut self, pr_id: &str) -> u32 {
        let count = self.review_cycles.entry(pr_id.to_string()).or_insert(0);
        *count += 1;
        *count
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum AgentStatus {
    Idle,
    Working,
    Stalled {
        since: DateTime<Utc>,
        reason: StallReason,
    },
    Blocked {
        reason: String,
    },
    Failed {
        error: String,
    },
    Completing,
    Destroyed,
}

#[derive(Debug, Clone, PartialEq)]
pub enum StallReason {
    NoGitActivity,
    NoHeartbeat,
    RepeatedErrors(ErrorType),
    ReviewLoopDetected { pr_id: String },
}

#[derive(Debug, Clone, PartialEq)]
pub enum ErrorType {
    CredentialsFailure { tool: String },
    RateLimited { tool: String },
    ToolUnavailable { tool: String },
    Unknown,
}

#[derive(Debug, Clone, PartialEq)]
pub enum HealthVerdict {
    Healthy,
    ProcessDead,
    Stalled(StallReason),
    ReviewLoopDetected(String),
}

/// Messages sent between the harness and the daemon over the Unix socket.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum HarnessMessage {
    Heartbeat {
        agent_role: String,
        task_id: Option<String>,
        tokens_used: u64,
    },
    Status {
        agent_role: String,
        status: String,
        detail: String,
    },
    Error {
        agent_role: String,
        error_type: String,
        detail: String,
        count: u32,
    },
    Completed {
        agent_role: String,
        task_id: String,
    },
}

/// Watchdog state for monitoring the Supervisor process (Addendum H1).
#[derive(Debug, Clone)]
pub struct SupervisorWatchdog {
    pub last_heartbeat: DateTime<Utc>,
    pub heartbeat_interval: Duration,
    pub max_missed_heartbeats: u32,
    pub restart_count: u32,
    pub max_restarts: u32,
}

impl Default for SupervisorWatchdog {
    fn default() -> Self {
        Self {
            last_heartbeat: Utc::now(),
            heartbeat_interval: Duration::from_secs(60),
            max_missed_heartbeats: 2,
            restart_count: 0,
            max_restarts: 3,
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum WatchdogVerdict {
    Healthy,
    RestartSupervisor,
    EscalateToHuman,
}

impl SupervisorWatchdog {
    pub fn check(&self) -> WatchdogVerdict {
        let now = Utc::now();
        let elapsed = (now - self.last_heartbeat)
            .to_std()
            .unwrap_or(Duration::ZERO);
        let missed = elapsed.as_secs() / self.heartbeat_interval.as_secs();

        if missed > self.max_missed_heartbeats as u64 {
            if self.restart_count >= self.max_restarts {
                return WatchdogVerdict::EscalateToHuman;
            }
            return WatchdogVerdict::RestartSupervisor;
        }
        WatchdogVerdict::Healthy
    }

    pub fn record_heartbeat(&mut self) {
        self.last_heartbeat = Utc::now();
    }

    pub fn record_restart(&mut self) {
        self.restart_count += 1;
        self.last_heartbeat = Utc::now();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_agent_health_error_tracking() {
        let mut health =
            AgentHealth::new("backend-1".into(), "backend-developer".into(), Duration::from_secs(1200));

        assert_eq!(health.consecutive_errors, 0);

        health.record_error(ErrorType::CredentialsFailure {
            tool: "github".into(),
        });
        health.record_error(ErrorType::CredentialsFailure {
            tool: "github".into(),
        });
        assert_eq!(health.consecutive_errors, 2);

        health.clear_errors();
        assert_eq!(health.consecutive_errors, 0);
    }

    #[test]
    fn test_review_cycle_tracking() {
        let mut health =
            AgentHealth::new("reviewer-1".into(), "reviewer".into(), Duration::from_secs(1200));

        assert_eq!(health.increment_review_cycle("pr-42"), 1);
        assert_eq!(health.increment_review_cycle("pr-42"), 2);
        assert_eq!(health.increment_review_cycle("pr-43"), 1);
        assert_eq!(health.increment_review_cycle("pr-42"), 3);
    }

    #[test]
    fn test_watchdog_healthy() {
        let watchdog = SupervisorWatchdog::default();
        assert_eq!(watchdog.check(), WatchdogVerdict::Healthy);
    }

    #[test]
    fn test_watchdog_escalate_after_max_restarts() {
        let mut watchdog = SupervisorWatchdog {
            last_heartbeat: Utc::now() - chrono::Duration::minutes(10),
            restart_count: 3,
            max_restarts: 3,
            ..Default::default()
        };
        assert_eq!(watchdog.check(), WatchdogVerdict::EscalateToHuman);

        // After recording heartbeat, should be healthy again
        watchdog.record_heartbeat();
        assert_eq!(watchdog.check(), WatchdogVerdict::Healthy);
    }

    #[test]
    fn test_agent_definition_yaml_parsing() {
        let yaml = r#"
name: backend-developer
version: "1.0"
description: "Implements backend features"
llm:
  model: default
  max_tokens: 8192
  temperature: 0.2
system_prompt: "You are a backend developer."
tools:
  - github_mcp
  - bash
mcps:
  - url: "https://github.mcp.claude.com/mcp"
    name: "github"
nix:
  packages:
    - nodejs_20
    - rustc
  env:
    NODE_ENV: "test"
credentials:
  - GITHUB_TOKEN
  - ANTHROPIC_API_KEY
stall_threshold_minutes: 20
max_review_cycles: 3
"#;
        let def: AgentDefinition = serde_yaml::from_str(yaml).unwrap();
        assert_eq!(def.name, "backend-developer");
        assert_eq!(def.tools.len(), 2);
        assert_eq!(def.nix.packages.len(), 2);
        assert_eq!(def.credentials.len(), 2);
    }

    #[test]
    fn test_agent_definition_defaults() {
        let yaml = r#"
name: minimal
version: "1.0"
description: "Minimal agent"
llm:
  model: default
system_prompt: "You are an agent."
tools:
  - bash
"#;
        let def: AgentDefinition = serde_yaml::from_str(yaml).unwrap();
        assert_eq!(def.stall_threshold_minutes, 20);
        assert_eq!(def.max_review_cycles, 3);
        assert_eq!(def.llm.max_tokens, 8192);
        assert!(def.mcps.is_empty());
        assert!(def.credentials.is_empty());
    }

    #[test]
    fn test_harness_message_serialization() {
        let msg = HarnessMessage::Heartbeat {
            agent_role: "backend-developer".into(),
            task_id: Some("issue-42".into()),
            tokens_used: 4200,
        };
        let json = serde_json::to_string(&msg).unwrap();
        assert!(json.contains("\"type\":\"heartbeat\""));

        let deserialized: HarnessMessage = serde_json::from_str(&json).unwrap();
        if let HarnessMessage::Heartbeat { tokens_used, .. } = deserialized {
            assert_eq!(tokens_used, 4200);
        } else {
            panic!("wrong variant");
        }
    }
}
