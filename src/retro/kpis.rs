use std::collections::HashMap;

use serde::{Deserialize, Serialize};

use caloron_types::feedback::SelfAssessment;

use super::collector::SprintFeedback;

/// Sprint-level KPIs computed from feedback.
/// Designed to be tracked across sprints for trend analysis.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SprintKpis {
    pub sprint_id: String,

    // -- Delivery --
    /// Tasks completed / total tasks
    pub completion_rate: f64,
    /// Tasks that failed or crashed
    pub failure_count: u32,
    /// Tasks that were blocked at retro time
    pub blocked_count: u32,

    // -- Quality --
    /// Average task clarity score (1-10)
    pub avg_clarity: f64,
    /// Percentage of tasks with clarity >= 7
    pub high_clarity_pct: f64,
    /// Average review cycles per task
    pub avg_review_cycles: f64,
    /// Tasks that went through 3+ review cycles
    pub review_loop_count: u32,

    // -- Efficiency --
    /// Total tokens consumed across all tasks
    pub total_tokens: u64,
    /// Average tokens per completed task
    pub avg_tokens_per_task: f64,
    /// Total time across all tasks (minutes)
    pub total_time_min: u32,
    /// Average time per completed task (minutes)
    pub avg_time_per_task: f64,

    // -- Resilience --
    /// Total supervisor interventions
    pub total_interventions: u32,
    /// Interventions per task (lower is better)
    pub interventions_per_task: f64,
    /// Tasks that required no intervention at all
    pub clean_task_pct: f64,

    // -- Agent health --
    /// Per-agent role metrics
    pub agent_metrics: HashMap<String, AgentKpis>,
}

/// Per-agent-role KPIs.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentKpis {
    pub role: String,
    pub tasks_completed: u32,
    pub tasks_failed: u32,
    pub avg_clarity: f64,
    pub avg_tokens: f64,
    pub avg_time_min: f64,
    pub avg_review_cycles: f64,
    pub interventions: u32,
}

/// Compute KPIs from sprint feedback.
pub fn compute_kpis(feedback: &SprintFeedback) -> SprintKpis {
    let total = feedback.tasks.len() as f64;
    if total == 0.0 {
        return empty_kpis(&feedback.sprint_id);
    }

    let completed: Vec<_> = feedback
        .tasks
        .iter()
        .filter(|t| t.self_assessment == SelfAssessment::Completed)
        .collect();

    let failed = feedback
        .tasks
        .iter()
        .filter(|t| matches!(t.self_assessment, SelfAssessment::Failed | SelfAssessment::Crashed))
        .count() as u32;

    let blocked = feedback
        .tasks
        .iter()
        .filter(|t| t.self_assessment == SelfAssessment::Blocked)
        .count() as u32;

    let completion_rate = completed.len() as f64 / total;

    let avg_clarity = feedback.tasks.iter().map(|t| t.task_clarity as f64).sum::<f64>() / total;

    let high_clarity = feedback.tasks.iter().filter(|t| t.task_clarity >= 7).count() as f64;
    let high_clarity_pct = high_clarity / total;

    let avg_review_cycles =
        feedback.tasks.iter().map(|t| t.review_cycles as f64).sum::<f64>() / total;

    let review_loop_count = feedback
        .tasks
        .iter()
        .filter(|t| t.review_cycles >= 3)
        .count() as u32;

    let total_tokens: u64 = feedback.tasks.iter().map(|t| t.tokens_consumed).sum();
    let avg_tokens_per_task = if completed.is_empty() {
        0.0
    } else {
        completed.iter().map(|t| t.tokens_consumed as f64).sum::<f64>() / completed.len() as f64
    };

    let total_time_min: u32 = feedback.tasks.iter().map(|t| t.time_to_complete_min).sum();
    let avg_time_per_task = if completed.is_empty() {
        0.0
    } else {
        completed.iter().map(|t| t.time_to_complete_min as f64).sum::<f64>()
            / completed.len() as f64
    };

    let total_interventions: u32 = feedback.tasks.iter().map(|t| t.supervisor_interventions).sum();
    let interventions_per_task = total_interventions as f64 / total;

    let clean_tasks = feedback
        .tasks
        .iter()
        .filter(|t| t.supervisor_interventions == 0 && t.self_assessment == SelfAssessment::Completed)
        .count() as f64;
    let clean_task_pct = clean_tasks / total;

    // Per-agent metrics
    let mut by_role: HashMap<String, Vec<&super::collector::TaskFeedback>> = HashMap::new();
    for task in &feedback.tasks {
        by_role.entry(task.agent_role.clone()).or_default().push(task);
    }

    let agent_metrics = by_role
        .into_iter()
        .map(|(role, tasks)| {
            let n = tasks.len() as f64;
            let completed_count = tasks
                .iter()
                .filter(|t| t.self_assessment == SelfAssessment::Completed)
                .count() as u32;
            let failed_count = tasks
                .iter()
                .filter(|t| {
                    matches!(
                        t.self_assessment,
                        SelfAssessment::Failed | SelfAssessment::Crashed
                    )
                })
                .count() as u32;

            let kpi = AgentKpis {
                role: role.clone(),
                tasks_completed: completed_count,
                tasks_failed: failed_count,
                avg_clarity: tasks.iter().map(|t| t.task_clarity as f64).sum::<f64>() / n,
                avg_tokens: tasks.iter().map(|t| t.tokens_consumed as f64).sum::<f64>() / n,
                avg_time_min: tasks.iter().map(|t| t.time_to_complete_min as f64).sum::<f64>() / n,
                avg_review_cycles: tasks.iter().map(|t| t.review_cycles as f64).sum::<f64>() / n,
                interventions: tasks.iter().map(|t| t.supervisor_interventions).sum(),
            };
            (role, kpi)
        })
        .collect();

    SprintKpis {
        sprint_id: feedback.sprint_id.clone(),
        completion_rate,
        failure_count: failed,
        blocked_count: blocked,
        avg_clarity,
        high_clarity_pct,
        avg_review_cycles,
        review_loop_count,
        total_tokens,
        avg_tokens_per_task,
        total_time_min,
        avg_time_per_task,
        total_interventions,
        interventions_per_task,
        clean_task_pct,
        agent_metrics,
    }
}

/// Compare two sprint KPIs and describe trends.
pub fn compare_kpis(previous: &SprintKpis, current: &SprintKpis) -> Vec<KpiTrend> {
    let mut trends = Vec::new();

    trends.push(trend(
        "Completion rate",
        previous.completion_rate,
        current.completion_rate,
        Direction::HigherIsBetter,
    ));
    trends.push(trend(
        "Avg clarity",
        previous.avg_clarity,
        current.avg_clarity,
        Direction::HigherIsBetter,
    ));
    trends.push(trend(
        "Avg tokens/task",
        previous.avg_tokens_per_task,
        current.avg_tokens_per_task,
        Direction::LowerIsBetter,
    ));
    trends.push(trend(
        "Avg time/task (min)",
        previous.avg_time_per_task,
        current.avg_time_per_task,
        Direction::LowerIsBetter,
    ));
    trends.push(trend(
        "Avg review cycles",
        previous.avg_review_cycles,
        current.avg_review_cycles,
        Direction::LowerIsBetter,
    ));
    trends.push(trend(
        "Interventions/task",
        previous.interventions_per_task,
        current.interventions_per_task,
        Direction::LowerIsBetter,
    ));
    trends.push(trend(
        "Clean task %",
        previous.clean_task_pct,
        current.clean_task_pct,
        Direction::HigherIsBetter,
    ));

    trends
}

#[derive(Debug, Clone)]
pub struct KpiTrend {
    pub name: String,
    pub previous: f64,
    pub current: f64,
    pub change_pct: f64,
    pub direction: TrendDirection,
}

#[derive(Debug, Clone, PartialEq)]
pub enum TrendDirection {
    Improved,
    Degraded,
    Stable,
}

enum Direction {
    HigherIsBetter,
    LowerIsBetter,
}

fn trend(name: &str, prev: f64, curr: f64, dir: Direction) -> KpiTrend {
    let change_pct = if prev == 0.0 {
        0.0
    } else {
        ((curr - prev) / prev) * 100.0
    };

    let threshold = 5.0; // 5% change to be significant
    let direction = if change_pct.abs() < threshold {
        TrendDirection::Stable
    } else {
        match dir {
            Direction::HigherIsBetter => {
                if change_pct > 0.0 {
                    TrendDirection::Improved
                } else {
                    TrendDirection::Degraded
                }
            }
            Direction::LowerIsBetter => {
                if change_pct < 0.0 {
                    TrendDirection::Improved
                } else {
                    TrendDirection::Degraded
                }
            }
        }
    };

    KpiTrend {
        name: name.into(),
        previous: prev,
        current: curr,
        change_pct,
        direction,
    }
}

fn empty_kpis(sprint_id: &str) -> SprintKpis {
    SprintKpis {
        sprint_id: sprint_id.into(),
        completion_rate: 0.0,
        failure_count: 0,
        blocked_count: 0,
        avg_clarity: 0.0,
        high_clarity_pct: 0.0,
        avg_review_cycles: 0.0,
        review_loop_count: 0,
        total_tokens: 0,
        avg_tokens_per_task: 0.0,
        total_time_min: 0,
        avg_time_per_task: 0.0,
        total_interventions: 0,
        interventions_per_task: 0.0,
        clean_task_pct: 0.0,
        agent_metrics: HashMap::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use super::super::collector::TaskFeedback;

    fn task(id: &str, role: &str, clarity: u8, tokens: u64, time: u32, reviews: u32, interventions: u32, assessment: SelfAssessment) -> TaskFeedback {
        TaskFeedback {
            task_id: id.into(),
            agent_role: role.into(),
            task_clarity: clarity,
            blockers: vec![],
            tools_used: vec![],
            tokens_consumed: tokens,
            time_to_complete_min: time,
            self_assessment: assessment,
            notes: None,
            review_cycles: reviews,
            supervisor_interventions: interventions,
        }
    }

    fn sample_feedback() -> SprintFeedback {
        SprintFeedback {
            sprint_id: "s1".into(),
            tasks: vec![
                task("t1", "dev", 8, 10000, 30, 1, 0, SelfAssessment::Completed),
                task("t2", "dev", 6, 15000, 45, 2, 1, SelfAssessment::Completed),
                task("t3", "qa", 3, 20000, 60, 4, 2, SelfAssessment::Completed),
                task("t4", "dev", 7, 8000, 20, 1, 0, SelfAssessment::Failed),
            ],
        }
    }

    #[test]
    fn test_compute_kpis_completion() {
        let kpis = compute_kpis(&sample_feedback());
        assert_eq!(kpis.completion_rate, 0.75); // 3/4
        assert_eq!(kpis.failure_count, 1);
    }

    #[test]
    fn test_compute_kpis_clarity() {
        let kpis = compute_kpis(&sample_feedback());
        assert_eq!(kpis.avg_clarity, 6.0); // (8+6+3+7)/4
        assert_eq!(kpis.high_clarity_pct, 0.5); // 2/4 (t1=8, t4=7)
    }

    #[test]
    fn test_compute_kpis_efficiency() {
        let kpis = compute_kpis(&sample_feedback());
        assert_eq!(kpis.total_tokens, 53000);
        // avg_tokens_per_task: only completed tasks (t1,t2,t3)
        assert!((kpis.avg_tokens_per_task - 15000.0).abs() < 0.1);
    }

    #[test]
    fn test_compute_kpis_resilience() {
        let kpis = compute_kpis(&sample_feedback());
        assert_eq!(kpis.total_interventions, 3);
        assert_eq!(kpis.interventions_per_task, 0.75);
        // Clean: t1 (completed, 0 interventions) = 1/4
        assert_eq!(kpis.clean_task_pct, 0.25);
    }

    #[test]
    fn test_compute_kpis_review_loops() {
        let kpis = compute_kpis(&sample_feedback());
        assert_eq!(kpis.review_loop_count, 1); // t3 has 4 cycles
        assert_eq!(kpis.avg_review_cycles, 2.0);
    }

    #[test]
    fn test_agent_metrics() {
        let kpis = compute_kpis(&sample_feedback());
        let dev = &kpis.agent_metrics["dev"];
        assert_eq!(dev.tasks_completed, 2);
        assert_eq!(dev.tasks_failed, 1);
        assert_eq!(dev.interventions, 1);

        let qa = &kpis.agent_metrics["qa"];
        assert_eq!(qa.tasks_completed, 1);
        assert_eq!(qa.interventions, 2);
    }

    #[test]
    fn test_compare_kpis_improved() {
        let prev = SprintKpis {
            avg_clarity: 5.0,
            avg_tokens_per_task: 20000.0,
            interventions_per_task: 1.5,
            ..empty_kpis("s1")
        };
        let curr = SprintKpis {
            avg_clarity: 7.0,
            avg_tokens_per_task: 12000.0,
            interventions_per_task: 0.5,
            ..empty_kpis("s2")
        };

        let trends = compare_kpis(&prev, &curr);

        let clarity = trends.iter().find(|t| t.name == "Avg clarity").unwrap();
        assert_eq!(clarity.direction, TrendDirection::Improved);

        let tokens = trends.iter().find(|t| t.name == "Avg tokens/task").unwrap();
        assert_eq!(tokens.direction, TrendDirection::Improved);

        let interventions = trends.iter().find(|t| t.name == "Interventions/task").unwrap();
        assert_eq!(interventions.direction, TrendDirection::Improved);
    }

    #[test]
    fn test_compare_kpis_degraded() {
        let prev = SprintKpis {
            avg_clarity: 8.0,
            avg_review_cycles: 1.0,
            ..empty_kpis("s1")
        };
        let curr = SprintKpis {
            avg_clarity: 4.0,
            avg_review_cycles: 3.0,
            ..empty_kpis("s2")
        };

        let trends = compare_kpis(&prev, &curr);

        let clarity = trends.iter().find(|t| t.name == "Avg clarity").unwrap();
        assert_eq!(clarity.direction, TrendDirection::Degraded);
    }

    #[test]
    fn test_empty_feedback() {
        let kpis = compute_kpis(&SprintFeedback {
            sprint_id: "empty".into(),
            tasks: vec![],
        });
        assert_eq!(kpis.completion_rate, 0.0);
        assert_eq!(kpis.total_tokens, 0);
    }
}
