use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use chrono::Utc;

use caloron_types::dag::{DagState, TaskStatus};
use caloron_types::dashboard::{
    ProjectRegistry, RegisteredProject, SprintStatus, SprintSummary,
};

/// Path to the global project registry.
fn registry_path() -> PathBuf {
    dirs_path().join("projects.json")
}

fn dirs_path() -> PathBuf {
    let home = std::env::var("HOME").unwrap_or_else(|_| "/tmp".into());
    PathBuf::from(home).join(".caloron")
}

/// Load the global project registry.
pub fn load_registry() -> Result<ProjectRegistry> {
    let path = registry_path();
    if !path.exists() {
        return Ok(ProjectRegistry::default());
    }
    let content = std::fs::read_to_string(&path)
        .with_context(|| format!("Failed to read {}", path.display()))?;
    let reg: ProjectRegistry = serde_json::from_str(&content)?;
    Ok(reg)
}

/// Save the global project registry.
pub fn save_registry(reg: &ProjectRegistry) -> Result<()> {
    let path = registry_path();
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let json = serde_json::to_string_pretty(reg)?;
    std::fs::write(&path, json)?;
    Ok(())
}

/// Register the current project in the global registry.
pub fn register_current_project(name: &str, repo: &str, project_path: &Path) -> Result<()> {
    let mut reg = load_registry()?;
    reg.register(RegisteredProject {
        name: name.into(),
        repo: repo.into(),
        path: project_path.to_path_buf(),
        registered_at: Utc::now(),
        last_sprint: None,
    });
    save_registry(&reg)
}

/// Update the sprint summary for the current project.
pub fn update_sprint_summary(project_path: &Path, dag: &DagState) -> Result<()> {
    let mut reg = load_registry()?;

    let summary = build_sprint_summary(dag);
    reg.update_sprint(&project_path.to_path_buf(), summary);

    save_registry(&reg)
}

/// Build a SprintSummary from current DAG state.
pub fn build_sprint_summary(dag: &DagState) -> SprintSummary {
    let tasks_total = dag.tasks.len();
    let tasks_done = dag
        .tasks
        .values()
        .filter(|t| t.status == TaskStatus::Done)
        .count();
    let tasks_in_progress = dag
        .tasks
        .values()
        .filter(|t| {
            matches!(
                t.status,
                TaskStatus::InProgress | TaskStatus::InReview
            )
        })
        .count();
    let tasks_blocked = dag
        .tasks
        .values()
        .filter(|t| matches!(t.status, TaskStatus::Blocked { .. }))
        .count();
    let total_interventions: u32 = dag.tasks.values().map(|t| t.intervention_count).sum();

    let is_complete = dag.is_sprint_complete();
    let has_cancelled = dag
        .tasks
        .values()
        .any(|t| matches!(t.status, TaskStatus::Cancelled { .. }));

    let status = if is_complete && has_cancelled {
        SprintStatus::Cancelled
    } else if is_complete {
        SprintStatus::Completed
    } else {
        SprintStatus::Active
    };

    SprintSummary {
        id: dag.sprint.id.clone(),
        goal: dag.sprint.goal.clone(),
        status,
        started_at: dag.sprint.start,
        tasks_total,
        tasks_done,
        tasks_in_progress,
        tasks_blocked,
        agents_running: tasks_in_progress, // approximate
        total_interventions,
        updated_at: Utc::now(),
    }
}

/// Print the current project's status in rich format.
pub fn print_project_status(dag: &DagState) {
    let summary = build_sprint_summary(dag);

    println!("Sprint: {}", summary.id);
    println!("Goal:   {}", summary.goal);
    println!("Status: {:?}", summary.status);
    println!();

    // Progress bar
    let pct = if summary.tasks_total > 0 {
        summary.tasks_done as f64 / summary.tasks_total as f64
    } else {
        0.0
    };
    let filled = (pct * 20.0) as usize;
    let empty = 20 - filled;
    println!(
        "Progress: [{}{}] {}/{} ({:.0}%)",
        "#".repeat(filled),
        "-".repeat(empty),
        summary.tasks_done,
        summary.tasks_total,
        pct * 100.0
    );
    println!();

    // Task table
    println!("Tasks:");
    let mut tasks: Vec<_> = dag.tasks.iter().collect();
    tasks.sort_by_key(|(id, _)| id.clone());

    for (id, ts) in &tasks {
        let (icon, status_str) = match &ts.status {
            TaskStatus::Pending => (" ", "PENDING"),
            TaskStatus::Ready => (" ", "READY"),
            TaskStatus::InProgress => (">", "WORKING"),
            TaskStatus::InReview => (">", "IN REVIEW"),
            TaskStatus::Done => ("v", "DONE"),
            TaskStatus::Blocked { .. } => ("!", "BLOCKED"),
            TaskStatus::Cancelled { .. } => ("x", "CANCELLED"),
            TaskStatus::HumanAssigned => ("H", "HUMAN"),
        };

        let agent = &ts.task.assigned_to;
        let prs = if ts.pr_numbers.is_empty() {
            String::new()
        } else {
            format!(
                " PR {}",
                ts.pr_numbers
                    .iter()
                    .map(|n| format!("#{n}"))
                    .collect::<Vec<_>>()
                    .join(",")
            )
        };
        let interventions = if ts.intervention_count > 0 {
            format!(" ({} interventions)", ts.intervention_count)
        } else {
            String::new()
        };

        println!(
            "  [{icon}] {id:<16} {:<36} {status_str:<12} {agent}{prs}{interventions}",
            ts.task.title,
        );
    }

    println!();

    // Summary stats
    if summary.tasks_blocked > 0 {
        println!("Blocked: {} tasks", summary.tasks_blocked);
    }
    if summary.total_interventions > 0 {
        println!("Interventions: {}", summary.total_interventions);
    }

    match summary.status {
        SprintStatus::Completed => println!("\nSprint COMPLETE"),
        SprintStatus::Cancelled => println!("\nSprint CANCELLED"),
        SprintStatus::Active => {}
    }
}

/// Print the cross-project dashboard.
pub fn print_dashboard() -> Result<()> {
    let reg = load_registry()?;

    if reg.projects.is_empty() {
        println!("No projects registered.");
        println!("Run `caloron start` in a project directory to register it.");
        return Ok(());
    }

    println!("Caloron Dashboard");
    println!("=================\n");

    // Active sprints first
    let active = reg.active_sprints();
    if !active.is_empty() {
        println!("Active Sprints:");
        println!();
        for project in &active {
            let sprint = project.last_sprint.as_ref().unwrap();
            let pct = if sprint.tasks_total > 0 {
                sprint.tasks_done as f64 / sprint.tasks_total as f64
            } else {
                0.0
            };
            let filled = (pct * 10.0) as usize;
            let empty = 10 - filled;

            println!(
                "  {} ({})",
                project.name, project.repo
            );
            println!(
                "    Sprint: {} — {}",
                sprint.id, sprint.goal
            );
            println!(
                "    [{}{}] {}/{} tasks ({} in progress, {} blocked)",
                "#".repeat(filled),
                "-".repeat(empty),
                sprint.tasks_done,
                sprint.tasks_total,
                sprint.tasks_in_progress,
                sprint.tasks_blocked,
            );
            if sprint.total_interventions > 0 {
                println!(
                    "    Interventions: {}",
                    sprint.total_interventions
                );
            }
            println!(
                "    Path: {}",
                project.path.display()
            );
            println!();
        }
    }

    // Completed/other projects
    let other: Vec<_> = reg
        .projects
        .iter()
        .filter(|p| {
            p.last_sprint
                .as_ref()
                .map(|s| s.status != SprintStatus::Active)
                .unwrap_or(true)
        })
        .collect();

    if !other.is_empty() {
        println!("Other Projects:");
        println!();
        for project in &other {
            let status = project
                .last_sprint
                .as_ref()
                .map(|s| format!("{:?} ({})", s.status, s.id))
                .unwrap_or_else(|| "No sprint".into());

            println!(
                "  {:<24} {:<24} {}",
                project.name,
                status,
                project.path.display()
            );
        }
        println!();
    }

    Ok(())
}
