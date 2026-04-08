use std::collections::HashMap;

use serde::{Deserialize, Serialize};

use super::analyzer::RetroAnalysis;
use super::collector::SprintFeedback;
use super::kpis::{AgentKpis, SprintKpis};

/// A concrete, actionable improvement derived from retro analysis.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Improvement {
    pub id: String,
    pub category: ImprovementCategory,
    pub priority: Priority,
    pub description: String,
    /// What to change, specifically
    pub action: ImprovementAction,
    /// Evidence from the sprint that motivates this
    pub evidence: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum ImprovementCategory {
    AgentDefinition,
    TaskTemplate,
    DagStructure,
    SystemPrompt,
    Tooling,
    ReviewProcess,
}

#[derive(Debug, Clone, PartialEq, PartialOrd, Serialize, Deserialize)]
pub enum Priority {
    Critical,
    High,
    Medium,
    Low,
}

/// Specific action to take.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ImprovementAction {
    /// Add a tool to an agent's definition
    AddTool {
        agent_role: String,
        tool: String,
    },
    /// Add a Nix package to an agent's environment
    AddNixPackage {
        agent_role: String,
        package: String,
    },
    /// Adjust stall threshold for an agent role
    AdjustStallThreshold {
        agent_role: String,
        current_minutes: u32,
        recommended_minutes: u32,
    },
    /// Add a system prompt instruction to an agent
    AddPromptInstruction {
        agent_role: String,
        instruction: String,
    },
    /// Add a missing dependency edge to the DAG
    AddDagDependency {
        task_pattern: String,
        depends_on_pattern: String,
    },
    /// Improve a task template with more detail
    ImproveTaskTemplate {
        template: String,
        add_section: String,
        reason: String,
    },
    /// Change the model for an agent role (e.g., needs stronger reasoning)
    ChangeModel {
        agent_role: String,
        from_model: String,
        to_model: String,
    },
    /// Reduce review cycles by adjusting reviewer instructions
    AdjustReviewProcess {
        suggestion: String,
    },
}

/// Generate improvements from retro analysis, feedback, and KPIs.
pub fn generate_improvements(
    feedback: &SprintFeedback,
    analysis: &RetroAnalysis,
    kpis: &SprintKpis,
) -> Vec<Improvement> {
    let mut improvements = Vec::new();
    let mut id_counter = 0u32;

    let mut next_id = || {
        id_counter += 1;
        format!("imp-{id_counter}")
    };

    // --- Tool gaps → AddTool ---
    for gap in &analysis.tool_gaps {
        let tool = extract_tool_name(&gap.description);
        improvements.push(Improvement {
            id: next_id(),
            category: ImprovementCategory::Tooling,
            priority: Priority::High,
            description: format!(
                "Agent '{}' was missing a required tool during task {}",
                gap.agent_role, gap.task_id
            ),
            action: ImprovementAction::AddTool {
                agent_role: gap.agent_role.clone(),
                tool: tool.clone(),
            },
            evidence: gap.description.clone(),
        });
    }

    // --- Discovered dependencies → AddDagDependency ---
    for dep in &analysis.discovered_dependencies {
        improvements.push(Improvement {
            id: next_id(),
            category: ImprovementCategory::DagStructure,
            priority: Priority::High,
            description: format!(
                "Task {} discovered a runtime dependency not in the DAG",
                dep.task_id
            ),
            action: ImprovementAction::AddDagDependency {
                task_pattern: dep.task_id.clone(),
                depends_on_pattern: dep.description.clone(),
            },
            evidence: dep.description.clone(),
        });
    }

    // --- Clarity issues → ImproveTaskTemplate + AddPromptInstruction ---
    for issue in &analysis.clarity_issues {
        // Group blockers into template improvements
        for blocker in &issue.blockers {
            let section = infer_missing_section(blocker);
            improvements.push(Improvement {
                id: next_id(),
                category: ImprovementCategory::TaskTemplate,
                priority: if issue.clarity_score <= 3 {
                    Priority::Critical
                } else {
                    Priority::Medium
                },
                description: format!(
                    "Task {} had clarity {}/10 — missing information",
                    issue.task_id, issue.clarity_score
                ),
                action: ImprovementAction::ImproveTaskTemplate {
                    template: issue.task_id.clone(),
                    add_section: section,
                    reason: blocker.clone(),
                },
                evidence: blocker.clone(),
            });
        }
    }

    // --- Review loops → AdjustReviewProcess ---
    for loop_issue in &analysis.review_loop_analysis {
        improvements.push(Improvement {
            id: next_id(),
            category: ImprovementCategory::ReviewProcess,
            priority: Priority::Medium,
            description: format!(
                "Task {} went through {} review cycles",
                loop_issue.task_id, loop_issue.cycles
            ),
            action: ImprovementAction::AdjustReviewProcess {
                suggestion: format!(
                    "Add explicit acceptance criteria to task template to reduce ambiguity. \
                     Blocker hints: {:?}",
                    loop_issue.blockers
                ),
            },
            evidence: format!("{} review cycles", loop_issue.cycles),
        });
    }

    // --- Agent-level KPI issues ---
    for (role, agent_kpis) in &kpis.agent_metrics {
        // High intervention rate → maybe needs prompt improvement
        if agent_kpis.interventions > 0 && agent_kpis.tasks_completed > 0 {
            let rate = agent_kpis.interventions as f64 / agent_kpis.tasks_completed as f64;
            if rate > 1.0 {
                improvements.push(Improvement {
                    id: next_id(),
                    category: ImprovementCategory::SystemPrompt,
                    priority: Priority::Medium,
                    description: format!(
                        "Agent '{}' required {:.1} interventions per task",
                        role, rate
                    ),
                    action: ImprovementAction::AddPromptInstruction {
                        agent_role: role.clone(),
                        instruction: "If you are stuck or blocked, post a comment on the issue \
                                      describing what you need before the stall timer triggers."
                            .into(),
                    },
                    evidence: format!(
                        "{} interventions across {} tasks",
                        agent_kpis.interventions, agent_kpis.tasks_completed
                    ),
                });
            }
        }

        // High failure rate → maybe needs a stronger model
        if agent_kpis.tasks_failed > 0 && agent_kpis.tasks_completed > 0 {
            let fail_rate =
                agent_kpis.tasks_failed as f64 / (agent_kpis.tasks_completed + agent_kpis.tasks_failed) as f64;
            if fail_rate > 0.3 {
                improvements.push(Improvement {
                    id: next_id(),
                    category: ImprovementCategory::AgentDefinition,
                    priority: Priority::High,
                    description: format!(
                        "Agent '{}' has a {:.0}% failure rate — may need a stronger model",
                        role,
                        fail_rate * 100.0
                    ),
                    action: ImprovementAction::ChangeModel {
                        agent_role: role.clone(),
                        from_model: "current".into(),
                        to_model: "strong".into(),
                    },
                    evidence: format!(
                        "{} failed out of {} total tasks",
                        agent_kpis.tasks_failed,
                        agent_kpis.tasks_completed + agent_kpis.tasks_failed
                    ),
                });
            }
        }

        // Very high token usage → stall threshold might be too generous
        if agent_kpis.avg_tokens > 30000.0 {
            improvements.push(Improvement {
                id: next_id(),
                category: ImprovementCategory::AgentDefinition,
                priority: Priority::Low,
                description: format!(
                    "Agent '{}' averages {:.0} tokens per task — may benefit from tighter scope",
                    role, agent_kpis.avg_tokens
                ),
                action: ImprovementAction::AddPromptInstruction {
                    agent_role: role.clone(),
                    instruction: "Keep implementations minimal. If a task seems too large, \
                                  comment on the issue suggesting it be split into subtasks."
                        .into(),
                },
                evidence: format!("{:.0} avg tokens", agent_kpis.avg_tokens),
            });
        }
    }

    // Sort by priority
    improvements.sort_by(|a, b| a.priority.partial_cmp(&b.priority).unwrap());

    improvements
}

/// Try to extract a tool name from a blocker description.
fn extract_tool_name(description: &str) -> String {
    let lower = description.to_lowercase();
    // Common patterns: "Redis tool not available", "missing X tool"
    let known = [
        "redis", "postgres", "docker", "kubernetes", "terraform",
        "eslint", "prettier", "jest", "pytest", "cargo",
    ];
    for tool in known {
        if lower.contains(tool) {
            return tool.to_string();
        }
    }
    // Fallback: extract the first capitalized word near "tool"
    description
        .split_whitespace()
        .find(|w| w.chars().next().is_some_and(|c| c.is_uppercase()))
        .unwrap_or("unknown")
        .to_lowercase()
}

/// Infer what section should be added to a task template from a blocker.
fn infer_missing_section(blocker: &str) -> String {
    let lower = blocker.to_lowercase();
    if lower.contains("format") || lower.contains("response") || lower.contains("api") {
        "API Response Format".into()
    } else if lower.contains("config") || lower.contains("environment") || lower.contains("setup") {
        "Environment Setup".into()
    } else if lower.contains("auth") || lower.contains("credential") || lower.contains("token") {
        "Authentication Requirements".into()
    } else if lower.contains("schema") || lower.contains("data") || lower.contains("model") {
        "Data Schema".into()
    } else if lower.contains("dependency") || lower.contains("depends") {
        "Dependencies".into()
    } else {
        "Additional Context".into()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use super::super::analyzer;
    use super::super::collector::TaskFeedback;
    use super::super::kpis;
    use caloron_types::feedback::SelfAssessment;

    fn task(id: &str, role: &str, clarity: u8, tokens: u64, reviews: u32, interventions: u32, assessment: SelfAssessment, blockers: Vec<&str>) -> TaskFeedback {
        TaskFeedback {
            task_id: id.into(),
            agent_role: role.into(),
            task_clarity: clarity,
            blockers: blockers.into_iter().map(|s| s.into()).collect(),
            tools_used: vec![],
            tokens_consumed: tokens,
            time_to_complete_min: 30,
            self_assessment: assessment,
            notes: None,
            review_cycles: reviews,
            supervisor_interventions: interventions,
        }
    }

    #[test]
    fn test_tool_gap_generates_add_tool() {
        let fb = SprintFeedback {
            sprint_id: "s1".into(),
            tasks: vec![task("t1", "dev", 6, 10000, 0, 0, SelfAssessment::Completed,
                vec!["Redis tool not available in agent config"])],
        };
        let analysis = analyzer::analyze(&fb);
        let kpi = kpis::compute_kpis(&fb);
        let improvements = generate_improvements(&fb, &analysis, &kpi);

        let tool_imp = improvements.iter().find(|i| matches!(i.action, ImprovementAction::AddTool { .. }));
        assert!(tool_imp.is_some());
        if let ImprovementAction::AddTool { tool, .. } = &tool_imp.unwrap().action {
            assert_eq!(tool, "redis");
        }
    }

    #[test]
    fn test_clarity_issue_generates_template_improvement() {
        let fb = SprintFeedback {
            sprint_id: "s1".into(),
            tasks: vec![task("t1", "dev", 2, 10000, 0, 0, SelfAssessment::Completed,
                vec!["Error response format not specified"])],
        };
        let analysis = analyzer::analyze(&fb);
        let kpi = kpis::compute_kpis(&fb);
        let improvements = generate_improvements(&fb, &analysis, &kpi);

        let template_imp = improvements.iter().find(|i| matches!(i.action, ImprovementAction::ImproveTaskTemplate { .. }));
        assert!(template_imp.is_some());
        assert_eq!(template_imp.unwrap().priority, Priority::Critical);
    }

    #[test]
    fn test_high_failure_rate_suggests_model_change() {
        let fb = SprintFeedback {
            sprint_id: "s1".into(),
            tasks: vec![
                task("t1", "dev", 7, 10000, 0, 0, SelfAssessment::Completed, vec![]),
                task("t2", "dev", 7, 10000, 0, 0, SelfAssessment::Failed, vec![]),
                task("t3", "dev", 7, 10000, 0, 0, SelfAssessment::Failed, vec![]),
            ],
        };
        let analysis = analyzer::analyze(&fb);
        let kpi = kpis::compute_kpis(&fb);
        let improvements = generate_improvements(&fb, &analysis, &kpi);

        let model_imp = improvements.iter().find(|i| matches!(i.action, ImprovementAction::ChangeModel { .. }));
        assert!(model_imp.is_some(), "Should suggest model change for 66% failure rate");
    }

    #[test]
    fn test_high_intervention_rate_adds_prompt() {
        let fb = SprintFeedback {
            sprint_id: "s1".into(),
            tasks: vec![
                task("t1", "dev", 7, 10000, 0, 2, SelfAssessment::Completed, vec![]),
                task("t2", "dev", 7, 10000, 0, 3, SelfAssessment::Completed, vec![]),
            ],
        };
        let analysis = analyzer::analyze(&fb);
        let kpi = kpis::compute_kpis(&fb);
        let improvements = generate_improvements(&fb, &analysis, &kpi);

        let prompt_imp = improvements.iter().find(|i| {
            matches!(i.action, ImprovementAction::AddPromptInstruction { .. })
                && i.category == ImprovementCategory::SystemPrompt
        });
        assert!(prompt_imp.is_some(), "Should suggest prompt improvement for high intervention rate");
    }

    #[test]
    fn test_review_loop_generates_process_improvement() {
        let fb = SprintFeedback {
            sprint_id: "s1".into(),
            tasks: vec![task("t1", "dev", 5, 10000, 4, 0, SelfAssessment::Completed,
                vec!["Ambiguous acceptance criteria"])],
        };
        let analysis = analyzer::analyze(&fb);
        let kpi = kpis::compute_kpis(&fb);
        let improvements = generate_improvements(&fb, &analysis, &kpi);

        let review_imp = improvements.iter().find(|i| i.category == ImprovementCategory::ReviewProcess);
        assert!(review_imp.is_some());
    }

    #[test]
    fn test_discovered_dep_generates_dag_improvement() {
        let fb = SprintFeedback {
            sprint_id: "s1".into(),
            tasks: vec![task("t1", "dev", 5, 10000, 0, 0, SelfAssessment::Completed,
                vec!["Dependency on issue #38 was not in the DAG"])],
        };
        let analysis = analyzer::analyze(&fb);
        let kpi = kpis::compute_kpis(&fb);
        let improvements = generate_improvements(&fb, &analysis, &kpi);

        let dag_imp = improvements.iter().find(|i| i.category == ImprovementCategory::DagStructure);
        assert!(dag_imp.is_some());
    }

    #[test]
    fn test_improvements_sorted_by_priority() {
        let fb = SprintFeedback {
            sprint_id: "s1".into(),
            tasks: vec![
                task("t1", "dev", 2, 10000, 4, 2, SelfAssessment::Completed,
                    vec!["Error format not specified", "Redis tool not available"]),
            ],
        };
        let analysis = analyzer::analyze(&fb);
        let kpi = kpis::compute_kpis(&fb);
        let improvements = generate_improvements(&fb, &analysis, &kpi);

        // Should be sorted: Critical first, then High, Medium, Low
        for window in improvements.windows(2) {
            assert!(window[0].priority <= window[1].priority);
        }
    }

    #[test]
    fn test_no_improvements_for_clean_sprint() {
        let fb = SprintFeedback {
            sprint_id: "s1".into(),
            tasks: vec![
                task("t1", "dev", 9, 8000, 1, 0, SelfAssessment::Completed, vec![]),
                task("t2", "qa", 8, 6000, 1, 0, SelfAssessment::Completed, vec![]),
            ],
        };
        let analysis = analyzer::analyze(&fb);
        let kpi = kpis::compute_kpis(&fb);
        let improvements = generate_improvements(&fb, &analysis, &kpi);

        assert!(improvements.is_empty(), "Clean sprint should generate no improvements");
    }
}
