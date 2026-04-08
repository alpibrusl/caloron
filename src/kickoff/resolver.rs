use std::collections::HashMap;
use std::path::Path;

use anyhow::{Context, Result};

use caloron_types::agent::AgentDefinition;
use caloron_types::agent_gen::{AgentOverrides, AgentRegistry, AgentSpec};
use caloron_types::dag::{AgentRoleSpec, Dag};

use crate::agent::registry::default_registry;

/// Resolve all agent specs in a DAG into full AgentDefinitions.
///
/// For each agent that has a `spec` field, the registry generates
/// a complete definition. Agents with only a `definition_path` are
/// loaded from disk. Returns a map of agent_id → AgentDefinition.
pub fn resolve_agents(
    dag: &Dag,
    project_root: &Path,
) -> Result<HashMap<String, AgentDefinition>> {
    let registry = default_registry();
    let mut definitions = HashMap::new();

    for agent_node in &dag.agents {
        let def = if let Some(spec) = &agent_node.spec {
            // Generate from spec via the 4-axis registry
            resolve_from_spec(&agent_node.id, spec, &registry)?
        } else if !agent_node.definition_path.to_string_lossy().is_empty() {
            // Load from YAML file
            let path = project_root.join(&agent_node.definition_path);
            if path.exists() {
                let content = std::fs::read_to_string(&path)
                    .with_context(|| format!("Failed to read {}", path.display()))?;
                serde_yaml::from_str(&content)
                    .with_context(|| format!("Failed to parse {}", path.display()))?
            } else {
                // Fallback to generated default
                tracing::warn!(
                    agent_id = agent_node.id,
                    path = %path.display(),
                    "Definition file not found — generating from role"
                );
                resolve_default(&agent_node.id, &agent_node.role, &registry)?
            }
        } else {
            // No spec and no path — generate from role name
            resolve_default(&agent_node.id, &agent_node.role, &registry)?
        };

        tracing::info!(
            agent_id = agent_node.id,
            personality = def.name,
            model = def.llm.model,
            tools = ?def.tools,
            "Resolved agent definition"
        );

        definitions.insert(agent_node.id.clone(), def);
    }

    Ok(definitions)
}

/// Generate an AgentDefinition from an AgentRoleSpec using the registry.
fn resolve_from_spec(
    agent_id: &str,
    spec: &AgentRoleSpec,
    registry: &AgentRegistry,
) -> Result<AgentDefinition> {
    let agent_spec = AgentSpec {
        name: agent_id.to_string(),
        personality: spec.personality.clone(),
        capabilities: spec.capabilities.clone(),
        model: spec.model.clone(),
        framework: spec.framework.clone(),
        extra_instructions: spec.extra_instructions.clone(),
        overrides: AgentOverrides::default(),
    };

    registry
        .generate(&agent_spec)
        .map_err(|e| anyhow::anyhow!("Failed to resolve agent '{}': {}", agent_id, e))
}

/// Fallback: generate a sensible default from a role name.
fn resolve_default(
    agent_id: &str,
    role: &str,
    registry: &AgentRegistry,
) -> Result<AgentDefinition> {
    // Map common role names to personalities
    let personality = match role {
        "developer" | "backend-developer" | "frontend-developer" | "data-scientist" => "developer",
        "qa" | "qa-engineer" | "tester" => "qa",
        "reviewer" | "senior-reviewer" | "code-reviewer" => "reviewer",
        "architect" | "tech-lead" => "architect",
        "designer" | "ui-designer" | "ux-designer" => "designer",
        "ux-researcher" | "researcher" => "ux-researcher",
        "devops" | "sre" | "infra" => "devops",
        _ => "developer", // safe fallback
    };

    // Default capabilities based on personality
    let capabilities = match personality {
        "developer" => vec!["code-writing".into(), "testing".into(), "python".into()],
        "qa" => vec!["code-writing".into(), "testing".into()],
        "reviewer" => vec!["code-writing".into()],
        "architect" => vec!["code-writing".into(), "browser-research".into()],
        "designer" => vec!["code-writing".into(), "frontend".into()],
        "ux-researcher" => vec!["browser-research".into()],
        "devops" => vec!["code-writing".into(), "testing".into()],
        _ => vec!["code-writing".into()],
    };

    let agent_spec = AgentSpec {
        name: agent_id.to_string(),
        personality: personality.to_string(),
        capabilities,
        model: "balanced".into(),
        framework: "claude-code".into(),
        extra_instructions: None,
        overrides: AgentOverrides::default(),
    };

    registry
        .generate(&agent_spec)
        .map_err(|e| anyhow::anyhow!("Failed to generate default agent '{}': {}", agent_id, e))
}

/// Print a summary of resolved agents.
pub fn print_agent_summary(agents: &HashMap<String, AgentDefinition>) {
    println!("Agents ({}):", agents.len());
    let mut sorted: Vec<_> = agents.iter().collect();
    sorted.sort_by_key(|(id, _)| id.clone());

    for (id, def) in sorted {
        let tools: Vec<&str> = def.tools.iter().map(|s| s.as_str()).collect();
        println!(
            "  {id:<16} {:<16} model={:<20} tools=[{}]",
            def.name,
            def.llm.model,
            tools.join(", ")
        );
        if let Some(extra) = &def.system_prompt.lines().last() {
            if !extra.is_empty() && def.system_prompt.lines().count() > 5 {
                // Show hint of extra instructions if any
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use caloron_types::dag::*;
    use chrono::Utc;

    fn make_dag_with_specs() -> Dag {
        Dag {
            sprint: Sprint {
                id: "test".into(),
                goal: "Test".into(),
                start: Utc::now(),
                max_duration_hours: 24,
            },
            agents: vec![
                AgentNode {
                    id: "dev-1".into(),
                    role: "developer".into(),
                    definition_path: "".into(),
                    spec: Some(AgentRoleSpec {
                        personality: "developer".into(),
                        capabilities: vec!["code-writing".into(), "python".into(), "testing".into()],
                        model: "balanced".into(),
                        framework: "claude-code".into(),
                        extra_instructions: Some("Use FastAPI for the API.".into()),
                    }),
                },
                AgentNode {
                    id: "ds-1".into(),
                    role: "data-scientist".into(),
                    definition_path: "".into(),
                    spec: Some(AgentRoleSpec {
                        personality: "developer".into(),
                        capabilities: vec!["code-writing".into(), "python".into(), "testing".into()],
                        model: "strong".into(),
                        framework: "claude-code".into(),
                        extra_instructions: Some("You are a data scientist. Use pandas, scikit-learn, XGBoost.".into()),
                    }),
                },
                AgentNode {
                    id: "qa-1".into(),
                    role: "qa".into(),
                    definition_path: "".into(),
                    spec: Some(AgentRoleSpec {
                        personality: "qa".into(),
                        capabilities: vec!["code-writing".into(), "testing".into(), "python".into()],
                        model: "balanced".into(),
                        framework: "claude-code".into(),
                        extra_instructions: None,
                    }),
                },
                AgentNode {
                    id: "rev-1".into(),
                    role: "reviewer".into(),
                    definition_path: "".into(),
                    spec: Some(AgentRoleSpec {
                        personality: "reviewer".into(),
                        capabilities: vec!["code-writing".into(), "python".into()],
                        model: "strong".into(),
                        framework: "claude-code".into(),
                        extra_instructions: None,
                    }),
                },
            ],
            tasks: vec![
                Task {
                    id: "t1".into(),
                    title: "Build API".into(),
                    assigned_to: "dev-1".into(),
                    issue_template: "t.md".into(),
                    depends_on: vec![],
                    reviewed_by: Some("rev-1".into()),
                    github_issue_number: None,
                },
            ],
            review_policy: ReviewPolicy { required_approvals: 1, auto_merge: true, max_review_cycles: 3 },
            escalation: EscalationConfig { stall_threshold_minutes: 20, supervisor_id: "sup".into(), human_contact: "gh".into() },
        }
    }

    #[test]
    fn test_resolve_from_specs() {
        let dag = make_dag_with_specs();
        let agents = resolve_agents(&dag, std::path::Path::new("/tmp")).unwrap();

        assert_eq!(agents.len(), 4);

        // dev-1 should have python, code-writing, testing
        let dev = &agents["dev-1"];
        assert!(dev.tools.contains(&"bash".into()));
        assert!(dev.tools.contains(&"github_mcp".into()));
        assert_eq!(dev.llm.model, "claude-sonnet-4-6");
        assert!(dev.system_prompt.contains("FastAPI"));

        // ds-1 should have strong model
        let ds = &agents["ds-1"];
        assert_eq!(ds.llm.model, "claude-opus-4-6");
        assert!(ds.system_prompt.contains("data scientist"));

        // qa-1 should have qa personality
        let qa = &agents["qa-1"];
        assert!(qa.system_prompt.contains("QA"));
        assert_eq!(qa.stall_threshold_minutes, 25); // qa personality default

        // rev-1 should have reviewer personality with strong model
        let rev = &agents["rev-1"];
        assert!(rev.system_prompt.contains("code reviewer"));
        assert_eq!(rev.llm.model, "claude-opus-4-6");
    }

    #[test]
    fn test_resolve_default_from_role() {
        let registry = default_registry();

        let def = resolve_default("test-1", "backend-developer", &registry).unwrap();
        assert!(def.system_prompt.contains("developer"));

        let def = resolve_default("test-2", "qa-engineer", &registry).unwrap();
        assert!(def.system_prompt.contains("QA"));

        let def = resolve_default("test-3", "unknown-role", &registry).unwrap();
        // Falls back to developer
        assert!(def.system_prompt.contains("developer"));
    }

    #[test]
    fn test_resolve_with_extra_instructions() {
        let registry = default_registry();
        let spec = AgentRoleSpec {
            personality: "developer".into(),
            capabilities: vec!["code-writing".into(), "python".into()],
            model: "balanced".into(),
            framework: "claude-code".into(),
            extra_instructions: Some("Always use type hints. Use pytest for testing.".into()),
        };

        let def = resolve_from_spec("dev-typed", &spec, &registry).unwrap();
        assert!(def.system_prompt.contains("type hints"));
        assert!(def.system_prompt.contains("pytest"));
    }

    #[test]
    fn test_dag_with_specs_serialization() {
        let dag = make_dag_with_specs();
        let json = serde_json::to_string_pretty(&dag).unwrap();

        // Should contain spec fields
        assert!(json.contains("\"personality\""));
        assert!(json.contains("\"capabilities\""));
        assert!(json.contains("\"extra_instructions\""));

        // Roundtrip
        let restored: Dag = serde_json::from_str(&json).unwrap();
        assert!(restored.agents[0].spec.is_some());
        assert_eq!(
            restored.agents[0].spec.as_ref().unwrap().personality,
            "developer"
        );
    }
}
