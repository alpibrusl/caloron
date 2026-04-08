use std::collections::HashMap;
use std::path::PathBuf;

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

/// A sprint is the unit of work in Caloron. It has a start (kickoff),
/// an execution phase, and an end (retro). The DAG is fixed for its duration.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Sprint {
    pub id: String,
    pub goal: String,
    pub start: DateTime<Utc>,
    pub max_duration_hours: u32,
}

/// A node in the DAG representing an agent assignment.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentNode {
    pub id: String,
    pub role: String,
    pub definition_path: PathBuf,
}

/// A task in the sprint DAG.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Task {
    pub id: String,
    pub title: String,
    pub assigned_to: String,
    pub issue_template: PathBuf,
    pub depends_on: Vec<String>,
    pub reviewed_by: Option<String>,
    /// Set after the GitHub issue is created
    #[serde(default)]
    pub github_issue_number: Option<u64>,
}

/// Task status in the DAG state machine.
///
/// ```text
/// PENDING → READY → IN_PROGRESS → IN_REVIEW → DONE
///                                      ↕ (pr closed without merge)
///            at any point → BLOCKED → CANCELLED
///                                   → HUMAN_ASSIGNED
/// ```
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(tag = "status", rename_all = "snake_case")]
pub enum TaskStatus {
    Pending,
    Ready,
    InProgress,
    InReview,
    Done,
    Blocked { reason: String },
    Cancelled { reason: String },
    HumanAssigned,
}

/// Wraps a Task with its runtime state (Addendum R4).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TaskState {
    pub task: Task,
    pub status: TaskStatus,
    pub status_changed_at: DateTime<Utc>,
    pub intervention_count: u32,
    /// PRs opened for this task (may be >1 if reworked after pr.closed)
    #[serde(default)]
    pub pr_numbers: Vec<u64>,
}

impl TaskState {
    pub fn new(task: Task) -> Self {
        Self {
            task,
            status: TaskStatus::Pending,
            status_changed_at: Utc::now(),
            intervention_count: 0,
            pr_numbers: Vec::new(),
        }
    }

    /// Transition to a new status, updating the timestamp.
    pub fn transition(&mut self, new_status: TaskStatus) {
        self.status = new_status;
        self.status_changed_at = Utc::now();
    }
}

/// The full runtime state of a sprint's DAG.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DagState {
    pub sprint: Sprint,
    pub tasks: HashMap<String, TaskState>,
    pub agents: HashMap<String, AgentNode>,
    pub last_updated: DateTime<Utc>,
}

/// The review policy for a sprint.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReviewPolicy {
    pub required_approvals: u32,
    pub auto_merge: bool,
    pub max_review_cycles: u32,
}

/// Escalation configuration.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EscalationConfig {
    pub stall_threshold_minutes: u32,
    pub supervisor_id: String,
    pub human_contact: String,
}

/// The complete DAG definition as stored in dag.json.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Dag {
    pub sprint: Sprint,
    pub agents: Vec<AgentNode>,
    pub tasks: Vec<Task>,
    pub review_policy: ReviewPolicy,
    pub escalation: EscalationConfig,
}

impl DagState {
    /// Create a new DagState from a Dag definition.
    /// All tasks start as Pending.
    pub fn from_dag(dag: Dag) -> Self {
        let tasks = dag
            .tasks
            .into_iter()
            .map(|t| {
                let id = t.id.clone();
                (id, TaskState::new(t))
            })
            .collect();

        let agents = dag
            .agents
            .into_iter()
            .map(|a| (a.id.clone(), a))
            .collect();

        Self {
            sprint: dag.sprint,
            tasks,
            agents,
            last_updated: Utc::now(),
        }
    }

    /// Find all PENDING tasks whose dependencies are all DONE.
    pub fn evaluate_unblocked(&self) -> Vec<String> {
        self.tasks
            .iter()
            .filter(|(_, ts)| ts.status == TaskStatus::Pending)
            .filter(|(_, ts)| {
                ts.task.depends_on.iter().all(|dep_id| {
                    self.tasks
                        .get(dep_id)
                        .is_some_and(|dep| dep.status == TaskStatus::Done)
                })
            })
            .map(|(id, _)| id.clone())
            .collect()
    }

    /// Get a task by its linked GitHub issue number.
    pub fn get_task_by_issue_number(&self, number: u64) -> Option<&TaskState> {
        self.tasks
            .values()
            .find(|ts| ts.task.github_issue_number == Some(number))
    }

    /// Get a mutable task by its linked GitHub issue number.
    pub fn get_task_by_issue_number_mut(&mut self, number: u64) -> Option<&mut TaskState> {
        self.tasks
            .values_mut()
            .find(|ts| ts.task.github_issue_number == Some(number))
    }

    /// Get the reviewer agent node for a given task.
    pub fn get_reviewer_for_task(&self, task_id: &str) -> Option<&AgentNode> {
        self.tasks
            .get(task_id)
            .and_then(|ts| ts.task.reviewed_by.as_ref())
            .and_then(|reviewer_id| self.agents.get(reviewer_id))
    }

    /// Get all tasks in a given status.
    pub fn get_tasks_in_status(&self, status: &TaskStatus) -> Vec<&TaskState> {
        self.tasks
            .values()
            .filter(|ts| &ts.status == status)
            .collect()
    }

    /// Check if all tasks are in a terminal state (DONE, CANCELLED, HUMAN_ASSIGNED).
    pub fn is_sprint_complete(&self) -> bool {
        self.tasks.values().all(|ts| {
            matches!(
                ts.status,
                TaskStatus::Done | TaskStatus::Cancelled { .. } | TaskStatus::HumanAssigned
            )
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_test_dag() -> Dag {
        Dag {
            sprint: Sprint {
                id: "test-sprint".into(),
                goal: "Test sprint".into(),
                start: Utc::now(),
                max_duration_hours: 24,
            },
            agents: vec![
                AgentNode {
                    id: "backend-1".into(),
                    role: "backend-developer".into(),
                    definition_path: "agents/backend-developer.yaml".into(),
                },
                AgentNode {
                    id: "qa-1".into(),
                    role: "qa-engineer".into(),
                    definition_path: "agents/qa-engineer.yaml".into(),
                },
                AgentNode {
                    id: "reviewer-1".into(),
                    role: "senior-reviewer".into(),
                    definition_path: "agents/senior-reviewer.yaml".into(),
                },
            ],
            tasks: vec![
                Task {
                    id: "task-1".into(),
                    title: "Implement feature A".into(),
                    assigned_to: "backend-1".into(),
                    issue_template: "tasks/feature-a.md".into(),
                    depends_on: vec![],
                    reviewed_by: Some("reviewer-1".into()),
                    github_issue_number: None,
                },
                Task {
                    id: "task-2".into(),
                    title: "Implement feature B".into(),
                    assigned_to: "backend-1".into(),
                    issue_template: "tasks/feature-b.md".into(),
                    depends_on: vec![],
                    reviewed_by: Some("reviewer-1".into()),
                    github_issue_number: None,
                },
                Task {
                    id: "task-3".into(),
                    title: "Integration tests".into(),
                    assigned_to: "qa-1".into(),
                    issue_template: "tasks/integration-tests.md".into(),
                    depends_on: vec!["task-1".into(), "task-2".into()],
                    reviewed_by: Some("reviewer-1".into()),
                    github_issue_number: None,
                },
            ],
            review_policy: ReviewPolicy {
                required_approvals: 1,
                auto_merge: true,
                max_review_cycles: 3,
            },
            escalation: EscalationConfig {
                stall_threshold_minutes: 20,
                supervisor_id: "supervisor".into(),
                human_contact: "github_issue".into(),
            },
        }
    }

    #[test]
    fn test_initial_state_all_pending() {
        let state = DagState::from_dag(make_test_dag());
        assert!(state
            .tasks
            .values()
            .all(|ts| ts.status == TaskStatus::Pending));
    }

    #[test]
    fn test_evaluate_unblocked_no_deps() {
        let state = DagState::from_dag(make_test_dag());
        let mut unblocked = state.evaluate_unblocked();
        unblocked.sort();
        // task-1 and task-2 have no deps, task-3 depends on both
        assert_eq!(unblocked, vec!["task-1", "task-2"]);
    }

    #[test]
    fn test_evaluate_unblocked_partial_deps() {
        let mut state = DagState::from_dag(make_test_dag());
        // Complete task-1 only
        state
            .tasks
            .get_mut("task-1")
            .unwrap()
            .transition(TaskStatus::Done);

        let unblocked = state.evaluate_unblocked();
        // task-3 still blocked on task-2
        assert!(!unblocked.contains(&"task-3".to_string()));
    }

    #[test]
    fn test_evaluate_unblocked_all_deps_done() {
        let mut state = DagState::from_dag(make_test_dag());
        state
            .tasks
            .get_mut("task-1")
            .unwrap()
            .transition(TaskStatus::Done);
        state
            .tasks
            .get_mut("task-2")
            .unwrap()
            .transition(TaskStatus::Done);

        let unblocked = state.evaluate_unblocked();
        assert!(unblocked.contains(&"task-3".to_string()));
    }

    #[test]
    fn test_sprint_not_complete_with_pending() {
        let state = DagState::from_dag(make_test_dag());
        assert!(!state.is_sprint_complete());
    }

    #[test]
    fn test_sprint_complete_all_done() {
        let mut state = DagState::from_dag(make_test_dag());
        for ts in state.tasks.values_mut() {
            ts.transition(TaskStatus::Done);
        }
        assert!(state.is_sprint_complete());
    }

    #[test]
    fn test_sprint_complete_mixed_terminal() {
        let mut state = DagState::from_dag(make_test_dag());
        state
            .tasks
            .get_mut("task-1")
            .unwrap()
            .transition(TaskStatus::Done);
        state
            .tasks
            .get_mut("task-2")
            .unwrap()
            .transition(TaskStatus::Cancelled {
                reason: "sprint_cancelled".into(),
            });
        state
            .tasks
            .get_mut("task-3")
            .unwrap()
            .transition(TaskStatus::HumanAssigned);
        assert!(state.is_sprint_complete());
    }

    #[test]
    fn test_get_reviewer_for_task() {
        let state = DagState::from_dag(make_test_dag());
        let reviewer = state.get_reviewer_for_task("task-1").unwrap();
        assert_eq!(reviewer.id, "reviewer-1");
    }

    #[test]
    fn test_task_state_serialization_roundtrip() {
        let state = DagState::from_dag(make_test_dag());
        let json = serde_json::to_string_pretty(&state).unwrap();
        let deserialized: DagState = serde_json::from_str(&json).unwrap();
        assert_eq!(deserialized.tasks.len(), 3);
        assert_eq!(deserialized.sprint.id, "test-sprint");
    }
}
