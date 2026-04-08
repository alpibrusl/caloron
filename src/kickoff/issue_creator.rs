use anyhow::{Context, Result};

use caloron_types::dag::{Dag, Task};
use caloron_types::git::labels;

use crate::git::GitHubClient;

/// Creates GitHub issues from a DAG's task definitions.
pub struct IssueCreator;

impl IssueCreator {
    /// Create all issues for a sprint DAG.
    /// Returns a map of task_id → issue_number.
    pub async fn create_all(
        client: &GitHubClient,
        dag: &Dag,
    ) -> Result<Vec<(String, u64)>> {
        let mut created = Vec::new();

        for task in &dag.tasks {
            let body = render_issue_body(task, dag);
            let issue_number = client
                .create_issue(
                    &task.title,
                    &body,
                    &[labels::TASK],
                )
                .await
                .with_context(|| format!("Failed to create issue for task {}", task.id))?;

            tracing::info!(
                task_id = task.id,
                issue_number,
                "Created issue"
            );

            created.push((task.id.clone(), issue_number));
        }

        Ok(created)
    }
}

/// Render the issue body from a task definition.
fn render_issue_body(task: &Task, dag: &Dag) -> String {
    let mut body = String::new();

    body.push_str(&format!("## {}\n\n", task.title));

    // Sprint context
    body.push_str(&format!(
        "**Sprint:** {}\n**Assigned to:** @caloron-agent-{}\n",
        dag.sprint.id, task.assigned_to
    ));

    if let Some(reviewer) = &task.reviewed_by {
        body.push_str(&format!("**Reviewer:** @caloron-agent-{reviewer}\n"));
    }

    body.push('\n');

    // Dependencies
    if !task.depends_on.is_empty() {
        body.push_str("## Dependencies\n\n");
        for dep_id in &task.depends_on {
            if let Some(dep_task) = dag.tasks.iter().find(|t| t.id == *dep_id) {
                body.push_str(&format!("- [ ] {dep_id}: {}\n", dep_task.title));
            } else {
                body.push_str(&format!("- [ ] {dep_id}\n"));
            }
        }
        body.push('\n');
    }

    // Definition of done
    body.push_str("## Definition of Done\n\n");
    body.push_str("- [ ] Implementation complete\n");
    body.push_str("- [ ] Tests written and passing\n");
    body.push_str("- [ ] PR opened with clear description\n");
    body.push_str("- [ ] Feedback comment posted\n");
    body.push('\n');

    // Metadata
    body.push_str("---\n");
    body.push_str(&format!(
        "*Task ID: {} | Template: {} | Managed by Caloron*\n",
        task.id,
        task.issue_template.display()
    ));

    body
}

#[cfg(test)]
mod tests {
    use super::*;
    use caloron_types::dag::*;
    use chrono::Utc;

    fn sample_dag() -> Dag {
        Dag {
            sprint: Sprint {
                id: "sprint-test".into(),
                goal: "Test".into(),
                start: Utc::now(),
                max_duration_hours: 24,
            },
            agents: vec![
                AgentNode { id: "dev-1".into(), role: "dev".into(), definition_path: "a.yaml".into() },
                AgentNode { id: "rev-1".into(), role: "rev".into(), definition_path: "r.yaml".into() },
            ],
            tasks: vec![
                Task {
                    id: "t1".into(), title: "Implement feature A".into(), assigned_to: "dev-1".into(),
                    issue_template: "tasks/feature.md".into(), depends_on: vec![],
                    reviewed_by: Some("rev-1".into()), github_issue_number: None,
                },
                Task {
                    id: "t2".into(), title: "Test feature A".into(), assigned_to: "dev-1".into(),
                    issue_template: "tasks/test.md".into(), depends_on: vec!["t1".into()],
                    reviewed_by: Some("rev-1".into()), github_issue_number: None,
                },
            ],
            review_policy: ReviewPolicy { required_approvals: 1, auto_merge: true, max_review_cycles: 3 },
            escalation: EscalationConfig { stall_threshold_minutes: 20, supervisor_id: "sup".into(), human_contact: "gh".into() },
        }
    }

    #[test]
    fn test_render_issue_body_basic() {
        let dag = sample_dag();
        let body = render_issue_body(&dag.tasks[0], &dag);

        assert!(body.contains("Implement feature A"));
        assert!(body.contains("@caloron-agent-dev-1"));
        assert!(body.contains("@caloron-agent-rev-1"));
        assert!(body.contains("Definition of Done"));
        assert!(body.contains("Task ID: t1"));
    }

    #[test]
    fn test_render_issue_body_with_deps() {
        let dag = sample_dag();
        let body = render_issue_body(&dag.tasks[1], &dag);

        assert!(body.contains("Dependencies"));
        assert!(body.contains("t1: Implement feature A"));
    }

    #[test]
    fn test_render_issue_body_no_deps() {
        let dag = sample_dag();
        let body = render_issue_body(&dag.tasks[0], &dag);

        // t1 has no deps, so no Dependencies section
        assert!(!body.contains("Dependencies"));
    }
}
