use anyhow::{Result, Context};

use caloron_types::dag::TaskStatus;
use caloron_types::feedback::FeedbackComment;
use caloron_types::git::{GitEvent, ReviewState, labels};

use crate::dag::engine::DagEngine;

/// Result of handling a Git event — describes what the orchestrator should do next.
#[derive(Debug, Clone, PartialEq)]
pub enum OrchestratorAction {
    /// No action needed.
    None,
    /// Spawn an agent for a task that is now ready.
    SpawnAgent {
        task_id: String,
        agent_def_id: String,
        issue_number: u64,
    },
    /// Assign a reviewer to a PR.
    AssignReviewer {
        pr_number: u64,
        reviewer_agent_id: String,
    },
    /// Merge a PR (all approvals received).
    MergePr {
        pr_number: u64,
    },
    /// Notify an agent about a comment directed at them.
    NotifyAgent {
        agent_id: String,
        issue_number: u64,
        message: String,
    },
    /// Store feedback for retro engine.
    StoreFeedback {
        task_id: String,
        feedback: caloron_types::feedback::CaloronFeedback,
    },
    /// Task completed — close the issue and unblock dependents.
    TaskCompleted {
        task_id: String,
        issue_number: u64,
        pr_number: u64,
        unblocked: Vec<String>,
    },
    /// PR closed without merge — agent needs to rework.
    TaskRework {
        task_id: String,
        issue_number: u64,
        author_agent_id: String,
    },
    /// Notify supervisor about repeated PR closures.
    NotifySupervisor {
        reason: String,
    },
    /// Multiple actions from a single event.
    Multiple(Vec<OrchestratorAction>),
}

/// The event handler: translates Git events into DAG state transitions and orchestrator actions.
/// All handlers are idempotent — calling twice with the same event produces the same result.
pub struct EventHandler;

impl EventHandler {
    /// Handle a single Git event, updating DAG state and returning required actions.
    pub fn handle(event: &GitEvent, dag: &mut DagEngine) -> Result<OrchestratorAction> {
        match event {
            GitEvent::IssueOpened {
                number,
                title,
                labels: issue_labels,
                ..
            } => Self::handle_issue_opened(dag, *number, title, issue_labels),

            GitEvent::IssueLabeled { number, label, .. } => {
                Self::handle_issue_labeled(dag, *number, label)
            }

            GitEvent::IssueClosed { number, .. } => {
                Self::handle_issue_closed(dag, *number)
            }

            GitEvent::PrOpened {
                number,
                linked_issue,
                author,
                ..
            } => Self::handle_pr_opened(dag, *number, *linked_issue, author),

            GitEvent::PrReviewSubmitted {
                pr_number,
                state,
                ..
            } => Self::handle_pr_review(dag, *pr_number, state),

            GitEvent::PrMerged { number, .. } => {
                Self::handle_pr_merged(dag, *number)
            }

            GitEvent::PrClosed {
                number,
                linked_issue,
                ..
            } => Self::handle_pr_closed(dag, *number, *linked_issue),

            GitEvent::CommentCreated {
                issue_number,
                body,
                author,
                ..
            } => Self::handle_comment(dag, *issue_number, body, author),

            GitEvent::PushReceived { author, .. } => {
                Self::handle_push(dag, author)
            }
        }
    }

    fn handle_issue_opened(
        dag: &mut DagEngine,
        number: u64,
        _title: &str,
        issue_labels: &[String],
    ) -> Result<OrchestratorAction> {
        // Only process Caloron-managed issues
        if !issue_labels.iter().any(|l| l == labels::TASK) {
            return Ok(OrchestratorAction::None);
        }

        // Find a ready task that doesn't have an issue yet and start it
        let ready_tasks = dag.get_ready_tasks();
        for ts in ready_tasks {
            if ts.task.github_issue_number.is_none() {
                let task_id = ts.task.id.clone();
                let agent_id = ts.task.assigned_to.clone();
                dag.task_started(&task_id, number)?;

                return Ok(OrchestratorAction::SpawnAgent {
                    task_id,
                    agent_def_id: agent_id,
                    issue_number: number,
                });
            }
        }

        Ok(OrchestratorAction::None)
    }

    fn handle_issue_labeled(
        _dag: &mut DagEngine,
        _number: u64,
        _label: &str,
    ) -> Result<OrchestratorAction> {
        // Re-evaluate DAG assignment if label changes task type.
        // For now, no-op — task type changes are rare in practice.
        Ok(OrchestratorAction::None)
    }

    fn handle_issue_closed(
        _dag: &mut DagEngine,
        _number: u64,
    ) -> Result<OrchestratorAction> {
        // Issues are closed by the orchestrator as part of the completion chain (Addendum E2).
        // If closed externally, we don't re-process.
        Ok(OrchestratorAction::None)
    }

    fn handle_pr_opened(
        dag: &mut DagEngine,
        pr_number: u64,
        linked_issue: Option<u64>,
        _author: &str,
    ) -> Result<OrchestratorAction> {
        let Some(issue_number) = linked_issue else {
            return Ok(OrchestratorAction::None);
        };

        // Find the task by issue number
        let task_id = match dag.state().get_task_by_issue_number(issue_number) {
            Some(ts) => ts.task.id.clone(),
            None => return Ok(OrchestratorAction::None),
        };

        // Transition to InReview
        if let Err(e) = dag.task_in_review(&task_id, pr_number) {
            tracing::warn!(task_id, pr_number, error = %e, "Could not transition to InReview");
            return Ok(OrchestratorAction::None);
        }

        // Find reviewer
        let reviewer = dag
            .state()
            .get_reviewer_for_task(&task_id)
            .map(|a| a.id.clone());

        match reviewer {
            Some(reviewer_id) => Ok(OrchestratorAction::AssignReviewer {
                pr_number,
                reviewer_agent_id: reviewer_id,
            }),
            None => Ok(OrchestratorAction::None),
        }
    }

    fn handle_pr_review(
        dag: &mut DagEngine,
        pr_number: u64,
        state: &ReviewState,
    ) -> Result<OrchestratorAction> {
        match state {
            ReviewState::Approved => {
                // Check if this PR is linked to a task
                let task = dag
                    .state()
                    .tasks
                    .values()
                    .find(|ts| ts.pr_numbers.contains(&pr_number));

                if task.is_some() {
                    // Auto-merge if configured
                    Ok(OrchestratorAction::MergePr { pr_number })
                } else {
                    Ok(OrchestratorAction::None)
                }
            }
            ReviewState::ChangesRequested => {
                // Find the task and notify the author
                let task_info = dag
                    .state()
                    .tasks
                    .values()
                    .find(|ts| ts.pr_numbers.contains(&pr_number))
                    .map(|ts| (ts.task.id.clone(), ts.task.assigned_to.clone()));

                if let Some((_task_id, author_id)) = task_info {
                    Ok(OrchestratorAction::NotifyAgent {
                        agent_id: author_id,
                        issue_number: pr_number, // PR number for comment
                        message: "Changes requested on your PR. Please review the feedback.".into(),
                    })
                } else {
                    Ok(OrchestratorAction::None)
                }
            }
            ReviewState::Commented => Ok(OrchestratorAction::None),
        }
    }

    /// Handle PR merged — the canonical completion signal (Addendum E2).
    fn handle_pr_merged(
        dag: &mut DagEngine,
        pr_number: u64,
    ) -> Result<OrchestratorAction> {
        // Find the task linked to this PR
        let task_info = dag
            .state()
            .tasks
            .values()
            .find(|ts| ts.pr_numbers.contains(&pr_number))
            .map(|ts| {
                (
                    ts.task.id.clone(),
                    ts.task.github_issue_number.unwrap_or(0),
                )
            });

        let Some((task_id, issue_number)) = task_info else {
            return Ok(OrchestratorAction::None);
        };

        // Atomic completion chain:
        // 1. Transition DAG to Done
        // 2. Get newly unblocked tasks
        let unblocked = dag.task_completed(&task_id)?;

        Ok(OrchestratorAction::TaskCompleted {
            task_id,
            issue_number,
            pr_number,
            unblocked,
        })
    }

    /// Handle PR closed without merge (Addendum E3).
    fn handle_pr_closed(
        dag: &mut DagEngine,
        pr_number: u64,
        linked_issue: Option<u64>,
    ) -> Result<OrchestratorAction> {
        // Find the task
        let task_info = dag
            .state()
            .tasks
            .values()
            .find(|ts| ts.pr_numbers.contains(&pr_number))
            .map(|ts| {
                let issue_num = ts.task.github_issue_number.unwrap_or(0);
                let closures = ts.pr_numbers.len(); // how many PRs have been opened
                (ts.task.id.clone(), ts.task.assigned_to.clone(), issue_num, closures)
            });

        let Some((task_id, author_id, issue_number, pr_count)) = task_info else {
            return Ok(OrchestratorAction::None);
        };

        // Transition back to InProgress
        if let Err(e) = dag.task_pr_closed(&task_id) {
            tracing::warn!(task_id, error = %e, "Could not revert task from review");
            return Ok(OrchestratorAction::None);
        }

        // If this is the 2nd+ PR closure, notify supervisor
        if pr_count >= 2 {
            return Ok(OrchestratorAction::Multiple(vec![
                OrchestratorAction::TaskRework {
                    task_id: task_id.clone(),
                    issue_number,
                    author_agent_id: author_id,
                },
                OrchestratorAction::NotifySupervisor {
                    reason: format!(
                        "Task {task_id} has had {pr_count} PRs closed without merge"
                    ),
                },
            ]));
        }

        Ok(OrchestratorAction::TaskRework {
            task_id,
            issue_number,
            author_agent_id: author_id,
        })
    }

    fn handle_comment(
        dag: &mut DagEngine,
        issue_number: u64,
        body: &str,
        _author: &str,
    ) -> Result<OrchestratorAction> {
        // Check for feedback comment
        if let Some(feedback) = FeedbackComment::parse_from_comment(body) {
            let task_id = feedback.task_id.clone();
            return Ok(OrchestratorAction::StoreFeedback { task_id, feedback });
        }

        // Check for @caloron-agent-{id} mentions
        if let Some(agent_id) = extract_agent_mention(body) {
            return Ok(OrchestratorAction::NotifyAgent {
                agent_id,
                issue_number,
                message: body.to_string(),
            });
        }

        Ok(OrchestratorAction::None)
    }

    fn handle_push(
        _dag: &mut DagEngine,
        _author: &str,
    ) -> Result<OrchestratorAction> {
        // Push events reset the stall timer for the pushing agent.
        // This is handled at the daemon level, not via an orchestrator action.
        Ok(OrchestratorAction::None)
    }
}

/// Extract agent ID from @caloron-agent-{id} mention pattern (Addendum R1).
fn extract_agent_mention(body: &str) -> Option<String> {
    const PREFIX: &str = "@caloron-agent-";
    let idx = body.find(PREFIX)?;
    let start = idx + PREFIX.len();
    let rest = &body[start..];

    // Agent ID ends at whitespace or end of string
    let end = rest
        .find(|c: char| c.is_whitespace() || c == ',' || c == '.')
        .unwrap_or(rest.len());

    let id = &rest[..end];
    if id.is_empty() {
        None
    } else {
        Some(id.to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use caloron_types::dag::*;
    use chrono::Utc;

    fn make_dag() -> Dag {
        Dag {
            sprint: Sprint {
                id: "test".into(),
                goal: "Test".into(),
                start: Utc::now(),
                max_duration_hours: 24,
            },
            agents: vec![
                AgentNode { id: "dev-1".into(), role: "dev".into(), definition_path: "a.yaml".into() },
                AgentNode { id: "dev-2".into(), role: "dev".into(), definition_path: "a.yaml".into() },
                AgentNode { id: "rev-1".into(), role: "reviewer".into(), definition_path: "r.yaml".into() },
            ],
            tasks: vec![
                Task {
                    id: "t1".into(), title: "Task 1".into(), assigned_to: "dev-1".into(),
                    issue_template: "t.md".into(), depends_on: vec![],
                    reviewed_by: Some("rev-1".into()), github_issue_number: None,
                },
                Task {
                    id: "t2".into(), title: "Task 2".into(), assigned_to: "dev-2".into(),
                    issue_template: "t.md".into(), depends_on: vec!["t1".into()],
                    reviewed_by: Some("rev-1".into()), github_issue_number: None,
                },
            ],
            review_policy: ReviewPolicy { required_approvals: 1, auto_merge: true, max_review_cycles: 3 },
            escalation: EscalationConfig { stall_threshold_minutes: 20, supervisor_id: "sup".into(), human_contact: "github_issue".into() },
        }
    }

    fn make_engine() -> DagEngine {
        DagEngine::from_dag(make_dag()).unwrap()
    }

    #[test]
    fn test_issue_opened_spawns_agent() {
        let mut engine = make_engine();

        let event = GitEvent::IssueOpened {
            number: 10,
            title: "Task 1".into(),
            labels: vec![labels::TASK.into()],
            timestamp: Utc::now(),
        };

        let action = EventHandler::handle(&event, &mut engine).unwrap();

        match action {
            OrchestratorAction::SpawnAgent { task_id, agent_def_id, issue_number } => {
                assert_eq!(task_id, "t1");
                assert_eq!(agent_def_id, "dev-1");
                assert_eq!(issue_number, 10);
            }
            other => panic!("Expected SpawnAgent, got {other:?}"),
        }

        // Task should now be InProgress
        assert_eq!(engine.state().tasks["t1"].status, TaskStatus::InProgress);
    }

    #[test]
    fn test_issue_opened_ignores_non_caloron() {
        let mut engine = make_engine();

        let event = GitEvent::IssueOpened {
            number: 99,
            title: "Bug report".into(),
            labels: vec!["bug".into()],
            timestamp: Utc::now(),
        };

        let action = EventHandler::handle(&event, &mut engine).unwrap();
        assert_eq!(action, OrchestratorAction::None);
    }

    #[test]
    fn test_pr_opened_assigns_reviewer() {
        let mut engine = make_engine();
        engine.task_started("t1", 10).unwrap();

        let event = GitEvent::PrOpened {
            number: 100,
            title: "Implement task 1".into(),
            linked_issue: Some(10),
            author: "dev-1".into(),
            timestamp: Utc::now(),
        };

        let action = EventHandler::handle(&event, &mut engine).unwrap();
        assert_eq!(
            action,
            OrchestratorAction::AssignReviewer {
                pr_number: 100,
                reviewer_agent_id: "rev-1".into(),
            }
        );
        assert_eq!(engine.state().tasks["t1"].status, TaskStatus::InReview);
    }

    #[test]
    fn test_pr_review_approved_merges() {
        let mut engine = make_engine();
        engine.task_started("t1", 10).unwrap();
        engine.task_in_review("t1", 100).unwrap();

        let event = GitEvent::PrReviewSubmitted {
            pr_number: 100,
            reviewer: "rev-1".into(),
            state: ReviewState::Approved,
            body: "LGTM".into(),
            timestamp: Utc::now(),
        };

        let action = EventHandler::handle(&event, &mut engine).unwrap();
        assert_eq!(action, OrchestratorAction::MergePr { pr_number: 100 });
    }

    #[test]
    fn test_pr_merged_completes_task_and_unblocks() {
        let mut engine = make_engine();
        engine.task_started("t1", 10).unwrap();
        engine.task_in_review("t1", 100).unwrap();

        let event = GitEvent::PrMerged {
            number: 100,
            timestamp: Utc::now(),
        };

        let action = EventHandler::handle(&event, &mut engine).unwrap();

        match action {
            OrchestratorAction::TaskCompleted { task_id, unblocked, .. } => {
                assert_eq!(task_id, "t1");
                assert_eq!(unblocked, vec!["t2"]);
            }
            other => panic!("Expected TaskCompleted, got {other:?}"),
        }

        assert_eq!(engine.state().tasks["t1"].status, TaskStatus::Done);
        assert_eq!(engine.state().tasks["t2"].status, TaskStatus::Ready);
    }

    #[test]
    fn test_pr_closed_reverts_to_rework() {
        let mut engine = make_engine();
        engine.task_started("t1", 10).unwrap();
        engine.task_in_review("t1", 100).unwrap();

        let event = GitEvent::PrClosed {
            number: 100,
            closer: "rev-1".into(),
            linked_issue: Some(10),
            timestamp: Utc::now(),
        };

        let action = EventHandler::handle(&event, &mut engine).unwrap();

        match action {
            OrchestratorAction::TaskRework { task_id, author_agent_id, .. } => {
                assert_eq!(task_id, "t1");
                assert_eq!(author_agent_id, "dev-1");
            }
            other => panic!("Expected TaskRework, got {other:?}"),
        }

        assert_eq!(engine.state().tasks["t1"].status, TaskStatus::InProgress);
    }

    #[test]
    fn test_second_pr_closure_notifies_supervisor() {
        let mut engine = make_engine();
        engine.task_started("t1", 10).unwrap();
        engine.task_in_review("t1", 100).unwrap();

        // First closure
        let event = GitEvent::PrClosed {
            number: 100, closer: "rev-1".into(), linked_issue: Some(10), timestamp: Utc::now(),
        };
        EventHandler::handle(&event, &mut engine).unwrap();

        // Second PR opened and goes to review
        engine.task_in_review("t1", 101).unwrap();

        // Second closure
        let event = GitEvent::PrClosed {
            number: 101, closer: "rev-1".into(), linked_issue: Some(10), timestamp: Utc::now(),
        };
        let action = EventHandler::handle(&event, &mut engine).unwrap();

        match action {
            OrchestratorAction::Multiple(actions) => {
                assert!(actions.iter().any(|a| matches!(a, OrchestratorAction::NotifySupervisor { .. })));
                assert!(actions.iter().any(|a| matches!(a, OrchestratorAction::TaskRework { .. })));
            }
            other => panic!("Expected Multiple with supervisor notification, got {other:?}"),
        }
    }

    #[test]
    fn test_comment_with_feedback() {
        let mut engine = make_engine();

        let body = "---\ncaloron_feedback:\n  task_id: \"t1\"\n  agent_role: \"dev\"\n  task_clarity: 8\n  blockers: []\n  tools_used: [\"bash\"]\n  tokens_consumed: 5000\n  time_to_complete_min: 30\n  self_assessment: completed\n---";
        let event = GitEvent::CommentCreated {
            issue_number: 10,
            body: body.into(),
            author: "dev-1".into(),
            timestamp: Utc::now(),
        };

        let action = EventHandler::handle(&event, &mut engine).unwrap();
        assert!(matches!(action, OrchestratorAction::StoreFeedback { .. }));
    }

    #[test]
    fn test_comment_with_agent_mention() {
        let mut engine = make_engine();

        let event = GitEvent::CommentCreated {
            issue_number: 10,
            body: "Hey @caloron-agent-dev-1 please check this".into(),
            author: "human".into(),
            timestamp: Utc::now(),
        };

        let action = EventHandler::handle(&event, &mut engine).unwrap();
        match action {
            OrchestratorAction::NotifyAgent { agent_id, .. } => {
                assert_eq!(agent_id, "dev-1");
            }
            other => panic!("Expected NotifyAgent, got {other:?}"),
        }
    }

    #[test]
    fn test_extract_agent_mention() {
        assert_eq!(
            extract_agent_mention("Hello @caloron-agent-backend-1 please help"),
            Some("backend-1".into())
        );
        assert_eq!(
            extract_agent_mention("@caloron-agent-qa-1"),
            Some("qa-1".into())
        );
        assert_eq!(extract_agent_mention("No mention here"), None);
        assert_eq!(extract_agent_mention("@caloron-agent-"), None);
    }

    #[test]
    fn test_pr_changes_requested_notifies_author() {
        let mut engine = make_engine();
        engine.task_started("t1", 10).unwrap();
        engine.task_in_review("t1", 100).unwrap();

        let event = GitEvent::PrReviewSubmitted {
            pr_number: 100,
            reviewer: "rev-1".into(),
            state: ReviewState::ChangesRequested,
            body: "Please fix the error handling".into(),
            timestamp: Utc::now(),
        };

        let action = EventHandler::handle(&event, &mut engine).unwrap();
        match action {
            OrchestratorAction::NotifyAgent { agent_id, .. } => {
                assert_eq!(agent_id, "dev-1");
            }
            other => panic!("Expected NotifyAgent, got {other:?}"),
        }
    }

    #[test]
    fn test_push_event_is_noop() {
        let mut engine = make_engine();
        let event = GitEvent::PushReceived {
            branch: "agent/dev-1/test".into(),
            author: "dev-1".into(),
            commit_sha: "abc123".into(),
            timestamp: Utc::now(),
        };

        let action = EventHandler::handle(&event, &mut engine).unwrap();
        assert_eq!(action, OrchestratorAction::None);
    }
}
