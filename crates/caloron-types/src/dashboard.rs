use std::collections::HashMap;
use std::path::PathBuf;

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

/// A registered project that Caloron knows about.
/// Stored in ~/.caloron/projects.json.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RegisteredProject {
    /// Project name from caloron.toml
    pub name: String,
    /// GitHub owner/repo
    pub repo: String,
    /// Absolute path to the project root on disk
    pub path: PathBuf,
    /// When this project was registered
    pub registered_at: DateTime<Utc>,
    /// Last known sprint state
    pub last_sprint: Option<SprintSummary>,
}

/// Compact sprint summary for dashboard display.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SprintSummary {
    pub id: String,
    pub goal: String,
    pub status: SprintStatus,
    pub started_at: DateTime<Utc>,
    pub tasks_total: usize,
    pub tasks_done: usize,
    pub tasks_in_progress: usize,
    pub tasks_blocked: usize,
    pub agents_running: usize,
    pub total_interventions: u32,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum SprintStatus {
    Active,
    Completed,
    Cancelled,
}

/// The global project registry.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ProjectRegistry {
    pub projects: Vec<RegisteredProject>,
}

impl ProjectRegistry {
    /// Register a project (or update if it already exists by path).
    pub fn register(&mut self, project: RegisteredProject) {
        if let Some(existing) = self
            .projects
            .iter_mut()
            .find(|p| p.path == project.path)
        {
            existing.name = project.name;
            existing.repo = project.repo;
            existing.last_sprint = project.last_sprint;
        } else {
            self.projects.push(project);
        }
    }

    /// Update the sprint summary for a project.
    pub fn update_sprint(&mut self, path: &PathBuf, summary: SprintSummary) {
        if let Some(project) = self.projects.iter_mut().find(|p| &p.path == path) {
            project.last_sprint = Some(summary);
        }
    }

    /// Get all projects with active sprints.
    pub fn active_sprints(&self) -> Vec<&RegisteredProject> {
        self.projects
            .iter()
            .filter(|p| {
                p.last_sprint
                    .as_ref()
                    .is_some_and(|s| s.status == SprintStatus::Active)
            })
            .collect()
    }

    /// Get all registered projects.
    pub fn all(&self) -> &[RegisteredProject] {
        &self.projects
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_register_and_update() {
        let mut reg = ProjectRegistry::default();

        reg.register(RegisteredProject {
            name: "project-a".into(),
            repo: "owner/project-a".into(),
            path: PathBuf::from("/home/user/project-a"),
            registered_at: Utc::now(),
            last_sprint: None,
        });

        assert_eq!(reg.projects.len(), 1);

        // Register same path again — should update, not duplicate
        reg.register(RegisteredProject {
            name: "project-a-renamed".into(),
            repo: "owner/project-a".into(),
            path: PathBuf::from("/home/user/project-a"),
            registered_at: Utc::now(),
            last_sprint: None,
        });

        assert_eq!(reg.projects.len(), 1);
        assert_eq!(reg.projects[0].name, "project-a-renamed");
    }

    #[test]
    fn test_active_sprints() {
        let mut reg = ProjectRegistry::default();

        reg.register(RegisteredProject {
            name: "active".into(),
            repo: "owner/active".into(),
            path: PathBuf::from("/active"),
            registered_at: Utc::now(),
            last_sprint: Some(SprintSummary {
                id: "s1".into(),
                goal: "Test".into(),
                status: SprintStatus::Active,
                started_at: Utc::now(),
                tasks_total: 5,
                tasks_done: 2,
                tasks_in_progress: 2,
                tasks_blocked: 1,
                agents_running: 3,
                total_interventions: 1,
                updated_at: Utc::now(),
            }),
        });

        reg.register(RegisteredProject {
            name: "done".into(),
            repo: "owner/done".into(),
            path: PathBuf::from("/done"),
            registered_at: Utc::now(),
            last_sprint: Some(SprintSummary {
                id: "s2".into(),
                goal: "Done".into(),
                status: SprintStatus::Completed,
                started_at: Utc::now(),
                tasks_total: 3,
                tasks_done: 3,
                tasks_in_progress: 0,
                tasks_blocked: 0,
                agents_running: 0,
                total_interventions: 0,
                updated_at: Utc::now(),
            }),
        });

        reg.register(RegisteredProject {
            name: "idle".into(),
            repo: "owner/idle".into(),
            path: PathBuf::from("/idle"),
            registered_at: Utc::now(),
            last_sprint: None,
        });

        let active = reg.active_sprints();
        assert_eq!(active.len(), 1);
        assert_eq!(active[0].name, "active");
    }

    #[test]
    fn test_serialization_roundtrip() {
        let mut reg = ProjectRegistry::default();
        reg.register(RegisteredProject {
            name: "test".into(),
            repo: "o/r".into(),
            path: PathBuf::from("/test"),
            registered_at: Utc::now(),
            last_sprint: None,
        });

        let json = serde_json::to_string(&reg).unwrap();
        let restored: ProjectRegistry = serde_json::from_str(&json).unwrap();
        assert_eq!(restored.projects.len(), 1);
    }
}
