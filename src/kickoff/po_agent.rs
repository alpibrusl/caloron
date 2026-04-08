use std::path::Path;

use anyhow::{Context, Result, bail};

use caloron_types::config::CaloronConfig;
use caloron_types::dag::Dag;

use crate::dag::engine::DagEngine;

/// The PO Agent system prompt template.
const PO_SYSTEM_PROMPT: &str = r#"You are the Product Owner agent for a software development sprint managed by Caloron.

Your responsibilities:
1. Analyze the current state of the project repository
2. Collaborate with the human operator to define clear sprint goals
3. Generate a DAG (Directed Acyclic Graph) that describes the tasks, agents, and dependencies
4. Create well-specified GitHub issues for each task

When creating tasks, always specify:
- Clear acceptance criteria
- Expected inputs and outputs
- Dependencies on other tasks
- Which agent role should handle it
- Estimated complexity (S/M/L)

When creating issues, always include:
- A "Definition of Done" section
- An "Error handling" section for tasks involving external APIs
- A "Dependencies" section linking to other issues

Your DAG must be valid JSON matching the Caloron dag format.
Never start the sprint until the human has explicitly approved the DAG.

Output the DAG as a JSON code block when ready for approval."#;

/// Summarize the current state of a repository for the PO Agent.
pub fn read_repository_state(repo_root: &Path) -> Result<RepositoryState> {
    let mut state = RepositoryState::default();

    // Get recent commits
    let output = std::process::Command::new("git")
        .args(["log", "--oneline", "-20"])
        .current_dir(repo_root)
        .output()
        .context("Failed to read git log")?;

    if output.status.success() {
        state.recent_commits = String::from_utf8_lossy(&output.stdout)
            .lines()
            .map(|l| l.to_string())
            .collect();
    }

    // Get file structure (top-level + one level deep)
    let output = std::process::Command::new("git")
        .args(["ls-tree", "--name-only", "-r", "HEAD"])
        .current_dir(repo_root)
        .output()
        .context("Failed to list files")?;

    if output.status.success() {
        state.file_tree = String::from_utf8_lossy(&output.stdout)
            .lines()
            .take(100) // Limit to avoid overwhelming the PO
            .map(|l| l.to_string())
            .collect();
    }

    // Get current branch
    let output = std::process::Command::new("git")
        .args(["branch", "--show-current"])
        .current_dir(repo_root)
        .output()
        .context("Failed to get current branch")?;

    if output.status.success() {
        state.current_branch = String::from_utf8_lossy(&output.stdout).trim().to_string();
    }

    Ok(state)
}

/// Repository state summary for PO context.
#[derive(Debug, Default)]
pub struct RepositoryState {
    pub current_branch: String,
    pub recent_commits: Vec<String>,
    pub file_tree: Vec<String>,
}

impl RepositoryState {
    /// Format as a context string for the PO Agent.
    pub fn to_context_string(&self) -> String {
        let mut ctx = String::new();

        ctx.push_str(&format!("Current branch: {}\n\n", self.current_branch));

        ctx.push_str("Recent commits:\n");
        for commit in &self.recent_commits {
            ctx.push_str(&format!("  {commit}\n"));
        }

        ctx.push_str("\nFile structure:\n");
        for file in &self.file_tree {
            ctx.push_str(&format!("  {file}\n"));
        }

        ctx
    }
}

/// Extract a DAG JSON block from PO Agent output.
/// Looks for ```json ... ``` code blocks containing a valid DAG.
pub fn extract_dag_from_output(output: &str) -> Option<Dag> {
    // Find JSON code blocks
    let mut in_block = false;
    let mut json_content = String::new();

    for line in output.lines() {
        if line.trim().starts_with("```json") {
            in_block = true;
            json_content.clear();
            continue;
        }
        if line.trim() == "```" && in_block {
            in_block = false;
            // Try to parse as DAG
            if let Ok(dag) = serde_json::from_str::<Dag>(&json_content) {
                return Some(dag);
            }
        }
        if in_block {
            json_content.push_str(line);
            json_content.push('\n');
        }
    }

    // Also try parsing raw JSON (no code fence)
    if let Ok(dag) = serde_json::from_str::<Dag>(output.trim()) {
        return Some(dag);
    }

    None
}

/// Validate a DAG and produce a human-readable summary.
pub fn summarize_dag(dag: &Dag) -> String {
    let mut summary = String::new();

    summary.push_str(&format!("Sprint: {}\n", dag.sprint.id));
    summary.push_str(&format!("Goal: {}\n", dag.sprint.goal));
    summary.push_str(&format!("Max duration: {} hours\n\n", dag.sprint.max_duration_hours));

    summary.push_str(&format!("Agents ({}):\n", dag.agents.len()));
    for agent in &dag.agents {
        summary.push_str(&format!("  {} ({})\n", agent.id, agent.role));
    }

    summary.push_str(&format!("\nTasks ({}):\n", dag.tasks.len()));
    for task in &dag.tasks {
        let deps = if task.depends_on.is_empty() {
            "(no deps)".to_string()
        } else {
            format!("depends on: {}", task.depends_on.join(", "))
        };
        let reviewer = task
            .reviewed_by
            .as_deref()
            .unwrap_or("(no reviewer)");
        summary.push_str(&format!(
            "  {} — {} [{}] → reviewed by {}\n    {}\n",
            task.id, task.title, task.assigned_to, reviewer, deps
        ));
    }

    // Find critical path (longest dependency chain)
    let max_depth = dag
        .tasks
        .iter()
        .map(|t| dep_depth(t.id.as_str(), &dag.tasks))
        .max()
        .unwrap_or(0);

    summary.push_str(&format!("\nCritical path depth: {max_depth} tasks\n"));
    summary.push_str(&format!(
        "Review policy: {} approvals, auto-merge: {}\n",
        dag.review_policy.required_approvals, dag.review_policy.auto_merge
    ));

    summary
}

fn dep_depth(task_id: &str, tasks: &[caloron_types::dag::Task]) -> usize {
    let task = match tasks.iter().find(|t| t.id == task_id) {
        Some(t) => t,
        None => return 0,
    };

    if task.depends_on.is_empty() {
        return 1;
    }

    1 + task
        .depends_on
        .iter()
        .map(|dep| dep_depth(dep, tasks))
        .max()
        .unwrap_or(0)
}

/// Run the kickoff flow: validate DAG, write to meta repo, and start daemon.
pub fn write_dag_to_file(dag: &Dag, path: &Path) -> Result<()> {
    let json = serde_json::to_string_pretty(dag)
        .context("Failed to serialize DAG")?;

    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }

    std::fs::write(path, &json)
        .with_context(|| format!("Failed to write DAG to {}", path.display()))?;

    tracing::info!(path = %path.display(), "DAG written");
    Ok(())
}

/// Get the PO Agent system prompt.
pub fn po_system_prompt() -> &'static str {
    PO_SYSTEM_PROMPT
}

#[cfg(test)]
mod tests {
    use super::*;
    use caloron_types::dag::*;
    use chrono::Utc;

    fn sample_dag_json() -> String {
        serde_json::to_string_pretty(&Dag {
            sprint: Sprint {
                id: "sprint-test".into(),
                goal: "Test goal".into(),
                start: Utc::now(),
                max_duration_hours: 24,
            },
            agents: vec![
                AgentNode { id: "dev-1".into(), role: "developer".into(), definition_path: "a.yaml".into() },
                AgentNode { id: "rev-1".into(), role: "reviewer".into(), definition_path: "r.yaml".into() },
            ],
            tasks: vec![
                Task {
                    id: "t1".into(), title: "Task 1".into(), assigned_to: "dev-1".into(),
                    issue_template: "t.md".into(), depends_on: vec![], reviewed_by: Some("rev-1".into()),
                    github_issue_number: None,
                },
                Task {
                    id: "t2".into(), title: "Task 2".into(), assigned_to: "dev-1".into(),
                    issue_template: "t.md".into(), depends_on: vec!["t1".into()],
                    reviewed_by: Some("rev-1".into()), github_issue_number: None,
                },
            ],
            review_policy: ReviewPolicy { required_approvals: 1, auto_merge: true, max_review_cycles: 3 },
            escalation: EscalationConfig {
                stall_threshold_minutes: 20, supervisor_id: "sup".into(), human_contact: "github_issue".into(),
            },
        })
        .unwrap()
    }

    #[test]
    fn test_extract_dag_from_code_block() {
        let output = format!(
            "Here is the DAG:\n\n```json\n{}\n```\n\nPlease approve.",
            sample_dag_json()
        );
        let dag = extract_dag_from_output(&output).unwrap();
        assert_eq!(dag.sprint.id, "sprint-test");
        assert_eq!(dag.tasks.len(), 2);
    }

    #[test]
    fn test_extract_dag_raw_json() {
        let json = sample_dag_json();
        let dag = extract_dag_from_output(&json).unwrap();
        assert_eq!(dag.sprint.id, "sprint-test");
    }

    #[test]
    fn test_extract_dag_no_dag() {
        assert!(extract_dag_from_output("No DAG here, just text.").is_none());
    }

    #[test]
    fn test_summarize_dag() {
        let dag: Dag = serde_json::from_str(&sample_dag_json()).unwrap();
        let summary = summarize_dag(&dag);

        assert!(summary.contains("sprint-test"));
        assert!(summary.contains("Test goal"));
        assert!(summary.contains("t1"));
        assert!(summary.contains("t2"));
        assert!(summary.contains("depends on: t1"));
        assert!(summary.contains("Critical path depth: 2"));
    }

    #[test]
    fn test_dep_depth() {
        let tasks = vec![
            Task {
                id: "a".into(), title: "A".into(), assigned_to: "x".into(),
                issue_template: "t.md".into(), depends_on: vec![], reviewed_by: None,
                github_issue_number: None,
            },
            Task {
                id: "b".into(), title: "B".into(), assigned_to: "x".into(),
                issue_template: "t.md".into(), depends_on: vec!["a".into()], reviewed_by: None,
                github_issue_number: None,
            },
            Task {
                id: "c".into(), title: "C".into(), assigned_to: "x".into(),
                issue_template: "t.md".into(), depends_on: vec!["b".into()], reviewed_by: None,
                github_issue_number: None,
            },
        ];

        assert_eq!(dep_depth("a", &tasks), 1);
        assert_eq!(dep_depth("b", &tasks), 2);
        assert_eq!(dep_depth("c", &tasks), 3);
    }

    #[test]
    fn test_write_dag_roundtrip() {
        let dag: Dag = serde_json::from_str(&sample_dag_json()).unwrap();
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("dag.json");

        write_dag_to_file(&dag, &path).unwrap();

        let engine = DagEngine::load_from_file(&path).unwrap();
        assert_eq!(engine.sprint_id(), "sprint-test");
    }

    #[test]
    fn test_repo_state_to_context() {
        let state = RepositoryState {
            current_branch: "main".into(),
            recent_commits: vec!["abc123 Initial commit".into()],
            file_tree: vec!["src/main.rs".into(), "Cargo.toml".into()],
        };

        let ctx = state.to_context_string();
        assert!(ctx.contains("main"));
        assert!(ctx.contains("Initial commit"));
        assert!(ctx.contains("src/main.rs"));
    }
}
