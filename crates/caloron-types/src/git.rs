use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

/// All Git events that the monitor can observe from the project repository.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "event_type", rename_all = "snake_case")]
pub enum GitEvent {
    IssueOpened {
        number: u64,
        title: String,
        labels: Vec<String>,
        timestamp: DateTime<Utc>,
    },
    IssueLabeled {
        number: u64,
        label: String,
        timestamp: DateTime<Utc>,
    },
    IssueClosed {
        number: u64,
        closer: String,
        timestamp: DateTime<Utc>,
    },
    PrOpened {
        number: u64,
        title: String,
        linked_issue: Option<u64>,
        author: String,
        timestamp: DateTime<Utc>,
    },
    PrReviewSubmitted {
        pr_number: u64,
        reviewer: String,
        state: ReviewState,
        body: String,
        timestamp: DateTime<Utc>,
    },
    PrMerged {
        number: u64,
        timestamp: DateTime<Utc>,
    },
    /// Added per Addendum E3: handles PR closed without merge.
    PrClosed {
        number: u64,
        closer: String,
        linked_issue: Option<u64>,
        timestamp: DateTime<Utc>,
    },
    CommentCreated {
        issue_number: u64,
        body: String,
        author: String,
        timestamp: DateTime<Utc>,
    },
    PushReceived {
        branch: String,
        author: String,
        commit_sha: String,
        timestamp: DateTime<Utc>,
    },
}

impl GitEvent {
    pub fn timestamp(&self) -> DateTime<Utc> {
        match self {
            GitEvent::IssueOpened { timestamp, .. }
            | GitEvent::IssueLabeled { timestamp, .. }
            | GitEvent::IssueClosed { timestamp, .. }
            | GitEvent::PrOpened { timestamp, .. }
            | GitEvent::PrReviewSubmitted { timestamp, .. }
            | GitEvent::PrMerged { timestamp, .. }
            | GitEvent::PrClosed { timestamp, .. }
            | GitEvent::CommentCreated { timestamp, .. }
            | GitEvent::PushReceived { timestamp, .. } => *timestamp,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ReviewState {
    Approved,
    ChangesRequested,
    Commented,
}

/// Caloron-managed label constants.
pub mod labels {
    pub const TASK: &str = "caloron:task";
    pub const ASSIGNED: &str = "caloron:assigned";
    pub const IN_PROGRESS: &str = "caloron:in-progress";
    pub const BLOCKED: &str = "caloron:blocked";
    pub const REVIEW_PENDING: &str = "caloron:review-pending";
    pub const CHANGES_REQUESTED: &str = "caloron:changes-requested";
    pub const MERGE_READY: &str = "caloron:merge-ready";
    pub const DONE: &str = "caloron:done";
    pub const ESCALATED: &str = "caloron:escalated";
    pub const STALLED: &str = "caloron:stalled";
    pub const SPRINT_CANCELLED: &str = "caloron:sprint-cancelled";

    /// All labels that Caloron manages (for LabelManager setup).
    pub const ALL: &[&str] = &[
        TASK,
        ASSIGNED,
        IN_PROGRESS,
        BLOCKED,
        REVIEW_PENDING,
        CHANGES_REQUESTED,
        MERGE_READY,
        DONE,
        ESCALATED,
        STALLED,
        SPRINT_CANCELLED,
    ];
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_git_event_serialization() {
        let event = GitEvent::PrMerged {
            number: 42,
            timestamp: Utc::now(),
        };
        let json = serde_json::to_string(&event).unwrap();
        assert!(json.contains("\"event_type\":\"pr_merged\""));

        let deserialized: GitEvent = serde_json::from_str(&json).unwrap();
        if let GitEvent::PrMerged { number, .. } = deserialized {
            assert_eq!(number, 42);
        } else {
            panic!("wrong variant");
        }
    }

    #[test]
    fn test_pr_closed_event() {
        let event = GitEvent::PrClosed {
            number: 15,
            closer: "reviewer-1".into(),
            linked_issue: Some(10),
            timestamp: Utc::now(),
        };
        let json = serde_json::to_string(&event).unwrap();
        assert!(json.contains("\"event_type\":\"pr_closed\""));
    }

    #[test]
    fn test_review_state_serialization() {
        let state = ReviewState::ChangesRequested;
        let json = serde_json::to_string(&state).unwrap();
        assert_eq!(json, "\"changes_requested\"");
    }

    #[test]
    fn test_all_labels_prefixed() {
        for label in labels::ALL {
            assert!(
                label.starts_with("caloron:"),
                "Label {label} missing caloron: prefix"
            );
        }
    }
}
