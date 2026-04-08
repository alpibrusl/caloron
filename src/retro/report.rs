use std::fmt::Write;
use std::path::Path;

use anyhow::{Context, Result};

use caloron_types::feedback::SelfAssessment;

use super::analyzer::RetroAnalysis;
use super::collector::SprintFeedback;
use super::improvements::Improvement;
use super::kpis::{KpiTrend, SprintKpis, TrendDirection};

/// Generate a markdown retro report from feedback and analysis.
pub fn generate_report(feedback: &SprintFeedback, analysis: &RetroAnalysis) -> String {
    let mut report = String::new();

    // Header
    writeln!(report, "# Sprint Retro — {}\n", feedback.sprint_id).unwrap();

    // Summary
    writeln!(report, "## Summary\n").unwrap();
    let completed = feedback
        .tasks
        .iter()
        .filter(|t| t.self_assessment == SelfAssessment::Completed)
        .count();
    let failed = feedback
        .tasks
        .iter()
        .filter(|t| matches!(t.self_assessment, SelfAssessment::Failed | SelfAssessment::Crashed))
        .count();
    let blocked = feedback
        .tasks
        .iter()
        .filter(|t| t.self_assessment == SelfAssessment::Blocked)
        .count();
    let total_tokens: u64 = feedback.tasks.iter().map(|t| t.tokens_consumed).sum();
    let total_interventions: u32 = feedback.tasks.iter().map(|t| t.supervisor_interventions).sum();
    let avg_clarity = if feedback.tasks.is_empty() {
        0.0
    } else {
        feedback.tasks.iter().map(|t| t.task_clarity as f64).sum::<f64>() / feedback.tasks.len() as f64
    };

    writeln!(report, "- Tasks completed: {completed}/{}", feedback.tasks.len()).unwrap();
    if failed > 0 {
        writeln!(report, "- Tasks failed/crashed: {failed}").unwrap();
    }
    if blocked > 0 {
        writeln!(report, "- Tasks blocked: {blocked}").unwrap();
    }
    writeln!(report, "- Average task clarity: {avg_clarity:.1}/10").unwrap();
    writeln!(report, "- Total tokens consumed: {total_tokens}").unwrap();
    writeln!(report, "- Supervisor interventions: {total_interventions}").unwrap();
    writeln!(report).unwrap();

    // Critical issues
    if !analysis.clarity_issues.is_empty()
        || !analysis.tool_gaps.is_empty()
    {
        writeln!(report, "## Critical Issues\n").unwrap();

        for issue in &analysis.clarity_issues {
            writeln!(
                report,
                "### Low clarity: {} (score: {}/10)\n",
                issue.task_id, issue.clarity_score
            )
            .unwrap();
            if !issue.blockers.is_empty() {
                writeln!(report, "Reported blockers:").unwrap();
                for b in &issue.blockers {
                    writeln!(report, "- \"{b}\"").unwrap();
                }
                writeln!(report).unwrap();
            }
        }

        for gap in &analysis.tool_gaps {
            writeln!(
                report,
                "### Tool gap: {} (agent: {})\n",
                gap.task_id, gap.agent_role
            )
            .unwrap();
            writeln!(report, "{}\n", gap.description).unwrap();
        }
    }

    // Discovered dependencies
    if !analysis.discovered_dependencies.is_empty() {
        writeln!(report, "## DAG Improvements\n").unwrap();
        for dep in &analysis.discovered_dependencies {
            writeln!(report, "- **{}**: {}", dep.task_id, dep.description).unwrap();
        }
        writeln!(report).unwrap();
    }

    // Review loops
    if !analysis.review_loop_analysis.is_empty() {
        writeln!(report, "## Review Loop Issues\n").unwrap();
        for loop_issue in &analysis.review_loop_analysis {
            writeln!(
                report,
                "- **{}**: {} review cycles",
                loop_issue.task_id, loop_issue.cycles
            )
            .unwrap();
        }
        writeln!(report).unwrap();
    }

    // Efficiency
    if !analysis.efficiency_anomalies.is_empty() {
        writeln!(report, "## Efficiency Anomalies\n").unwrap();
        for anomaly in &analysis.efficiency_anomalies {
            writeln!(
                report,
                "- **{}**: {} = {:.0} (expected ~{:.0})",
                anomaly.task_id, anomaly.metric, anomaly.value, anomaly.expected
            )
            .unwrap();
        }
        writeln!(report).unwrap();
    }

    // What worked
    let well_rated: Vec<_> = feedback
        .tasks
        .iter()
        .filter(|t| t.task_clarity >= 7 && t.self_assessment == SelfAssessment::Completed)
        .collect();

    if !well_rated.is_empty() {
        writeln!(report, "## What Worked Well\n").unwrap();
        for task in well_rated {
            writeln!(
                report,
                "- {} completed successfully (clarity: {}/10, {} tokens, {}min)",
                task.task_id, task.task_clarity, task.tokens_consumed, task.time_to_complete_min
            )
            .unwrap();
        }
        writeln!(report).unwrap();
    }

    // Noether usage
    let nu = &analysis.noether_usage;
    if nu.total_invocations > 0 {
        writeln!(report, "## Noether Usage\n").unwrap();
        writeln!(
            report,
            "- Tasks using Noether: {}/{}",
            nu.tasks_using_noether.len(),
            feedback.tasks.len()
        )
        .unwrap();
        writeln!(report, "- Total stage invocations: {}", nu.total_invocations).unwrap();
        writeln!(report, "- Unique stages used: {}", nu.stage_counts.len()).unwrap();

        if !nu.stage_counts.is_empty() {
            writeln!(report, "\nMost used stages:").unwrap();
            let mut sorted: Vec<_> = nu.stage_counts.iter().collect();
            sorted.sort_by(|a, b| b.1.cmp(a.1));
            for (stage, count) in sorted.iter().take(5) {
                writeln!(report, "- `{stage}` ({count} uses)").unwrap();
            }
        }
        writeln!(report).unwrap();
    }

    writeln!(
        report,
        "---\n*Generated by Caloron Retro Engine*"
    )
    .unwrap();

    report
}

/// Generate an extended report including KPIs, trends, and actionable improvements.
pub fn generate_full_report(
    feedback: &SprintFeedback,
    analysis: &RetroAnalysis,
    kpis: &SprintKpis,
    trends: Option<&[KpiTrend]>,
    improvements: &[Improvement],
) -> String {
    // Start with the base report (without the footer)
    let mut report = generate_report(feedback, analysis);
    // Remove the trailing "---\n*Generated..." line to append more sections
    if let Some(pos) = report.rfind("---\n*Generated") {
        report.truncate(pos);
    }

    // KPI Dashboard
    writeln!(report, "## KPI Dashboard\n").unwrap();
    writeln!(report, "| Metric | Value |").unwrap();
    writeln!(report, "|--------|-------|").unwrap();
    writeln!(report, "| Completion rate | {:.0}% |", kpis.completion_rate * 100.0).unwrap();
    writeln!(report, "| Average clarity | {:.1}/10 |", kpis.avg_clarity).unwrap();
    writeln!(report, "| High clarity tasks (>=7) | {:.0}% |", kpis.high_clarity_pct * 100.0).unwrap();
    writeln!(report, "| Average review cycles | {:.1} |", kpis.avg_review_cycles).unwrap();
    writeln!(report, "| Average tokens/task | {:.0} |", kpis.avg_tokens_per_task).unwrap();
    writeln!(report, "| Average time/task | {:.0} min |", kpis.avg_time_per_task).unwrap();
    writeln!(report, "| Interventions/task | {:.1} |", kpis.interventions_per_task).unwrap();
    writeln!(report, "| Clean task rate | {:.0}% |", kpis.clean_task_pct * 100.0).unwrap();
    writeln!(report).unwrap();

    // Per-agent breakdown
    if !kpis.agent_metrics.is_empty() {
        writeln!(report, "### Agent Performance\n").unwrap();
        writeln!(report, "| Agent | Completed | Failed | Clarity | Tokens | Time | Reviews | Interventions |").unwrap();
        writeln!(report, "|-------|-----------|--------|---------|--------|------|---------|---------------|").unwrap();
        let mut agents: Vec<_> = kpis.agent_metrics.iter().collect();
        agents.sort_by_key(|(k, _)| k.clone());
        for (role, a) in agents {
            writeln!(
                report,
                "| {role} | {} | {} | {:.1} | {:.0} | {:.0}m | {:.1} | {} |",
                a.tasks_completed, a.tasks_failed, a.avg_clarity,
                a.avg_tokens, a.avg_time_min, a.avg_review_cycles, a.interventions
            )
            .unwrap();
        }
        writeln!(report).unwrap();
    }

    // Trends (vs previous sprint)
    if let Some(trends) = trends {
        writeln!(report, "## Trends vs Previous Sprint\n").unwrap();
        for t in trends {
            let arrow = match t.direction {
                TrendDirection::Improved => "^",
                TrendDirection::Degraded => "v",
                TrendDirection::Stable => "=",
            };
            writeln!(
                report,
                "- {arrow} **{}**: {:.1} -> {:.1} ({:+.0}%)",
                t.name, t.previous, t.current, t.change_pct
            )
            .unwrap();
        }
        writeln!(report).unwrap();
    }

    // Actionable improvements
    if !improvements.is_empty() {
        writeln!(report, "## Actionable Improvements\n").unwrap();
        for imp in improvements {
            let priority = match imp.priority {
                super::improvements::Priority::Critical => "CRITICAL",
                super::improvements::Priority::High => "HIGH",
                super::improvements::Priority::Medium => "MEDIUM",
                super::improvements::Priority::Low => "LOW",
            };
            writeln!(report, "### [{priority}] {}\n", imp.description).unwrap();
            writeln!(report, "**Action:** {}\n", format_action(&imp.action)).unwrap();
            writeln!(report, "**Evidence:** {}\n", imp.evidence).unwrap();
        }
    }

    writeln!(report, "---\n*Generated by Caloron Retro Engine*").unwrap();

    report
}

fn format_action(action: &super::improvements::ImprovementAction) -> String {
    match action {
        super::improvements::ImprovementAction::AddTool { agent_role, tool } => {
            format!("Add tool `{tool}` to agent `{agent_role}`")
        }
        super::improvements::ImprovementAction::AddNixPackage { agent_role, package } => {
            format!("Add Nix package `{package}` to agent `{agent_role}`")
        }
        super::improvements::ImprovementAction::AdjustStallThreshold {
            agent_role,
            current_minutes,
            recommended_minutes,
        } => {
            format!(
                "Change stall threshold for `{agent_role}` from {current_minutes}min to {recommended_minutes}min"
            )
        }
        super::improvements::ImprovementAction::AddPromptInstruction {
            agent_role,
            instruction,
        } => {
            format!("Add to `{agent_role}` system prompt: \"{instruction}\"")
        }
        super::improvements::ImprovementAction::AddDagDependency {
            task_pattern,
            depends_on_pattern,
        } => {
            format!("Add DAG dependency: `{task_pattern}` should depend on `{depends_on_pattern}`")
        }
        super::improvements::ImprovementAction::ImproveTaskTemplate {
            template,
            add_section,
            ..
        } => {
            format!("Add '{add_section}' section to task template `{template}`")
        }
        super::improvements::ImprovementAction::ChangeModel {
            agent_role,
            to_model,
            ..
        } => {
            format!("Switch agent `{agent_role}` to model `{to_model}`")
        }
        super::improvements::ImprovementAction::AdjustReviewProcess { suggestion } => {
            format!("Review process: {suggestion}")
        }
    }
}

/// Write a retro report to file.
pub fn write_report(report: &str, path: &Path) -> Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::write(path, report)
        .with_context(|| format!("Failed to write retro report to {}", path.display()))?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use super::super::analyzer;
    use super::super::collector::{SprintFeedback, TaskFeedback};

    fn sample_feedback() -> SprintFeedback {
        SprintFeedback {
            sprint_id: "sprint-test".into(),
            tasks: vec![
                TaskFeedback {
                    task_id: "t1".into(), agent_role: "dev".into(), task_clarity: 8,
                    blockers: vec![], tools_used: vec!["bash".into()], tokens_consumed: 10000,
                    time_to_complete_min: 30, self_assessment: SelfAssessment::Completed,
                    notes: None, review_cycles: 1, supervisor_interventions: 0,
                },
                TaskFeedback {
                    task_id: "t2".into(), agent_role: "dev".into(), task_clarity: 3,
                    blockers: vec!["Error format not specified".into(), "Dependency on #38 was not in the DAG".into()],
                    tools_used: vec!["bash".into()], tokens_consumed: 25000,
                    time_to_complete_min: 90, self_assessment: SelfAssessment::Completed,
                    notes: None, review_cycles: 3, supervisor_interventions: 2,
                },
                TaskFeedback {
                    task_id: "t3".into(), agent_role: "qa".into(), task_clarity: 5,
                    blockers: vec!["Redis tool not available in agent config".into()],
                    tools_used: vec!["bash".into()], tokens_consumed: 8000,
                    time_to_complete_min: 45, self_assessment: SelfAssessment::Failed,
                    notes: None, review_cycles: 0, supervisor_interventions: 1,
                },
            ],
        }
    }

    #[test]
    fn test_generate_report_contains_sections() {
        let fb = sample_feedback();
        let analysis = analyzer::analyze(&fb);
        let report = generate_report(&fb, &analysis);

        assert!(report.contains("# Sprint Retro — sprint-test"));
        assert!(report.contains("Tasks completed: 2/3"));
        assert!(report.contains("Tasks failed/crashed: 1"));
        assert!(report.contains("Average task clarity:"));
        assert!(report.contains("Critical Issues"));
        assert!(report.contains("DAG Improvements"));
        assert!(report.contains("What Worked Well"));
    }

    #[test]
    fn test_report_clarity_issues() {
        let fb = sample_feedback();
        let analysis = analyzer::analyze(&fb);
        let report = generate_report(&fb, &analysis);

        assert!(report.contains("Low clarity: t2"));
        assert!(report.contains("Error format not specified"));
    }

    #[test]
    fn test_report_tool_gaps() {
        let fb = sample_feedback();
        let analysis = analyzer::analyze(&fb);
        let report = generate_report(&fb, &analysis);

        assert!(report.contains("Tool gap: t3"));
        assert!(report.contains("Redis tool"));
    }

    #[test]
    fn test_write_report_creates_file() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("retro").join("sprint-test.md");

        write_report("# Test report", &path).unwrap();
        assert!(path.exists());
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "# Test report");
    }
}
