use caloron_types::dag::DagState;
use caloron_types::feedback::{CaloronFeedback, SelfAssessment};

/// Collected feedback for a sprint, with task-level and sprint-level data.
#[derive(Debug, Clone)]
pub struct SprintFeedback {
    pub sprint_id: String,
    pub tasks: Vec<TaskFeedback>,
}

#[derive(Debug, Clone)]
pub struct TaskFeedback {
    pub task_id: String,
    pub agent_role: String,
    pub task_clarity: u8,
    pub blockers: Vec<String>,
    pub tools_used: Vec<String>,
    pub tokens_consumed: u64,
    pub time_to_complete_min: u32,
    pub self_assessment: SelfAssessment,
    pub notes: Option<String>,
    pub review_cycles: u32,
    pub supervisor_interventions: u32,
}

impl SprintFeedback {
    /// Build sprint feedback from a list of feedback comments and DAG state.
    pub fn from_feedbacks(
        sprint_id: &str,
        feedbacks: Vec<CaloronFeedback>,
        dag: &DagState,
    ) -> Self {
        let tasks = feedbacks
            .into_iter()
            .map(|f| {
                let intervention_count = dag
                    .tasks
                    .get(&f.task_id)
                    .map(|ts| ts.intervention_count)
                    .unwrap_or(0);

                let review_cycles = dag
                    .tasks
                    .get(&f.task_id)
                    .map(|ts| ts.pr_numbers.len().saturating_sub(1) as u32)
                    .unwrap_or(0);

                TaskFeedback {
                    task_id: f.task_id,
                    agent_role: f.agent_role,
                    task_clarity: f.task_clarity,
                    blockers: f.blockers,
                    tools_used: f.tools_used,
                    tokens_consumed: f.tokens_consumed,
                    time_to_complete_min: f.time_to_complete_min,
                    self_assessment: f.self_assessment,
                    notes: f.notes,
                    review_cycles,
                    supervisor_interventions: intervention_count,
                }
            })
            .collect();

        Self {
            sprint_id: sprint_id.to_string(),
            tasks,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use caloron_types::dag::*;
    use chrono::Utc;

    fn sample_dag_state() -> DagState {
        let dag = Dag {
            sprint: Sprint { id: "s1".into(), goal: "Test".into(), start: Utc::now(), max_duration_hours: 24 },
            agents: vec![AgentNode { id: "a1".into(), role: "dev".into(), definition_path: "a.yaml".into() }],
            tasks: vec![Task {
                id: "t1".into(), title: "Task 1".into(), assigned_to: "a1".into(),
                issue_template: "t.md".into(), depends_on: vec![], reviewed_by: None,
                github_issue_number: Some(10),
            }],
            review_policy: ReviewPolicy { required_approvals: 1, auto_merge: true, max_review_cycles: 3 },
            escalation: EscalationConfig { stall_threshold_minutes: 20, supervisor_id: "sup".into(), human_contact: "gh".into() },
        };
        DagState::from_dag(dag)
    }

    #[test]
    fn test_from_feedbacks() {
        let feedback = CaloronFeedback {
            task_id: "t1".into(),
            agent_role: "dev".into(),
            task_clarity: 7,
            blockers: vec!["unclear API format".into()],
            tools_used: vec!["bash".into(), "github_mcp".into()],
            tokens_consumed: 12000,
            time_to_complete_min: 45,
            self_assessment: SelfAssessment::Completed,
            notes: Some("Went well".into()),
        };

        let dag = sample_dag_state();
        let sf = SprintFeedback::from_feedbacks("s1", vec![feedback], &dag);

        assert_eq!(sf.sprint_id, "s1");
        assert_eq!(sf.tasks.len(), 1);
        assert_eq!(sf.tasks[0].task_clarity, 7);
        assert_eq!(sf.tasks[0].tokens_consumed, 12000);
    }
}
