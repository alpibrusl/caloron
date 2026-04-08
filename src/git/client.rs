use std::time::Duration;

use anyhow::{Context, Result};
use chrono::{DateTime, Utc};
use octocrab::Octocrab;

use caloron_types::git::{self, GitEvent};

/// GitHub API client wrapping octocrab with Caloron-specific operations.
pub struct GitHubClient {
    octocrab: Octocrab,
    owner: String,
    repo: String,
    last_poll: DateTime<Utc>,
}

impl GitHubClient {
    pub fn new(token: &str, owner: &str, repo: &str) -> Result<Self> {
        let octocrab = Octocrab::builder()
            .personal_token(token.to_string())
            .build()
            .context("Failed to build GitHub client")?;

        Ok(Self {
            octocrab,
            owner: owner.to_string(),
            repo: repo.to_string(),
            last_poll: Utc::now(),
        })
    }

    /// Create a new issue in the project repository.
    pub async fn create_issue(&self, title: &str, body: &str, labels: &[&str]) -> Result<u64> {
        let issues = self.octocrab.issues(&self.owner, &self.repo);
        let mut builder = issues.create(title).body(body);

        if !labels.is_empty() {
            let label_strings: Vec<String> = labels.iter().map(|l| l.to_string()).collect();
            builder = builder.labels(label_strings);
        }

        let issue = builder.send().await.context("Failed to create issue")?;

        tracing::info!(
            issue_number = issue.number,
            title,
            "Created issue"
        );

        Ok(issue.number)
    }

    /// Add a label to an issue or PR.
    pub async fn add_label(&self, issue_number: u64, label: &str) -> Result<()> {
        self.octocrab
            .issues(&self.owner, &self.repo)
            .add_labels(issue_number, &[label.to_string()])
            .await
            .with_context(|| format!("Failed to add label {label} to #{issue_number}"))?;

        Ok(())
    }

    /// Remove a label from an issue or PR.
    pub async fn remove_label(&self, issue_number: u64, label: &str) -> Result<()> {
        self.octocrab
            .issues(&self.owner, &self.repo)
            .remove_label(issue_number, label)
            .await
            .with_context(|| format!("Failed to remove label {label} from #{issue_number}"))?;

        Ok(())
    }

    /// Post a comment on an issue or PR.
    pub async fn create_comment(&self, issue_number: u64, body: &str) -> Result<()> {
        self.octocrab
            .issues(&self.owner, &self.repo)
            .create_comment(issue_number, body)
            .await
            .with_context(|| format!("Failed to comment on #{issue_number}"))?;

        Ok(())
    }

    /// Close an issue with a comment.
    pub async fn close_issue(&self, issue_number: u64, comment: &str) -> Result<()> {
        // Post comment first
        self.create_comment(issue_number, comment).await?;

        // Then close
        self.octocrab
            .issues(&self.owner, &self.repo)
            .update(issue_number)
            .state(octocrab::models::IssueState::Closed)
            .send()
            .await
            .with_context(|| format!("Failed to close #{issue_number}"))?;

        Ok(())
    }

    /// Request a review on a PR.
    pub async fn request_review(&self, pr_number: u64, reviewer: &str) -> Result<()> {
        self.octocrab
            .pulls(&self.owner, &self.repo)
            .request_reviews(pr_number, vec![reviewer.to_string()], Vec::new())
            .await
            .with_context(|| {
                format!("Failed to request review from {reviewer} on PR #{pr_number}")
            })?;

        Ok(())
    }

    /// Merge a pull request.
    pub async fn merge_pr(&self, pr_number: u64) -> Result<()> {
        self.octocrab
            .pulls(&self.owner, &self.repo)
            .merge(pr_number)
            .send()
            .await
            .with_context(|| format!("Failed to merge PR #{pr_number}"))?;

        tracing::info!(pr_number, "Merged PR");

        Ok(())
    }

    /// Ensure all Caloron labels exist in the repository.
    pub async fn ensure_labels(&self) -> Result<()> {
        let existing: Vec<String> = self
            .octocrab
            .issues(&self.owner, &self.repo)
            .list_labels_for_repo()
            .send()
            .await
            .context("Failed to list labels")?
            .items
            .into_iter()
            .map(|l| l.name)
            .collect();

        for label in git::labels::ALL {
            if !existing.iter().any(|e| e == label) {
                tracing::info!(label, "Creating missing label");
                self.octocrab
                    .issues(&self.owner, &self.repo)
                    .create_label(label, "0366d6", "Managed by Caloron")
                    .await
                    .with_context(|| format!("Failed to create label {label}"))?;
            }
        }

        Ok(())
    }

    /// Poll for new events since last poll.
    /// Implements event coalescing (Addendum H4): returns ALL events in one call.
    pub async fn poll_events(&mut self) -> Result<Vec<GitEvent>> {
        let since = self.last_poll;
        let events = self.fetch_events_since(since).await?;

        if let Some(last) = events.last() {
            self.last_poll = last.timestamp();
        }

        Ok(events)
    }

    /// Fetch events from the repository since a given timestamp.
    /// This is the core polling implementation with exponential backoff on rate limits.
    async fn fetch_events_since(&self, _since: DateTime<Utc>) -> Result<Vec<GitEvent>> {
        // TODO: Implement full event fetching from GitHub API
        // This will use octocrab to fetch:
        // - New/updated issues
        // - New/updated PRs
        // - PR reviews
        // - Comments
        // - Push events
        //
        // For now, return empty vec. Phase 4 implements the full event loop.
        //
        // Rate limit handling: exponential backoff with jitter
        // retry_delay = min(base_delay * 2^attempt + jitter, max_delay)
        Ok(Vec::new())
    }

    /// Exponential backoff delay calculation for rate limiting.
    pub fn backoff_delay(attempt: u32, base_ms: u64, max_ms: u64) -> Duration {
        let delay = base_ms.saturating_mul(1u64 << attempt.min(10));
        let jitter = rand_jitter(delay / 4);
        Duration::from_millis(delay.saturating_add(jitter).min(max_ms))
    }
}

/// Simple jitter based on timestamp (no external rand dependency needed).
fn rand_jitter(max: u64) -> u64 {
    if max == 0 {
        return 0;
    }
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .subsec_nanos() as u64;
    nanos % max
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_backoff_delay_increases() {
        let d0 = GitHubClient::backoff_delay(0, 100, 10_000);
        let d1 = GitHubClient::backoff_delay(1, 100, 10_000);
        let d2 = GitHubClient::backoff_delay(2, 100, 10_000);

        // Each attempt should roughly double (minus jitter)
        assert!(d1 > d0);
        assert!(d2 > d1);
    }

    #[test]
    fn test_backoff_delay_caps_at_max() {
        let d = GitHubClient::backoff_delay(20, 100, 5_000);
        assert!(d.as_millis() <= 5_000);
    }

    #[test]
    fn test_jitter_bounded() {
        for _ in 0..100 {
            let j = rand_jitter(1000);
            assert!(j < 1000);
        }
    }
}
