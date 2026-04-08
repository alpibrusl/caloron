//! End-to-end sprint integration test.
//!
//! This test exercises the full DAG lifecycle through the event handler,
//! simulating the Git events that would occur during a real sprint.

use chrono::Utc;

use caloron_types::dag::*;
use caloron_types::feedback::{CaloronFeedback, SelfAssessment};
use caloron_types::git::{GitEvent, ReviewState, labels};

// These are internal to the daemon crate, so we test via the types + dag module.
// The full event handler test is in src/git/monitor.rs tests.

#[test]
fn test_e2e_sprint_dag_lifecycle() {
    // === Setup: Create a DAG with 2 tasks and a dependency ===
    let dag = Dag {
        sprint: Sprint {
            id: "sprint-e2e-test".into(),
            goal: "Add health and metrics endpoints".into(),
            start: Utc::now(),
            max_duration_hours: 4,
        },
        agents: vec![
            AgentNode {
                id: "dev-1".into(),
                role: "api-developer".into(),
                definition_path: "agents/api-developer.yaml".into(), spec: None,
            },
            AgentNode {
                id: "dev-2".into(),
                role: "api-developer".into(),
                definition_path: "agents/api-developer.yaml".into(), spec: None,
            },
            AgentNode {
                id: "rev-1".into(),
                role: "code-reviewer".into(),
                definition_path: "agents/code-reviewer.yaml".into(), spec: None,
            },
        ],
        tasks: vec![
            Task {
                id: "health-endpoint".into(),
                title: "Implement /health endpoint".into(),
                assigned_to: "dev-1".into(),
                issue_template: "tasks/endpoint.md".into(),
                depends_on: vec![],
                reviewed_by: Some("rev-1".into()),
                github_issue_number: None,
            },
            Task {
                id: "metrics-endpoint".into(),
                title: "Implement /metrics endpoint".into(),
                assigned_to: "dev-2".into(),
                issue_template: "tasks/endpoint.md".into(),
                depends_on: vec!["health-endpoint".into()],
                reviewed_by: Some("rev-1".into()),
                github_issue_number: None,
            },
        ],
        review_policy: ReviewPolicy {
            required_approvals: 1,
            auto_merge: true,
            max_review_cycles: 3,
        },
        escalation: EscalationConfig {
            stall_threshold_minutes: 15,
            supervisor_id: "supervisor".into(),
            human_contact: "github_issue".into(),
        },
    };

    // === Phase 1: Load DAG ===
    let mut engine = DagState::from_dag(dag);

    // health-endpoint has no deps → should be in initial unblocked set
    let unblocked = engine.evaluate_unblocked();
    assert!(
        unblocked.contains(&"health-endpoint".to_string()),
        "health-endpoint should be unblocked"
    );
    assert!(
        !unblocked.contains(&"metrics-endpoint".to_string()),
        "metrics-endpoint should be blocked"
    );

    // Transition health-endpoint to Ready (as the DagEngine would)
    engine
        .tasks
        .get_mut("health-endpoint")
        .unwrap()
        .transition(TaskStatus::Ready);

    // === Phase 2: health-endpoint execution ===

    // Issue created → agent starts working
    let ts = engine.tasks.get_mut("health-endpoint").unwrap();
    ts.task.github_issue_number = Some(10);
    ts.transition(TaskStatus::InProgress);
    assert_eq!(ts.status, TaskStatus::InProgress);

    // PR opened → in review
    let ts = engine.tasks.get_mut("health-endpoint").unwrap();
    ts.pr_numbers.push(100);
    ts.transition(TaskStatus::InReview);
    assert_eq!(ts.status, TaskStatus::InReview);

    // PR merged → done
    let ts = engine.tasks.get_mut("health-endpoint").unwrap();
    ts.transition(TaskStatus::Done);
    assert_eq!(ts.status, TaskStatus::Done);

    // Check that metrics-endpoint is now unblocked
    let unblocked = engine.evaluate_unblocked();
    assert_eq!(unblocked, vec!["metrics-endpoint"]);

    // Transition metrics-endpoint to Ready
    engine
        .tasks
        .get_mut("metrics-endpoint")
        .unwrap()
        .transition(TaskStatus::Ready);

    // === Phase 3: metrics-endpoint execution with changes requested ===

    // Issue created → agent starts
    let ts = engine.tasks.get_mut("metrics-endpoint").unwrap();
    ts.task.github_issue_number = Some(11);
    ts.transition(TaskStatus::InProgress);

    // PR opened → in review
    let ts = engine.tasks.get_mut("metrics-endpoint").unwrap();
    ts.pr_numbers.push(101);
    ts.transition(TaskStatus::InReview);

    // Reviewer requests changes → back to in progress
    let ts = engine.tasks.get_mut("metrics-endpoint").unwrap();
    ts.transition(TaskStatus::InProgress);
    assert_eq!(ts.status, TaskStatus::InProgress);

    // Agent pushes fixes, opens updated PR review cycle
    let ts = engine.tasks.get_mut("metrics-endpoint").unwrap();
    ts.transition(TaskStatus::InReview);

    // Approved and merged
    let ts = engine.tasks.get_mut("metrics-endpoint").unwrap();
    ts.transition(TaskStatus::Done);

    // === Phase 4: Sprint complete ===
    assert!(engine.is_sprint_complete());

    // Both tasks are Done
    assert_eq!(engine.tasks["health-endpoint"].status, TaskStatus::Done);
    assert_eq!(engine.tasks["metrics-endpoint"].status, TaskStatus::Done);
}

#[test]
fn test_e2e_sprint_cancellation() {
    let dag = Dag {
        sprint: Sprint {
            id: "sprint-cancel-test".into(),
            goal: "Test cancellation".into(),
            start: Utc::now(),
            max_duration_hours: 4,
        },
        agents: vec![
            AgentNode {
                id: "dev-1".into(),
                role: "developer".into(),
                definition_path: "a.yaml".into(), spec: None,
            },
        ],
        tasks: vec![
            Task {
                id: "t1".into(),
                title: "Task 1".into(),
                assigned_to: "dev-1".into(),
                issue_template: "t.md".into(),
                depends_on: vec![],
                reviewed_by: None,
                github_issue_number: None,
            },
            Task {
                id: "t2".into(),
                title: "Task 2".into(),
                assigned_to: "dev-1".into(),
                issue_template: "t.md".into(),
                depends_on: vec!["t1".into()],
                reviewed_by: None,
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
            supervisor_id: "sup".into(),
            human_contact: "gh".into(),
        },
    };

    let mut engine = DagState::from_dag(dag);

    // Start t1
    engine
        .tasks
        .get_mut("t1")
        .unwrap()
        .transition(TaskStatus::Ready);
    engine
        .tasks
        .get_mut("t1")
        .unwrap()
        .transition(TaskStatus::InProgress);

    // Cancel sprint — both tasks should be cancelled
    for ts in engine.tasks.values_mut() {
        if !matches!(ts.status, TaskStatus::Done | TaskStatus::Cancelled { .. }) {
            ts.transition(TaskStatus::Cancelled {
                reason: "sprint_cancelled".into(),
            });
        }
    }

    assert!(matches!(
        engine.tasks["t1"].status,
        TaskStatus::Cancelled { .. }
    ));
    assert!(matches!(
        engine.tasks["t2"].status,
        TaskStatus::Cancelled { .. }
    ));

    // Sprint is "complete" (all tasks terminal)
    assert!(engine.is_sprint_complete());
}

#[test]
fn test_e2e_feedback_and_retro() {
    // Simulate feedback from completed tasks
    let feedback = CaloronFeedback {
        task_id: "health-endpoint".into(),
        agent_role: "api-developer".into(),
        task_clarity: 8,
        blockers: vec![],
        tools_used: vec!["bash".into(), "noether:json_validate".into()],
        tokens_consumed: 10000,
        time_to_complete_min: 10,
        self_assessment: SelfAssessment::Completed,
        notes: Some("Clean implementation, good issue spec".into()),
    };

    // Verify feedback can be serialized to YAML (as posted in comments)
    let yaml = serde_yaml::to_string(&serde_yaml::to_value(&feedback).unwrap()).unwrap();
    assert!(yaml.contains("health-endpoint"));
    assert!(yaml.contains("json_validate"));

    // Verify feedback roundtrips through YAML
    let wrapper = format!(
        "---\ncaloron_feedback:\n  task_id: \"{}\"\n  agent_role: \"{}\"\n  task_clarity: {}\n  blockers: []\n  tools_used:\n    - \"{}\"\n  tokens_consumed: {}\n  time_to_complete_min: {}\n  self_assessment: {}\n---",
        feedback.task_id,
        feedback.agent_role,
        feedback.task_clarity,
        feedback.tools_used[0],
        feedback.tokens_consumed,
        feedback.time_to_complete_min,
        "completed",
    );

    let parsed = caloron_types::feedback::FeedbackComment::parse_from_comment(&wrapper);
    assert!(parsed.is_some());
    let parsed = parsed.unwrap();
    assert_eq!(parsed.task_id, "health-endpoint");
    assert_eq!(parsed.task_clarity, 8);
    assert_eq!(parsed.self_assessment, SelfAssessment::Completed);
}

#[test]
fn test_e2e_dag_persistence_roundtrip() {
    let dag = Dag {
        sprint: Sprint {
            id: "sprint-persist-test".into(),
            goal: "Test persistence".into(),
            start: Utc::now(),
            max_duration_hours: 4,
        },
        agents: vec![AgentNode {
            id: "dev-1".into(),
            role: "developer".into(),
            definition_path: "a.yaml".into(), spec: None,
        }],
        tasks: vec![
            Task {
                id: "t1".into(),
                title: "Task 1".into(),
                assigned_to: "dev-1".into(),
                issue_template: "t.md".into(),
                depends_on: vec![],
                reviewed_by: None,
                github_issue_number: None,
            },
            Task {
                id: "t2".into(),
                title: "Task 2".into(),
                assigned_to: "dev-1".into(),
                issue_template: "t.md".into(),
                depends_on: vec!["t1".into()],
                reviewed_by: None,
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
            supervisor_id: "sup".into(),
            human_contact: "gh".into(),
        },
    };

    // Create state and make some transitions
    let mut state = DagState::from_dag(dag);

    let unblocked = state.evaluate_unblocked();
    for id in &unblocked {
        state.tasks.get_mut(id).unwrap().transition(TaskStatus::Ready);
    }

    state
        .tasks
        .get_mut("t1")
        .unwrap()
        .transition(TaskStatus::InProgress);

    // Serialize to JSON
    let json = serde_json::to_string_pretty(&state).unwrap();

    // Deserialize back
    let restored: DagState = serde_json::from_str(&json).unwrap();

    assert_eq!(restored.sprint.id, "sprint-persist-test");
    assert_eq!(restored.tasks["t1"].status, TaskStatus::InProgress);
    assert_eq!(restored.tasks["t2"].status, TaskStatus::Pending);
    assert_eq!(restored.tasks.len(), 2);
}
