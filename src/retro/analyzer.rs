use std::collections::HashMap;

use super::collector::{SprintFeedback, TaskFeedback};

/// Analysis results from a sprint's feedback.
#[derive(Debug, Clone, Default)]
pub struct RetroAnalysis {
    pub clarity_issues: Vec<ClarityIssue>,
    pub discovered_dependencies: Vec<DiscoveredDependency>,
    pub tool_gaps: Vec<ToolGap>,
    pub review_loop_analysis: Vec<ReviewLoopIssue>,
    pub efficiency_anomalies: Vec<EfficiencyAnomaly>,
    pub noether_usage: NoetherUsage,
}

/// Noether stage usage analysis.
#[derive(Debug, Clone, Default)]
pub struct NoetherUsage {
    /// Stage IDs used across the sprint, with usage count.
    pub stage_counts: HashMap<String, u32>,
    /// Total Noether stage invocations.
    pub total_invocations: u32,
    /// Tasks that used Noether stages.
    pub tasks_using_noether: Vec<String>,
}

#[derive(Debug, Clone)]
pub struct ClarityIssue {
    pub task_id: String,
    pub clarity_score: u8,
    pub blockers: Vec<String>,
}

#[derive(Debug, Clone)]
pub struct DiscoveredDependency {
    pub task_id: String,
    pub description: String,
}

#[derive(Debug, Clone)]
pub struct ToolGap {
    pub task_id: String,
    pub agent_role: String,
    pub description: String,
}

#[derive(Debug, Clone)]
pub struct ReviewLoopIssue {
    pub task_id: String,
    pub cycles: u32,
    pub blockers: Vec<String>,
}

#[derive(Debug, Clone)]
pub struct EfficiencyAnomaly {
    pub task_id: String,
    pub metric: String,
    pub value: f64,
    pub expected: f64,
}

/// Analyze sprint feedback and produce structured findings.
pub fn analyze(feedback: &SprintFeedback) -> RetroAnalysis {
    let mut analysis = RetroAnalysis::default();

    // Clarity analysis: flag tasks with clarity < 5
    for task in &feedback.tasks {
        if task.task_clarity < 5 {
            analysis.clarity_issues.push(ClarityIssue {
                task_id: task.task_id.clone(),
                clarity_score: task.task_clarity,
                blockers: task.blockers.clone(),
            });
        }
    }

    // Dependency discovery: look for "dependency" or "depends" in blockers
    for task in &feedback.tasks {
        for blocker in &task.blockers {
            let lower = blocker.to_lowercase();
            if lower.contains("dependency") || lower.contains("depends")
                || lower.contains("not in the dag") || lower.contains("discovered at runtime")
            {
                analysis.discovered_dependencies.push(DiscoveredDependency {
                    task_id: task.task_id.clone(),
                    description: blocker.clone(),
                });
            }
        }
    }

    // Tool gap analysis: look for "unavailable" or "missing" tool references
    for task in &feedback.tasks {
        for blocker in &task.blockers {
            let lower = blocker.to_lowercase();
            if lower.contains("tool") && (lower.contains("unavailable") || lower.contains("missing")
                || lower.contains("not available") || lower.contains("not configured"))
            {
                analysis.tool_gaps.push(ToolGap {
                    task_id: task.task_id.clone(),
                    agent_role: task.agent_role.clone(),
                    description: blocker.clone(),
                });
            }
        }
    }

    // Review loop analysis: tasks with > 2 review cycles
    for task in &feedback.tasks {
        if task.review_cycles > 2 {
            analysis.review_loop_analysis.push(ReviewLoopIssue {
                task_id: task.task_id.clone(),
                cycles: task.review_cycles,
                blockers: task.blockers.clone(),
            });
        }
    }

    // Token efficiency: flag tasks using > 2x average tokens
    let avg_tokens = if feedback.tasks.is_empty() {
        0.0
    } else {
        feedback.tasks.iter().map(|t| t.tokens_consumed as f64).sum::<f64>()
            / feedback.tasks.len() as f64
    };

    if avg_tokens > 0.0 {
        for task in &feedback.tasks {
            let ratio = task.tokens_consumed as f64 / avg_tokens;
            if ratio > 2.0 {
                analysis.efficiency_anomalies.push(EfficiencyAnomaly {
                    task_id: task.task_id.clone(),
                    metric: "tokens_consumed".into(),
                    value: task.tokens_consumed as f64,
                    expected: avg_tokens,
                });
            }
        }
    }

    // Noether stage usage analysis
    let mut noether_usage = NoetherUsage::default();
    for task in &feedback.tasks {
        let noether_tools: Vec<&String> = task
            .tools_used
            .iter()
            .filter(|t| t.starts_with("noether:"))
            .collect();

        if !noether_tools.is_empty() {
            noether_usage
                .tasks_using_noether
                .push(task.task_id.clone());
            for tool in noether_tools {
                let stage_id = tool.strip_prefix("noether:").unwrap_or(tool);
                *noether_usage
                    .stage_counts
                    .entry(stage_id.to_string())
                    .or_insert(0) += 1;
                noether_usage.total_invocations += 1;
            }
        }
    }
    analysis.noether_usage = noether_usage;

    analysis
}

#[cfg(test)]
mod tests {
    use super::*;
    use caloron_types::feedback::SelfAssessment;

    fn make_feedback(tasks: Vec<TaskFeedback>) -> SprintFeedback {
        SprintFeedback {
            sprint_id: "test".into(),
            tasks,
        }
    }

    fn task(id: &str, clarity: u8, tokens: u64, blockers: Vec<&str>) -> TaskFeedback {
        TaskFeedback {
            task_id: id.into(),
            agent_role: "dev".into(),
            task_clarity: clarity,
            blockers: blockers.into_iter().map(|s| s.to_string()).collect(),
            tools_used: vec![],
            tokens_consumed: tokens,
            time_to_complete_min: 30,
            self_assessment: SelfAssessment::Completed,
            notes: None,
            review_cycles: 0,
            supervisor_interventions: 0,
        }
    }

    #[test]
    fn test_clarity_issues() {
        let fb = make_feedback(vec![
            task("t1", 3, 5000, vec!["Error format not specified"]),
            task("t2", 8, 5000, vec![]),
        ]);

        let analysis = analyze(&fb);
        assert_eq!(analysis.clarity_issues.len(), 1);
        assert_eq!(analysis.clarity_issues[0].task_id, "t1");
        assert_eq!(analysis.clarity_issues[0].clarity_score, 3);
    }

    #[test]
    fn test_discovered_dependencies() {
        let fb = make_feedback(vec![
            task("t1", 6, 5000, vec!["Dependency on issue #38 was not in the DAG"]),
        ]);

        let analysis = analyze(&fb);
        assert_eq!(analysis.discovered_dependencies.len(), 1);
        assert!(analysis.discovered_dependencies[0].description.contains("DAG"));
    }

    #[test]
    fn test_tool_gaps() {
        let fb = make_feedback(vec![
            task("t1", 6, 5000, vec!["Redis tool not available in agent config"]),
        ]);

        let analysis = analyze(&fb);
        assert_eq!(analysis.tool_gaps.len(), 1);
    }

    #[test]
    fn test_token_efficiency_anomaly() {
        let fb = make_feedback(vec![
            task("t1", 7, 5000, vec![]),
            task("t2", 7, 5000, vec![]),
            task("t3", 7, 25000, vec![]), // 5x average
        ]);

        let analysis = analyze(&fb);
        assert_eq!(analysis.efficiency_anomalies.len(), 1);
        assert_eq!(analysis.efficiency_anomalies[0].task_id, "t3");
    }

    #[test]
    fn test_review_loop_analysis() {
        let mut t = task("t1", 7, 5000, vec!["Ambiguous acceptance criteria"]);
        t.review_cycles = 4;

        let fb = make_feedback(vec![t]);
        let analysis = analyze(&fb);
        assert_eq!(analysis.review_loop_analysis.len(), 1);
        assert_eq!(analysis.review_loop_analysis[0].cycles, 4);
    }

    #[test]
    fn test_empty_feedback() {
        let fb = make_feedback(vec![]);
        let analysis = analyze(&fb);
        assert!(analysis.clarity_issues.is_empty());
        assert!(analysis.efficiency_anomalies.is_empty());
    }
}
