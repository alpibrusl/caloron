use anyhow::Result;

use crate::git::GitHubClient;

/// Creates structured escalation issues on GitHub and monitors for human responses.
pub struct EscalationGateway;

/// Types of escalation issues the supervisor can create.
#[derive(Debug, Clone)]
pub enum EscalationIssue {
    CredentialsFailure {
        agent_role: String,
        tool: String,
        task_issue_number: u64,
        error_count: u32,
    },
    TaskBeyondCapability {
        agent_role: String,
        task_issue_number: u64,
        task_title: String,
        attempts: usize,
        failure_pattern: String,
    },
    ReviewLoop {
        pr_number: u64,
        reviewer_id: String,
        author_id: String,
        cycles: u32,
        analysis: String,
    },
    SupervisorDown {
        sprint_id: String,
        running_agents: usize,
        restart_attempts: u32,
    },
}

impl EscalationGateway {
    /// Create an escalation issue on GitHub.
    pub async fn escalate(
        client: &GitHubClient,
        escalation: &EscalationIssue,
    ) -> Result<u64> {
        let (title, body) = format_escalation(escalation);

        let issue_number = client
            .create_issue(&title, &body, &["caloron:escalated"])
            .await?;

        tracing::warn!(
            issue_number,
            escalation_type = escalation.type_name(),
            "Created escalation issue"
        );

        Ok(issue_number)
    }

    /// Check if a human has responded to an escalation issue.
    /// Looks for specific comment patterns: "resolved", "assign-human", "break-down", "caloron:take-over".
    pub async fn check_response(
        _client: &GitHubClient,
        _issue_number: u64,
    ) -> Result<Option<HumanResponse>> {
        // TODO: Implement in Phase 4 when Git Monitor can fetch comments.
        // For now, returns None (no response yet).
        Ok(None)
    }
}

/// Possible human responses to an escalation.
#[derive(Debug, Clone, PartialEq)]
pub enum HumanResponse {
    /// Human says the issue is resolved (e.g., fixed credentials).
    Resolved,
    /// Human will handle the task directly.
    TakeOver,
    /// Human wants the task broken into subtasks.
    BreakDown,
    /// Human assigned it to a human developer.
    AssignHuman,
}

impl EscalationIssue {
    fn type_name(&self) -> &'static str {
        match self {
            EscalationIssue::CredentialsFailure { .. } => "credentials_failure",
            EscalationIssue::TaskBeyondCapability { .. } => "task_beyond_capability",
            EscalationIssue::ReviewLoop { .. } => "review_loop",
            EscalationIssue::SupervisorDown { .. } => "supervisor_down",
        }
    }
}

fn format_escalation(escalation: &EscalationIssue) -> (String, String) {
    match escalation {
        EscalationIssue::CredentialsFailure {
            agent_role,
            tool,
            task_issue_number,
            error_count,
        } => {
            let title = format!("Escalation: credentials failure ({tool})");
            let body = format!(
                "## Human intervention required\n\n\
                 **Agent:** {agent_role}\n\
                 **Problem:** Credentials failure ({error_count} consecutive 401 errors)\n\
                 **Tool:** {tool}\n\
                 **Task:** #{task_issue_number}\n\n\
                 The agent cannot proceed until credentials are resolved.\n\
                 Please check {tool} token configuration and comment `resolved` when fixed.\n\n\
                 ---\n\
                 *Created by Caloron Supervisor*"
            );
            (title, body)
        }

        EscalationIssue::TaskBeyondCapability {
            agent_role,
            task_issue_number,
            task_title,
            attempts,
            failure_pattern,
        } => {
            let title = format!("Escalation: task #{task_issue_number} beyond agent capability");
            let body = format!(
                "## Task escalation required\n\n\
                 **Task:** #{task_issue_number} — {task_title}\n\
                 **Agent:** {agent_role} (attempted {attempts} times)\n\
                 **Failure pattern:** {failure_pattern}\n\n\
                 This task appears to require capabilities beyond the current agent configuration.\n\n\
                 **Options:**\n\
                 1. Clarify the task specification (comment with clarification)\n\
                 2. Assign to a human developer (comment `assign-human`)\n\
                 3. Break down into smaller subtasks (comment `break-down`)\n\n\
                 ---\n\
                 *Created by Caloron Supervisor*"
            );
            (title, body)
        }

        EscalationIssue::ReviewLoop {
            pr_number,
            reviewer_id,
            author_id,
            cycles,
            analysis,
        } => {
            let title = format!("Escalation: review loop on PR #{pr_number}");
            let body = format!(
                "## Review loop escalation\n\n\
                 **PR:** #{pr_number}\n\
                 **Reviewer:** @caloron-agent-{reviewer_id}\n\
                 **Author:** @caloron-agent-{author_id}\n\
                 **Cycles:** {cycles}\n\n\
                 Mediation was attempted but the loop continues.\n\n\
                 **Supervisor analysis:**\n{analysis}\n\n\
                 Please review PR #{pr_number} and either:\n\
                 - Comment with the resolution direction\n\
                 - Comment `caloron:take-over` to handle it directly\n\n\
                 ---\n\
                 *Created by Caloron Supervisor*"
            );
            (title, body)
        }

        EscalationIssue::SupervisorDown {
            sprint_id,
            running_agents,
            restart_attempts,
        } => {
            let title = "CRITICAL: Supervisor process unresponsive".into();
            let body = format!(
                "## CRITICAL: Supervisor process unresponsive\n\n\
                 The Supervisor agent has failed to produce a heartbeat.\n\
                 Automatic restart has been attempted {restart_attempts} times.\n\n\
                 **Sprint:** {sprint_id}\n\
                 **Running agents:** {running_agents} (operating without health monitoring)\n\n\
                 **Immediate action required:**\n\
                 1. Check daemon logs: `caloron logs supervisor`\n\
                 2. Manually restart: `caloron supervisor restart`\n\
                 3. If persistent, stop the sprint: `caloron stop`\n\n\
                 ---\n\
                 *Created by Caloron Daemon Watchdog*"
            );
            (title, body)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_credentials_escalation_format() {
        let esc = EscalationIssue::CredentialsFailure {
            agent_role: "backend-developer".into(),
            tool: "github_mcp".into(),
            task_issue_number: 42,
            error_count: 3,
        };

        let (title, body) = format_escalation(&esc);
        assert!(title.contains("credentials failure"));
        assert!(body.contains("backend-developer"));
        assert!(body.contains("#42"));
        assert!(body.contains("`resolved`"));
    }

    #[test]
    fn test_capability_escalation_format() {
        let esc = EscalationIssue::TaskBeyondCapability {
            agent_role: "backend-developer".into(),
            task_issue_number: 15,
            task_title: "Implement complex auth".into(),
            attempts: 3,
            failure_pattern: "Agent stalls at database migration step".into(),
        };

        let (title, body) = format_escalation(&esc);
        assert!(title.contains("#15"));
        assert!(body.contains("3 times"));
        assert!(body.contains("break-down"));
    }

    #[test]
    fn test_review_loop_escalation_format() {
        let esc = EscalationIssue::ReviewLoop {
            pr_number: 47,
            reviewer_id: "reviewer-1".into(),
            author_id: "backend-1".into(),
            cycles: 4,
            analysis: "Reviewer expects error handling; author says spec doesn't require it.".into(),
        };

        let (title, body) = format_escalation(&esc);
        assert!(title.contains("PR #47"));
        assert!(body.contains("@caloron-agent-reviewer-1"));
        assert!(body.contains("caloron:take-over"));
    }

    #[test]
    fn test_supervisor_down_escalation_format() {
        let esc = EscalationIssue::SupervisorDown {
            sprint_id: "sprint-2026-04-w2".into(),
            running_agents: 3,
            restart_attempts: 3,
        };

        let (title, body) = format_escalation(&esc);
        assert!(title.contains("CRITICAL"));
        assert!(body.contains("3 times"));
        assert!(body.contains("caloron stop"));
    }

    #[test]
    fn test_escalation_type_names() {
        assert_eq!(
            EscalationIssue::CredentialsFailure {
                agent_role: "".into(),
                tool: "".into(),
                task_issue_number: 0,
                error_count: 0
            }
            .type_name(),
            "credentials_failure"
        );
    }
}
