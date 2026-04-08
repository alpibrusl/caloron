use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

use super::improvements::Improvement;
use super::kpis::SprintKpis;

/// Persistent store of learnings across sprints.
/// Lives at `caloron-meta/learnings.json`.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct LearningsStore {
    /// KPI history, one entry per sprint (ordered chronologically)
    pub kpi_history: Vec<SprintKpis>,
    /// Improvements that have been applied
    pub applied: Vec<AppliedImprovement>,
    /// Improvements that are pending (generated but not yet applied)
    pub pending: Vec<Improvement>,
    /// Learnings that feed into the PO Agent's context
    pub po_context: Vec<Learning>,
}

/// A learning that the PO Agent should know about when planning the next sprint.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Learning {
    pub sprint_id: String,
    pub category: LearningCategory,
    pub description: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum LearningCategory {
    /// Tasks of this type need more specification
    TaskSpecification,
    /// This dependency pattern should be in DAGs
    DependencyPattern,
    /// This agent role needs adjustment
    AgentAdjustment,
    /// This review pattern causes problems
    ReviewPattern,
    /// General observation
    Observation,
}

/// An improvement that was applied, with tracking.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AppliedImprovement {
    pub improvement: Improvement,
    pub applied_in_sprint: String,
    pub effective: Option<bool>,
}

impl LearningsStore {
    /// Load from file, or create empty if file doesn't exist.
    pub fn load(path: &Path) -> Result<Self> {
        if !path.exists() {
            return Ok(Self::default());
        }

        let content = std::fs::read_to_string(path)
            .with_context(|| format!("Failed to read {}", path.display()))?;

        let store: Self = serde_json::from_str(&content)
            .with_context(|| format!("Failed to parse {}", path.display()))?;

        Ok(store)
    }

    /// Save to file.
    pub fn save(&self, path: &Path) -> Result<()> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }

        let json = serde_json::to_string_pretty(self)
            .context("Failed to serialize learnings")?;

        std::fs::write(path, json)
            .with_context(|| format!("Failed to write {}", path.display()))?;

        Ok(())
    }

    /// Record a sprint's KPIs.
    pub fn record_kpis(&mut self, kpis: SprintKpis) {
        self.kpi_history.push(kpis);
    }

    /// Add pending improvements from a retro.
    pub fn add_pending_improvements(&mut self, improvements: Vec<Improvement>) {
        self.pending.extend(improvements);
    }

    /// Mark an improvement as applied.
    pub fn mark_applied(&mut self, improvement_id: &str, sprint_id: &str) {
        if let Some(idx) = self.pending.iter().position(|i| i.id == improvement_id) {
            let improvement = self.pending.remove(idx);
            self.applied.push(AppliedImprovement {
                improvement,
                applied_in_sprint: sprint_id.to_string(),
                effective: None,
            });
        }
    }

    /// Add a learning for the PO Agent.
    pub fn add_learning(&mut self, sprint_id: &str, category: LearningCategory, description: &str) {
        self.po_context.push(Learning {
            sprint_id: sprint_id.to_string(),
            category,
            description: description.to_string(),
        });
    }

    /// Get the most recent KPIs for comparison.
    pub fn previous_kpis(&self) -> Option<&SprintKpis> {
        self.kpi_history.last()
    }

    /// Generate a context summary for the PO Agent.
    /// This is injected into the PO's system prompt when planning the next sprint.
    pub fn po_agent_context(&self) -> String {
        let mut ctx = String::new();

        if let Some(last_kpis) = self.kpi_history.last() {
            ctx.push_str("## Previous Sprint Performance\n\n");
            ctx.push_str(&format!("- Completion rate: {:.0}%\n", last_kpis.completion_rate * 100.0));
            ctx.push_str(&format!("- Average clarity: {:.1}/10\n", last_kpis.avg_clarity));
            ctx.push_str(&format!("- Average review cycles: {:.1}\n", last_kpis.avg_review_cycles));
            ctx.push_str(&format!("- Interventions per task: {:.1}\n", last_kpis.interventions_per_task));
            ctx.push_str(&format!("- Clean task rate: {:.0}%\n\n", last_kpis.clean_task_pct * 100.0));
        }

        if !self.pending.is_empty() {
            ctx.push_str("## Pending Improvements from Last Retro\n\n");
            for imp in &self.pending {
                ctx.push_str(&format!("- [{}] {}\n", format_priority(&imp.priority), imp.description));
            }
            ctx.push_str("\n");
        }

        if !self.po_context.is_empty() {
            ctx.push_str("## Learnings from Previous Sprints\n\n");
            // Show most recent 10 learnings
            for learning in self.po_context.iter().rev().take(10) {
                ctx.push_str(&format!("- ({}) {}\n", learning.sprint_id, learning.description));
            }
            ctx.push_str("\n");
        }

        ctx
    }

    /// Derive learnings from improvements and KPIs.
    pub fn derive_learnings(
        &mut self,
        sprint_id: &str,
        improvements: &[Improvement],
        kpis: &SprintKpis,
    ) {
        // Clarity issues → task specification learnings
        for imp in improvements {
            match &imp.action {
                super::improvements::ImprovementAction::ImproveTaskTemplate {
                    add_section,
                    reason,
                    ..
                } => {
                    self.add_learning(
                        sprint_id,
                        LearningCategory::TaskSpecification,
                        &format!("Tasks need a '{}' section — agents reported: {}", add_section, reason),
                    );
                }
                super::improvements::ImprovementAction::AddDagDependency {
                    task_pattern,
                    depends_on_pattern,
                    ..
                } => {
                    self.add_learning(
                        sprint_id,
                        LearningCategory::DependencyPattern,
                        &format!("'{}' type tasks depend on: {}", task_pattern, depends_on_pattern),
                    );
                }
                super::improvements::ImprovementAction::ChangeModel {
                    agent_role,
                    ..
                } => {
                    self.add_learning(
                        sprint_id,
                        LearningCategory::AgentAdjustment,
                        &format!("Agent '{}' may need a stronger model (high failure rate)", agent_role),
                    );
                }
                super::improvements::ImprovementAction::AdjustReviewProcess { suggestion } => {
                    self.add_learning(
                        sprint_id,
                        LearningCategory::ReviewPattern,
                        suggestion,
                    );
                }
                _ => {}
            }
        }

        // KPI-derived learnings
        if kpis.avg_clarity < 5.0 {
            self.add_learning(
                sprint_id,
                LearningCategory::Observation,
                &format!(
                    "Sprint clarity was low ({:.1}/10) — PO should invest more time in task specification",
                    kpis.avg_clarity
                ),
            );
        }

        if kpis.review_loop_count > 0 {
            self.add_learning(
                sprint_id,
                LearningCategory::ReviewPattern,
                &format!(
                    "{} tasks had 3+ review cycles — add explicit acceptance criteria to reduce ambiguity",
                    kpis.review_loop_count
                ),
            );
        }
    }
}

fn format_priority(p: &super::improvements::Priority) -> &'static str {
    match p {
        super::improvements::Priority::Critical => "CRITICAL",
        super::improvements::Priority::High => "HIGH",
        super::improvements::Priority::Medium => "MEDIUM",
        super::improvements::Priority::Low => "LOW",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use super::super::kpis;

    fn sample_kpis(sprint_id: &str) -> SprintKpis {
        SprintKpis {
            sprint_id: sprint_id.into(),
            completion_rate: 0.8,
            failure_count: 1,
            blocked_count: 0,
            avg_clarity: 6.5,
            high_clarity_pct: 0.6,
            avg_review_cycles: 1.5,
            review_loop_count: 0,
            total_tokens: 50000,
            avg_tokens_per_task: 12500.0,
            total_time_min: 120,
            avg_time_per_task: 30.0,
            total_interventions: 2,
            interventions_per_task: 0.5,
            clean_task_pct: 0.5,
            agent_metrics: std::collections::HashMap::new(),
        }
    }

    #[test]
    fn test_store_persistence_roundtrip() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("learnings.json");

        let mut store = LearningsStore::default();
        store.record_kpis(sample_kpis("s1"));
        store.add_learning("s1", LearningCategory::Observation, "Test learning");
        store.save(&path).unwrap();

        let loaded = LearningsStore::load(&path).unwrap();
        assert_eq!(loaded.kpi_history.len(), 1);
        assert_eq!(loaded.po_context.len(), 1);
        assert_eq!(loaded.po_context[0].description, "Test learning");
    }

    #[test]
    fn test_load_nonexistent_returns_empty() {
        let store = LearningsStore::load(Path::new("/nonexistent/path.json")).unwrap();
        assert!(store.kpi_history.is_empty());
    }

    #[test]
    fn test_mark_applied() {
        let mut store = LearningsStore::default();
        store.pending.push(Improvement {
            id: "imp-1".into(),
            category: super::super::improvements::ImprovementCategory::Tooling,
            priority: super::super::improvements::Priority::High,
            description: "Add redis".into(),
            action: super::super::improvements::ImprovementAction::AddTool {
                agent_role: "dev".into(),
                tool: "redis".into(),
            },
            evidence: "test".into(),
        });

        assert_eq!(store.pending.len(), 1);
        store.mark_applied("imp-1", "s2");
        assert_eq!(store.pending.len(), 0);
        assert_eq!(store.applied.len(), 1);
        assert_eq!(store.applied[0].applied_in_sprint, "s2");
    }

    #[test]
    fn test_po_agent_context() {
        let mut store = LearningsStore::default();
        store.record_kpis(sample_kpis("s1"));
        store.add_learning("s1", LearningCategory::TaskSpecification,
            "Tasks need an 'API Response Format' section");
        store.pending.push(Improvement {
            id: "imp-1".into(),
            category: super::super::improvements::ImprovementCategory::Tooling,
            priority: super::super::improvements::Priority::High,
            description: "Add redis tool to QA agent".into(),
            action: super::super::improvements::ImprovementAction::AddTool {
                agent_role: "qa".into(),
                tool: "redis".into(),
            },
            evidence: "".into(),
        });

        let ctx = store.po_agent_context();
        assert!(ctx.contains("Previous Sprint Performance"));
        assert!(ctx.contains("80%")); // completion rate
        assert!(ctx.contains("Pending Improvements"));
        assert!(ctx.contains("redis"));
        assert!(ctx.contains("Learnings"));
        assert!(ctx.contains("API Response Format"));
    }

    #[test]
    fn test_derive_learnings() {
        let kpis = SprintKpis {
            avg_clarity: 4.0,
            review_loop_count: 2,
            ..sample_kpis("s1")
        };

        let improvements = vec![
            Improvement {
                id: "imp-1".into(),
                category: super::super::improvements::ImprovementCategory::TaskTemplate,
                priority: super::super::improvements::Priority::High,
                description: "test".into(),
                action: super::super::improvements::ImprovementAction::ImproveTaskTemplate {
                    template: "t1".into(),
                    add_section: "API Response Format".into(),
                    reason: "Error format not specified".into(),
                },
                evidence: "".into(),
            },
        ];

        let mut store = LearningsStore::default();
        store.derive_learnings("s1", &improvements, &kpis);

        // Should have learnings from: template improvement + low clarity + review loops
        assert!(store.po_context.len() >= 3);
        assert!(store.po_context.iter().any(|l| l.description.contains("API Response Format")));
        assert!(store.po_context.iter().any(|l| l.description.contains("clarity was low")));
        assert!(store.po_context.iter().any(|l| l.description.contains("review cycles")));
    }
}
