use std::path::{Path, PathBuf};
use std::process::Command;

use anyhow::{Context, Result, bail};

/// Manages git worktrees for agent isolation.
/// Each agent gets a dedicated worktree so they work on the same git history
/// but in separate filesystem paths with separate uncommitted state.
pub struct WorktreeManager {
    /// Root of the project repository
    repo_root: PathBuf,
    /// Directory where worktrees are created (.caloron/worktrees/)
    worktree_dir: PathBuf,
}

/// Info about an existing worktree.
#[derive(Debug, Clone)]
pub struct WorktreeInfo {
    pub path: PathBuf,
    pub branch: String,
    pub agent_id: String,
    pub sprint_id: String,
}

impl WorktreeManager {
    pub fn new(repo_root: PathBuf) -> Self {
        let worktree_dir = repo_root.join(".caloron").join("worktrees");
        Self {
            repo_root,
            worktree_dir,
        }
    }

    /// Create a new worktree for an agent in a sprint.
    /// Creates a new branch `agent/{agent_id}/sprint-{sprint_id}` based on the current HEAD.
    pub fn create(&self, agent_id: &str, sprint_id: &str) -> Result<PathBuf> {
        let worktree_name = format!("{agent_id}-{sprint_id}");
        let worktree_path = self.worktree_dir.join(&worktree_name);
        let branch_name = format!("agent/{agent_id}/{sprint_id}");

        // Ensure the worktree directory exists
        std::fs::create_dir_all(&self.worktree_dir)
            .context("Failed to create worktree directory")?;

        // Check if worktree already exists (sprint resume case)
        if worktree_path.exists() {
            tracing::warn!(
                path = %worktree_path.display(),
                "Worktree already exists — reusing for sprint resume"
            );
            return Ok(worktree_path);
        }

        // Create the worktree with a new branch
        let output = Command::new("git")
            .args([
                "worktree",
                "add",
                worktree_path.to_str().unwrap(),
                "-b",
                &branch_name,
            ])
            .current_dir(&self.repo_root)
            .output()
            .context("Failed to execute git worktree add")?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            // Branch may already exist (from a previous sprint attempt)
            if stderr.contains("already exists") {
                // Try without -b, just checking out the existing branch
                let output = Command::new("git")
                    .args([
                        "worktree",
                        "add",
                        worktree_path.to_str().unwrap(),
                        &branch_name,
                    ])
                    .current_dir(&self.repo_root)
                    .output()
                    .context("Failed to execute git worktree add (existing branch)")?;

                if !output.status.success() {
                    let stderr = String::from_utf8_lossy(&output.stderr);
                    bail!("Failed to create worktree: {stderr}");
                }
            } else {
                bail!("Failed to create worktree: {stderr}");
            }
        }

        tracing::info!(
            path = %worktree_path.display(),
            branch = branch_name,
            "Created worktree"
        );

        Ok(worktree_path)
    }

    /// Remove a worktree. Handles dirty worktrees by force-removing.
    pub fn remove(&self, agent_id: &str, sprint_id: &str) -> Result<()> {
        let worktree_name = format!("{agent_id}-{sprint_id}");
        let worktree_path = self.worktree_dir.join(&worktree_name);

        if !worktree_path.exists() {
            tracing::debug!(
                path = %worktree_path.display(),
                "Worktree does not exist — nothing to remove"
            );
            return Ok(());
        }

        // Try clean removal first
        let output = Command::new("git")
            .args([
                "worktree",
                "remove",
                worktree_path.to_str().unwrap(),
            ])
            .current_dir(&self.repo_root)
            .output()
            .context("Failed to execute git worktree remove")?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            if stderr.contains("dirty") || stderr.contains("untracked") {
                tracing::warn!(
                    path = %worktree_path.display(),
                    "Worktree has uncommitted changes — force removing"
                );
                let output = Command::new("git")
                    .args([
                        "worktree",
                        "remove",
                        "--force",
                        worktree_path.to_str().unwrap(),
                    ])
                    .current_dir(&self.repo_root)
                    .output()
                    .context("Failed to force remove worktree")?;

                if !output.status.success() {
                    let stderr = String::from_utf8_lossy(&output.stderr);
                    bail!("Failed to force remove worktree: {stderr}");
                }
            } else {
                bail!("Failed to remove worktree: {stderr}");
            }
        }

        tracing::info!(path = %worktree_path.display(), "Removed worktree");

        Ok(())
    }

    /// List all Caloron-managed worktrees.
    pub fn list(&self) -> Result<Vec<WorktreeInfo>> {
        let output = Command::new("git")
            .args(["worktree", "list", "--porcelain"])
            .current_dir(&self.repo_root)
            .output()
            .context("Failed to execute git worktree list")?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            bail!("Failed to list worktrees: {stderr}");
        }

        let stdout = String::from_utf8_lossy(&output.stdout);
        let mut worktrees = Vec::new();

        let mut current_path: Option<PathBuf> = None;
        let mut current_branch: Option<String> = None;

        for line in stdout.lines() {
            if let Some(path) = line.strip_prefix("worktree ") {
                current_path = Some(PathBuf::from(path));
            } else if let Some(branch) = line.strip_prefix("branch refs/heads/") {
                current_branch = Some(branch.to_string());
            } else if line.is_empty() {
                // End of entry — check if it's a Caloron worktree
                if let (Some(path), Some(branch)) = (&current_path, &current_branch) {
                    if let Some(info) = parse_caloron_worktree(path, branch) {
                        worktrees.push(info);
                    }
                }
                current_path = None;
                current_branch = None;
            }
        }

        // Handle last entry (no trailing blank line)
        if let (Some(path), Some(branch)) = (&current_path, &current_branch) {
            if let Some(info) = parse_caloron_worktree(path, branch) {
                worktrees.push(info);
            }
        }

        Ok(worktrees)
    }

    /// Mark a worktree as cancelled (preserves it for debugging per Addendum H3).
    pub fn mark_cancelled(&self, agent_id: &str, sprint_id: &str) -> Result<()> {
        let worktree_name = format!("{agent_id}-{sprint_id}");
        let worktree_path = self.worktree_dir.join(&worktree_name);
        let marker = worktree_path.join(".cancelled");

        if worktree_path.exists() {
            std::fs::write(&marker, "Sprint cancelled\n")
                .context("Failed to write cancellation marker")?;
            tracing::info!(path = %worktree_path.display(), "Marked worktree as cancelled");
        }

        Ok(())
    }

    /// Get the path where a worktree would be created.
    pub fn worktree_path(&self, agent_id: &str, sprint_id: &str) -> PathBuf {
        let worktree_name = format!("{agent_id}-{sprint_id}");
        self.worktree_dir.join(worktree_name)
    }
}

/// Parse a Caloron worktree from git porcelain output.
/// Caloron branches follow the pattern: agent/{agent_id}/{sprint_id}
fn parse_caloron_worktree(path: &Path, branch: &str) -> Option<WorktreeInfo> {
    let parts: Vec<&str> = branch.splitn(3, '/').collect();
    if parts.len() == 3 && parts[0] == "agent" {
        Some(WorktreeInfo {
            path: path.to_path_buf(),
            branch: branch.to_string(),
            agent_id: parts[1].to_string(),
            sprint_id: parts[2].to_string(),
        })
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::process::Command;

    /// Create a temporary git repo for testing.
    fn setup_test_repo() -> tempfile::TempDir {
        let dir = tempfile::tempdir().unwrap();
        Command::new("git")
            .args(["init"])
            .current_dir(dir.path())
            .output()
            .unwrap();
        Command::new("git")
            .args(["commit", "--allow-empty", "-m", "initial"])
            .current_dir(dir.path())
            .output()
            .unwrap();
        dir
    }

    #[test]
    fn test_create_and_list_worktree() {
        let repo = setup_test_repo();
        let mgr = WorktreeManager::new(repo.path().to_path_buf());

        let path = mgr.create("backend-1", "sprint-1").unwrap();
        assert!(path.exists());

        let worktrees = mgr.list().unwrap();
        assert_eq!(worktrees.len(), 1);
        assert_eq!(worktrees[0].agent_id, "backend-1");
        assert_eq!(worktrees[0].sprint_id, "sprint-1");
        assert_eq!(worktrees[0].branch, "agent/backend-1/sprint-1");
    }

    #[test]
    fn test_create_worktree_already_exists() {
        let repo = setup_test_repo();
        let mgr = WorktreeManager::new(repo.path().to_path_buf());

        let path1 = mgr.create("backend-1", "sprint-1").unwrap();
        let path2 = mgr.create("backend-1", "sprint-1").unwrap();
        // Should reuse existing worktree
        assert_eq!(path1, path2);
    }

    #[test]
    fn test_remove_worktree() {
        let repo = setup_test_repo();
        let mgr = WorktreeManager::new(repo.path().to_path_buf());

        let path = mgr.create("backend-1", "sprint-1").unwrap();
        assert!(path.exists());

        mgr.remove("backend-1", "sprint-1").unwrap();
        assert!(!path.exists());

        let worktrees = mgr.list().unwrap();
        assert!(worktrees.is_empty());
    }

    #[test]
    fn test_remove_nonexistent_worktree() {
        let repo = setup_test_repo();
        let mgr = WorktreeManager::new(repo.path().to_path_buf());

        // Should not error
        mgr.remove("nonexistent", "sprint-1").unwrap();
    }

    #[test]
    fn test_mark_cancelled() {
        let repo = setup_test_repo();
        let mgr = WorktreeManager::new(repo.path().to_path_buf());

        let path = mgr.create("backend-1", "sprint-1").unwrap();
        mgr.mark_cancelled("backend-1", "sprint-1").unwrap();

        assert!(path.join(".cancelled").exists());
    }

    #[test]
    fn test_multiple_worktrees() {
        let repo = setup_test_repo();
        let mgr = WorktreeManager::new(repo.path().to_path_buf());

        mgr.create("backend-1", "sprint-1").unwrap();
        mgr.create("backend-2", "sprint-1").unwrap();
        mgr.create("qa-1", "sprint-1").unwrap();

        let worktrees = mgr.list().unwrap();
        assert_eq!(worktrees.len(), 3);
    }

    #[test]
    fn test_worktree_path() {
        let mgr = WorktreeManager::new(PathBuf::from("/project"));
        let path = mgr.worktree_path("backend-1", "sprint-1");
        assert_eq!(
            path,
            PathBuf::from("/project/.caloron/worktrees/backend-1-sprint-1")
        );
    }

    #[test]
    fn test_parse_caloron_worktree() {
        let info = parse_caloron_worktree(
            Path::new("/tmp/worktree"),
            "agent/backend-1/sprint-2026-w2",
        );
        let info = info.unwrap();
        assert_eq!(info.agent_id, "backend-1");
        assert_eq!(info.sprint_id, "sprint-2026-w2");
    }

    #[test]
    fn test_parse_non_caloron_branch() {
        let info = parse_caloron_worktree(Path::new("/tmp/worktree"), "main");
        assert!(info.is_none());

        let info = parse_caloron_worktree(Path::new("/tmp/worktree"), "feature/my-feature");
        assert!(info.is_none());
    }
}
