use std::collections::{HashMap, HashSet};
use std::path::Path;

use anyhow::{Context, Result, bail};

use caloron_types::dag::{Dag, DagState, TaskState, TaskStatus};

/// The DAG engine: loads, validates, and manages the sprint DAG at runtime.
#[derive(Debug)]
pub struct DagEngine {
    state: DagState,
    state_file: Option<String>,
}

/// Errors specific to DAG validation.
#[derive(Debug, thiserror::Error)]
pub enum DagValidationError {
    #[error("DAG contains a cycle involving task: {0}")]
    Cycle(String),
    #[error("Task '{task_id}' references unknown agent: {agent_id}")]
    UnknownAgent {
        task_id: String,
        agent_id: String,
    },
    #[error("Task '{task_id}' depends on unknown task: {dep_id}")]
    UnknownDependency {
        task_id: String,
        dep_id: String,
    },
    #[error("Task '{task_id}' references unknown reviewer: {reviewer_id}")]
    UnknownReviewer {
        task_id: String,
        reviewer_id: String,
    },
    #[error("Duplicate task ID: {0}")]
    DuplicateTaskId(String),
    #[error("Duplicate agent ID: {0}")]
    DuplicateAgentId(String),
}

impl DagEngine {
    /// Load a DAG from a JSON file and validate it.
    pub fn load_from_file(path: &Path) -> Result<Self> {
        let content =
            std::fs::read_to_string(path).with_context(|| format!("Failed to read {}", path.display()))?;

        let dag: Dag =
            serde_json::from_str(&content).with_context(|| format!("Failed to parse {}", path.display()))?;

        Self::from_dag(dag)
    }

    /// Create a DagEngine from a parsed Dag definition.
    pub fn from_dag(dag: Dag) -> Result<Self> {
        validate_dag(&dag)?;

        let state = DagState::from_dag(dag);

        // Transition tasks with no dependencies directly to Ready
        let mut engine = Self {
            state,
            state_file: None,
        };
        engine.initialize_ready_tasks();

        Ok(engine)
    }

    /// Resume from a persisted state file.
    pub fn resume_from_file(path: &Path) -> Result<Self> {
        let content =
            std::fs::read_to_string(path).with_context(|| format!("Failed to read {}", path.display()))?;

        let state: DagState = serde_json::from_str(&content)
            .with_context(|| format!("Failed to parse state {}", path.display()))?;

        tracing::info!(
            sprint_id = state.sprint.id,
            tasks = state.tasks.len(),
            "Resumed DAG state from file"
        );

        Ok(Self {
            state,
            state_file: Some(path.to_string_lossy().to_string()),
        })
    }

    /// Set the path where state will be persisted after each transition.
    pub fn set_state_file(&mut self, path: &str) {
        self.state_file = Some(path.to_string());
    }

    // -- State transitions --

    /// Mark a task as in-progress (agent has started working).
    pub fn task_started(&mut self, task_id: &str, issue_number: u64) -> Result<()> {
        let ts = self.get_task_mut(task_id)?;
        if ts.status != TaskStatus::Ready {
            bail!(
                "Cannot start task '{task_id}': current status is {:?}, expected Ready",
                ts.status
            );
        }
        ts.task.github_issue_number = Some(issue_number);
        ts.transition(TaskStatus::InProgress);
        self.persist()?;
        Ok(())
    }

    /// Mark a task as in-review (PR opened).
    pub fn task_in_review(&mut self, task_id: &str, pr_number: u64) -> Result<()> {
        let ts = self.get_task_mut(task_id)?;
        if ts.status != TaskStatus::InProgress {
            bail!(
                "Cannot move task '{task_id}' to review: current status is {:?}",
                ts.status
            );
        }
        ts.pr_numbers.push(pr_number);
        ts.transition(TaskStatus::InReview);
        self.persist()?;
        Ok(())
    }

    /// PR closed without merge — task goes back to InProgress (Addendum E3).
    pub fn task_pr_closed(&mut self, task_id: &str) -> Result<()> {
        let ts = self.get_task_mut(task_id)?;
        if ts.status != TaskStatus::InReview {
            bail!(
                "Cannot revert task '{task_id}' from review: current status is {:?}",
                ts.status
            );
        }
        ts.transition(TaskStatus::InProgress);
        self.persist()?;
        Ok(())
    }

    /// Complete a task (PR merged). Returns newly unblocked task IDs (Addendum E2).
    pub fn task_completed(&mut self, task_id: &str) -> Result<Vec<String>> {
        let ts = self.get_task_mut(task_id)?;
        if !matches!(ts.status, TaskStatus::InReview | TaskStatus::InProgress) {
            bail!(
                "Cannot complete task '{task_id}': current status is {:?}",
                ts.status
            );
        }
        ts.transition(TaskStatus::Done);

        // Evaluate which tasks are now unblocked
        let unblocked = self.state.evaluate_unblocked();

        // Transition unblocked tasks to Ready
        for id in &unblocked {
            if let Some(ts) = self.state.tasks.get_mut(id) {
                ts.transition(TaskStatus::Ready);
            }
        }

        self.persist()?;

        Ok(unblocked)
    }

    /// Block a task (supervisor intervention).
    pub fn task_blocked(&mut self, task_id: &str, reason: &str) -> Result<()> {
        let ts = self.get_task_mut(task_id)?;
        ts.intervention_count += 1;
        ts.transition(TaskStatus::Blocked {
            reason: reason.to_string(),
        });
        self.persist()?;
        Ok(())
    }

    /// Cancel a task.
    pub fn task_cancelled(&mut self, task_id: &str, reason: &str) -> Result<()> {
        let ts = self.get_task_mut(task_id)?;
        ts.transition(TaskStatus::Cancelled {
            reason: reason.to_string(),
        });
        self.persist()?;
        Ok(())
    }

    /// Mark task as taken over by human.
    pub fn task_human_assigned(&mut self, task_id: &str) -> Result<()> {
        let ts = self.get_task_mut(task_id)?;
        ts.transition(TaskStatus::HumanAssigned);
        self.persist()?;
        Ok(())
    }

    /// Cancel all non-terminal tasks (sprint cancellation, Addendum H3).
    pub fn cancel_sprint(&mut self) -> Result<Vec<String>> {
        let mut cancelled = Vec::new();

        let task_ids: Vec<String> = self.state.tasks.keys().cloned().collect();
        for id in task_ids {
            let ts = self.state.tasks.get(&id).unwrap();
            if !matches!(
                ts.status,
                TaskStatus::Done | TaskStatus::Cancelled { .. } | TaskStatus::HumanAssigned
            ) {
                let ts = self.state.tasks.get_mut(&id).unwrap();
                ts.transition(TaskStatus::Cancelled {
                    reason: "sprint_cancelled".to_string(),
                });
                cancelled.push(id);
            }
        }

        self.persist()?;
        Ok(cancelled)
    }

    // -- Queries --

    pub fn state(&self) -> &DagState {
        &self.state
    }

    pub fn is_sprint_complete(&self) -> bool {
        self.state.is_sprint_complete()
    }

    pub fn sprint_id(&self) -> &str {
        &self.state.sprint.id
    }

    pub fn get_ready_tasks(&self) -> Vec<&TaskState> {
        self.state.get_tasks_in_status(&TaskStatus::Ready)
    }

    // -- Internal --

    fn get_task_mut(&mut self, task_id: &str) -> Result<&mut TaskState> {
        self.state
            .tasks
            .get_mut(task_id)
            .with_context(|| format!("Task '{task_id}' not found in DAG"))
    }

    /// Transition tasks with no dependencies to Ready.
    fn initialize_ready_tasks(&mut self) {
        let unblocked = self.state.evaluate_unblocked();
        for id in unblocked {
            if let Some(ts) = self.state.tasks.get_mut(&id) {
                ts.transition(TaskStatus::Ready);
            }
        }
    }

    /// Persist current state to file.
    fn persist(&self) -> Result<()> {
        if let Some(path) = &self.state_file {
            let json = serde_json::to_string_pretty(&self.state)
                .context("Failed to serialize DAG state")?;

            if let Some(parent) = Path::new(path).parent() {
                std::fs::create_dir_all(parent)?;
            }

            std::fs::write(path, json)
                .with_context(|| format!("Failed to persist state to {path}"))?;

            tracing::debug!(path, "DAG state persisted");
        }
        Ok(())
    }
}

/// Validate a DAG definition for structural correctness.
fn validate_dag(dag: &Dag) -> Result<()> {
    // Check for duplicate agent IDs
    let mut agent_ids = HashSet::new();
    for agent in &dag.agents {
        if !agent_ids.insert(&agent.id) {
            bail!(DagValidationError::DuplicateAgentId(agent.id.clone()));
        }
    }

    // Check for duplicate task IDs
    let mut task_ids = HashSet::new();
    for task in &dag.tasks {
        if !task_ids.insert(&task.id) {
            bail!(DagValidationError::DuplicateTaskId(task.id.clone()));
        }
    }

    // Validate task references
    for task in &dag.tasks {
        // Check assigned agent exists
        if !agent_ids.contains(&task.assigned_to) {
            bail!(DagValidationError::UnknownAgent {
                task_id: task.id.clone(),
                agent_id: task.assigned_to.clone(),
            });
        }

        // Check reviewer exists
        if let Some(reviewer) = &task.reviewed_by {
            if !agent_ids.contains(reviewer) {
                bail!(DagValidationError::UnknownReviewer {
                    task_id: task.id.clone(),
                    reviewer_id: reviewer.clone(),
                });
            }
        }

        // Check dependencies exist
        for dep in &task.depends_on {
            if !task_ids.contains(dep) {
                bail!(DagValidationError::UnknownDependency {
                    task_id: task.id.clone(),
                    dep_id: dep.clone(),
                });
            }
        }
    }

    // Check for cycles using DFS
    detect_cycles(dag)?;

    Ok(())
}

/// Detect cycles in the DAG using topological sort (Kahn's algorithm).
fn detect_cycles(dag: &Dag) -> Result<()> {
    let mut in_degree: HashMap<&str, usize> = HashMap::new();
    let mut adj: HashMap<&str, Vec<&str>> = HashMap::new();

    for task in &dag.tasks {
        in_degree.entry(task.id.as_str()).or_insert(0);
        adj.entry(task.id.as_str()).or_default();
        for dep in &task.depends_on {
            adj.entry(dep.as_str()).or_default().push(task.id.as_str());
            *in_degree.entry(task.id.as_str()).or_insert(0) += 1;
        }
    }

    let mut queue: Vec<&str> = in_degree
        .iter()
        .filter(|(_, deg)| **deg == 0)
        .map(|(id, _)| *id)
        .collect();

    let mut visited = 0;

    while let Some(node) = queue.pop() {
        visited += 1;
        if let Some(neighbors) = adj.get(node) {
            for &neighbor in neighbors {
                let deg = in_degree.get_mut(neighbor).unwrap();
                *deg -= 1;
                if *deg == 0 {
                    queue.push(neighbor);
                }
            }
        }
    }

    if visited != dag.tasks.len() {
        // Find a task involved in the cycle
        let cycle_task = in_degree
            .iter()
            .find(|(_, deg)| **deg > 0)
            .map(|(id, _)| id.to_string())
            .unwrap_or_default();
        bail!(DagValidationError::Cycle(cycle_task));
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use caloron_types::dag::*;
    use chrono::Utc;

    fn make_dag() -> Dag {
        Dag {
            sprint: Sprint {
                id: "test-sprint".into(),
                goal: "Test".into(),
                start: Utc::now(),
                max_duration_hours: 24,
            },
            agents: vec![
                AgentNode { id: "a1".into(), role: "dev".into(), definition_path: "a.yaml".into() },
                AgentNode { id: "a2".into(), role: "dev".into(), definition_path: "a.yaml".into() },
                AgentNode { id: "r1".into(), role: "reviewer".into(), definition_path: "r.yaml".into() },
            ],
            tasks: vec![
                Task {
                    id: "t1".into(), title: "Task 1".into(), assigned_to: "a1".into(),
                    issue_template: "t.md".into(), depends_on: vec![], reviewed_by: Some("r1".into()),
                    github_issue_number: None,
                },
                Task {
                    id: "t2".into(), title: "Task 2".into(), assigned_to: "a2".into(),
                    issue_template: "t.md".into(), depends_on: vec![], reviewed_by: Some("r1".into()),
                    github_issue_number: None,
                },
                Task {
                    id: "t3".into(), title: "Task 3".into(), assigned_to: "a1".into(),
                    issue_template: "t.md".into(), depends_on: vec!["t1".into(), "t2".into()],
                    reviewed_by: Some("r1".into()), github_issue_number: None,
                },
            ],
            review_policy: ReviewPolicy { required_approvals: 1, auto_merge: true, max_review_cycles: 3 },
            escalation: EscalationConfig { stall_threshold_minutes: 20, supervisor_id: "sup".into(), human_contact: "github_issue".into() },
        }
    }

    #[test]
    fn test_load_and_initialize_ready_tasks() {
        let engine = DagEngine::from_dag(make_dag()).unwrap();
        // t1 and t2 have no deps → Ready, t3 depends on both → Pending
        assert_eq!(engine.state.tasks["t1"].status, TaskStatus::Ready);
        assert_eq!(engine.state.tasks["t2"].status, TaskStatus::Ready);
        assert_eq!(engine.state.tasks["t3"].status, TaskStatus::Pending);
    }

    #[test]
    fn test_full_task_lifecycle() {
        let mut engine = DagEngine::from_dag(make_dag()).unwrap();

        // Start t1
        engine.task_started("t1", 10).unwrap();
        assert_eq!(engine.state.tasks["t1"].status, TaskStatus::InProgress);
        assert_eq!(engine.state.tasks["t1"].task.github_issue_number, Some(10));

        // PR opened for t1
        engine.task_in_review("t1", 100).unwrap();
        assert_eq!(engine.state.tasks["t1"].status, TaskStatus::InReview);
        assert_eq!(engine.state.tasks["t1"].pr_numbers, vec![100]);

        // PR merged for t1
        let unblocked = engine.task_completed("t1").unwrap();
        assert_eq!(engine.state.tasks["t1"].status, TaskStatus::Done);
        // t3 still blocked on t2
        assert!(unblocked.is_empty());

        // Complete t2
        engine.task_started("t2", 11).unwrap();
        engine.task_in_review("t2", 101).unwrap();
        let unblocked = engine.task_completed("t2").unwrap();
        // Now t3 should be unblocked
        assert_eq!(unblocked, vec!["t3"]);
        assert_eq!(engine.state.tasks["t3"].status, TaskStatus::Ready);
    }

    #[test]
    fn test_pr_closed_reverts_to_in_progress() {
        let mut engine = DagEngine::from_dag(make_dag()).unwrap();
        engine.task_started("t1", 10).unwrap();
        engine.task_in_review("t1", 100).unwrap();

        engine.task_pr_closed("t1").unwrap();
        assert_eq!(engine.state.tasks["t1"].status, TaskStatus::InProgress);
    }

    #[test]
    fn test_sprint_cancellation() {
        let mut engine = DagEngine::from_dag(make_dag()).unwrap();
        engine.task_started("t1", 10).unwrap();

        let cancelled = engine.cancel_sprint().unwrap();
        // t1 (InProgress), t2 (Ready), t3 (Pending) all cancelled
        assert_eq!(cancelled.len(), 3);

        for ts in engine.state.tasks.values() {
            assert!(matches!(ts.status, TaskStatus::Cancelled { .. }));
        }
    }

    #[test]
    fn test_sprint_cancellation_preserves_done() {
        let mut engine = DagEngine::from_dag(make_dag()).unwrap();
        engine.task_started("t1", 10).unwrap();
        engine.task_in_review("t1", 100).unwrap();
        engine.task_completed("t1").unwrap();

        let cancelled = engine.cancel_sprint().unwrap();
        // Only t2 and t3 cancelled, t1 stays Done
        assert_eq!(cancelled.len(), 2);
        assert_eq!(engine.state.tasks["t1"].status, TaskStatus::Done);
    }

    #[test]
    fn test_is_sprint_complete() {
        let mut engine = DagEngine::from_dag(make_dag()).unwrap();
        assert!(!engine.is_sprint_complete());

        // Complete all tasks
        engine.task_started("t1", 10).unwrap();
        engine.task_in_review("t1", 100).unwrap();
        engine.task_completed("t1").unwrap();

        engine.task_started("t2", 11).unwrap();
        engine.task_in_review("t2", 101).unwrap();
        engine.task_completed("t2").unwrap();

        engine.task_started("t3", 12).unwrap();
        engine.task_in_review("t3", 102).unwrap();
        engine.task_completed("t3").unwrap();

        assert!(engine.is_sprint_complete());
    }

    #[test]
    fn test_invalid_transition_errors() {
        let mut engine = DagEngine::from_dag(make_dag()).unwrap();

        // Can't complete a Ready task
        assert!(engine.task_completed("t1").is_err());
        // Can't put a Ready task in review
        assert!(engine.task_in_review("t1", 100).is_err());
        // Can't start a Pending task
        assert!(engine.task_started("t3", 10).is_err());
    }

    #[test]
    fn test_state_persistence_roundtrip() {
        let dir = tempfile::tempdir().unwrap();
        let state_path = dir.path().join("state.json");

        let mut engine = DagEngine::from_dag(make_dag()).unwrap();
        engine.set_state_file(state_path.to_str().unwrap());

        engine.task_started("t1", 10).unwrap();
        engine.task_in_review("t1", 100).unwrap();

        // Resume from persisted state
        let resumed = DagEngine::resume_from_file(&state_path).unwrap();
        assert_eq!(resumed.state.tasks["t1"].status, TaskStatus::InReview);
        assert_eq!(resumed.state.tasks["t2"].status, TaskStatus::Ready);
    }

    #[test]
    fn test_validate_cyclic_dag() {
        let mut dag = make_dag();
        // Add a cycle: t1 depends on t3, t3 depends on t1
        dag.tasks[0].depends_on = vec!["t3".into()];

        let result = DagEngine::from_dag(dag);
        assert!(result.is_err());
        let err = result.unwrap_err().to_string();
        assert!(err.contains("cycle"), "Error should mention cycle: {err}");
    }

    #[test]
    fn test_validate_unknown_agent() {
        let mut dag = make_dag();
        dag.tasks[0].assigned_to = "nonexistent".into();

        let result = DagEngine::from_dag(dag);
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("unknown agent"));
    }

    #[test]
    fn test_validate_unknown_dependency() {
        let mut dag = make_dag();
        dag.tasks[0].depends_on = vec!["nonexistent".into()];

        let result = DagEngine::from_dag(dag);
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("unknown task"));
    }

    #[test]
    fn test_validate_duplicate_task_id() {
        let mut dag = make_dag();
        dag.tasks[1].id = "t1".into(); // duplicate

        let result = DagEngine::from_dag(dag);
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("Duplicate task"));
    }

    #[test]
    fn test_load_from_file() {
        let dir = tempfile::tempdir().unwrap();
        let dag_path = dir.path().join("dag.json");

        let dag = make_dag();
        let json = serde_json::to_string_pretty(&dag).unwrap();
        std::fs::write(&dag_path, json).unwrap();

        let engine = DagEngine::load_from_file(&dag_path).unwrap();
        assert_eq!(engine.sprint_id(), "test-sprint");
        assert_eq!(engine.state.tasks.len(), 3);
    }

    #[test]
    fn test_block_and_cancel_task() {
        let mut engine = DagEngine::from_dag(make_dag()).unwrap();
        engine.task_started("t1", 10).unwrap();

        engine.task_blocked("t1", "credential failure").unwrap();
        assert!(matches!(
            engine.state.tasks["t1"].status,
            TaskStatus::Blocked { .. }
        ));
        assert_eq!(engine.state.tasks["t1"].intervention_count, 1);

        engine.task_cancelled("t1", "not recoverable").unwrap();
        assert!(matches!(
            engine.state.tasks["t1"].status,
            TaskStatus::Cancelled { .. }
        ));
    }

    #[test]
    fn test_human_assigned() {
        let mut engine = DagEngine::from_dag(make_dag()).unwrap();
        engine.task_started("t1", 10).unwrap();
        engine.task_human_assigned("t1").unwrap();
        assert_eq!(engine.state.tasks["t1"].status, TaskStatus::HumanAssigned);
        // HumanAssigned is terminal
        assert!(!engine.is_sprint_complete()); // t2 and t3 still pending/ready
    }
}
