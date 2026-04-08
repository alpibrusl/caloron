use std::collections::HashMap;

use serde::{Deserialize, Serialize};

use crate::agent::{AgentDefinition, LlmConfig, McpConfig, NixConfig};

// =============================================================================
// Axis 1: Personality — who the agent is, how it behaves
// =============================================================================

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Personality {
    /// Role identifier (e.g., "developer", "qa", "architect")
    pub role: String,
    /// Human-readable description
    pub description: String,
    /// The system prompt defining behavior
    pub system_prompt: String,
    /// Stall threshold override (reviewers/researchers are naturally slower)
    #[serde(default)]
    pub stall_threshold_minutes: Option<u32>,
    /// Max review cycles override
    #[serde(default)]
    pub max_review_cycles: Option<u32>,
}

// =============================================================================
// Axis 2: Capabilities — what the agent can do
// =============================================================================

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CapabilityBundle {
    /// Bundle identifier (e.g., "code-writing", "testing", "browser-research")
    pub name: String,
    pub description: String,
    /// Tools this bundle provides
    #[serde(default)]
    pub tools: Vec<String>,
    /// MCP servers this bundle requires
    #[serde(default)]
    pub mcps: Vec<McpConfig>,
    /// Nix packages this bundle requires
    #[serde(default)]
    pub nix_packages: Vec<String>,
    /// Environment variables this bundle sets
    #[serde(default)]
    pub env: HashMap<String, String>,
    /// Credentials this bundle requires
    #[serde(default)]
    pub credentials: Vec<String>,
}

// =============================================================================
// Axis 3: Model — which LLM and configuration
// =============================================================================

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ModelConfig {
    /// Model identifier or alias (e.g., "default", "claude-sonnet-4-6")
    pub model: String,
    /// Max output tokens
    #[serde(default = "default_max_tokens")]
    pub max_tokens: u32,
    /// Temperature (0.0-2.0)
    #[serde(default = "default_temperature")]
    pub temperature: f32,
}

fn default_max_tokens() -> u32 {
    8192
}

fn default_temperature() -> f32 {
    0.2
}

// =============================================================================
// Axis 4: Framework — which harness/CLI runs the agent
// =============================================================================

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Framework {
    /// Framework identifier
    pub name: String,
    pub description: String,
    /// The command to invoke (e.g., "claude-code", "gemini-cli", "aider")
    pub command: String,
    /// Arguments to pass to the command
    #[serde(default)]
    pub args: Vec<String>,
    /// Additional Nix packages the framework itself requires
    #[serde(default)]
    pub nix_packages: Vec<String>,
    /// Environment variables the framework needs
    #[serde(default)]
    pub env: HashMap<String, String>,
    /// Which credential env vars the framework requires
    #[serde(default)]
    pub credentials: Vec<String>,
    /// Whether this framework supports the caloron harness heartbeat protocol
    #[serde(default = "default_true")]
    pub harness_compatible: bool,
}

fn default_true() -> bool {
    true
}

// =============================================================================
// Agent Spec — the composition of all four axes
// =============================================================================

/// A compact spec that references the four axes by name.
/// Used to generate a full AgentDefinition.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentSpec {
    /// Instance name for this agent (e.g., "backend-1")
    pub name: String,
    /// Which personality to use
    pub personality: String,
    /// Which capability bundles to include (merged)
    pub capabilities: Vec<String>,
    /// Which model config to use
    pub model: String,
    /// Which framework to use
    pub framework: String,
    /// Extra system prompt lines appended to the personality prompt
    #[serde(default)]
    pub extra_instructions: Option<String>,
    /// Override any field
    #[serde(default)]
    pub overrides: AgentOverrides,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct AgentOverrides {
    #[serde(default)]
    pub stall_threshold_minutes: Option<u32>,
    #[serde(default)]
    pub max_review_cycles: Option<u32>,
    #[serde(default)]
    pub extra_tools: Vec<String>,
    #[serde(default)]
    pub extra_nix_packages: Vec<String>,
    #[serde(default)]
    pub extra_credentials: Vec<String>,
    #[serde(default)]
    pub extra_env: HashMap<String, String>,
}

// =============================================================================
// Registry — holds all available personalities, capabilities, models, frameworks
// =============================================================================

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct AgentRegistry {
    pub personalities: HashMap<String, Personality>,
    pub capabilities: HashMap<String, CapabilityBundle>,
    pub models: HashMap<String, ModelConfig>,
    pub frameworks: HashMap<String, Framework>,
}

impl AgentRegistry {
    /// Generate a full AgentDefinition from an AgentSpec.
    pub fn generate(&self, spec: &AgentSpec) -> Result<AgentDefinition, String> {
        // Look up personality
        let personality = self
            .personalities
            .get(&spec.personality)
            .ok_or_else(|| format!("Unknown personality: {}", spec.personality))?;

        // Look up model
        let model = self
            .models
            .get(&spec.model)
            .ok_or_else(|| format!("Unknown model: {}", spec.model))?;

        // Look up framework
        let framework = self
            .frameworks
            .get(&spec.framework)
            .ok_or_else(|| format!("Unknown framework: {}", spec.framework))?;

        // Merge capability bundles
        let mut tools = Vec::new();
        let mut mcps = Vec::new();
        let mut nix_packages = Vec::new();
        let mut env = HashMap::new();
        let mut credentials = Vec::new();

        for cap_name in &spec.capabilities {
            let cap = self
                .capabilities
                .get(cap_name)
                .ok_or_else(|| format!("Unknown capability: {cap_name}"))?;
            tools.extend(cap.tools.clone());
            mcps.extend(cap.mcps.clone());
            nix_packages.extend(cap.nix_packages.clone());
            env.extend(cap.env.clone());
            credentials.extend(cap.credentials.clone());
        }

        // Add framework requirements
        nix_packages.extend(framework.nix_packages.clone());
        env.extend(framework.env.clone());
        credentials.extend(framework.credentials.clone());

        // Apply overrides
        tools.extend(spec.overrides.extra_tools.clone());
        nix_packages.extend(spec.overrides.extra_nix_packages.clone());
        credentials.extend(spec.overrides.extra_credentials.clone());
        env.extend(spec.overrides.extra_env.clone());

        // Deduplicate
        tools.sort();
        tools.dedup();
        nix_packages.sort();
        nix_packages.dedup();
        credentials.sort();
        credentials.dedup();

        // Build system prompt
        let mut system_prompt = personality.system_prompt.clone();
        if let Some(extra) = &spec.extra_instructions {
            system_prompt.push_str("\n\n");
            system_prompt.push_str(extra);
        }

        // Stall threshold: override > personality > default
        let stall_threshold = spec
            .overrides
            .stall_threshold_minutes
            .or(personality.stall_threshold_minutes)
            .unwrap_or(20);

        let max_review_cycles = spec
            .overrides
            .max_review_cycles
            .or(personality.max_review_cycles)
            .unwrap_or(3);

        Ok(AgentDefinition {
            name: spec.name.clone(),
            version: "1.0".into(),
            description: personality.description.clone(),
            llm: LlmConfig {
                model: model.model.clone(),
                max_tokens: model.max_tokens,
                temperature: model.temperature,
            },
            system_prompt,
            tools,
            mcps,
            nix: NixConfig {
                packages: nix_packages,
                env,
            },
            credentials,
            stall_threshold_minutes: stall_threshold,
            max_review_cycles,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_registry() -> AgentRegistry {
        let mut reg = AgentRegistry::default();

        reg.personalities.insert(
            "developer".into(),
            Personality {
                role: "developer".into(),
                description: "Implements features and writes tests".into(),
                system_prompt: "You are a senior software developer.\nYou receive tasks as GitHub issues.".into(),
                stall_threshold_minutes: Some(20),
                max_review_cycles: None,
            },
        );

        reg.personalities.insert(
            "qa".into(),
            Personality {
                role: "qa".into(),
                description: "Writes and runs tests".into(),
                system_prompt: "You are a QA engineer.\nYou write comprehensive tests.".into(),
                stall_threshold_minutes: Some(25),
                max_review_cycles: None,
            },
        );

        reg.personalities.insert(
            "architect".into(),
            Personality {
                role: "architect".into(),
                description: "Reviews architecture and system design".into(),
                system_prompt: "You are a software architect.\nYou review designs for scalability and correctness.".into(),
                stall_threshold_minutes: Some(30),
                max_review_cycles: Some(2),
            },
        );

        reg.capabilities.insert(
            "code-writing".into(),
            CapabilityBundle {
                name: "code-writing".into(),
                description: "Write and modify code".into(),
                tools: vec!["bash".into(), "github_mcp".into()],
                mcps: vec![McpConfig {
                    url: "https://github.mcp.claude.com/mcp".into(),
                    name: "github".into(),
                }],
                nix_packages: vec!["git".into()],
                env: HashMap::new(),
                credentials: vec!["GITHUB_TOKEN".into()],
            },
        );

        reg.capabilities.insert(
            "testing".into(),
            CapabilityBundle {
                name: "testing".into(),
                description: "Run tests and linters".into(),
                tools: vec!["bash".into()],
                mcps: vec![],
                nix_packages: vec![],
                env: HashMap::from([("NODE_ENV".into(), "test".into())]),
                credentials: vec![],
            },
        );

        reg.capabilities.insert(
            "browser-research".into(),
            CapabilityBundle {
                name: "browser-research".into(),
                description: "Browse the web for research".into(),
                tools: vec!["browser".into()],
                mcps: vec![],
                nix_packages: vec!["chromium".into()],
                env: HashMap::new(),
                credentials: vec![],
            },
        );

        reg.capabilities.insert(
            "noether".into(),
            CapabilityBundle {
                name: "noether".into(),
                description: "Verified computation via Noether stages".into(),
                tools: vec!["noether".into()],
                mcps: vec![McpConfig {
                    url: "http://localhost:8080/mcp".into(),
                    name: "noether".into(),
                }],
                nix_packages: vec![],
                env: HashMap::new(),
                credentials: vec![],
            },
        );

        reg.models.insert(
            "balanced".into(),
            ModelConfig {
                model: "claude-sonnet-4-6".into(),
                max_tokens: 8192,
                temperature: 0.2,
            },
        );

        reg.models.insert(
            "strong".into(),
            ModelConfig {
                model: "claude-opus-4-6".into(),
                max_tokens: 16384,
                temperature: 0.1,
            },
        );

        reg.models.insert(
            "fast".into(),
            ModelConfig {
                model: "claude-haiku-4-5".into(),
                max_tokens: 4096,
                temperature: 0.3,
            },
        );

        reg.models.insert(
            "gemini".into(),
            ModelConfig {
                model: "gemini-2.5-pro".into(),
                max_tokens: 8192,
                temperature: 0.2,
            },
        );

        reg.frameworks.insert(
            "claude-code".into(),
            Framework {
                name: "claude-code".into(),
                description: "Anthropic Claude Code CLI".into(),
                command: "claude".into(),
                args: vec!["--dangerously-skip-permissions".into()],
                nix_packages: vec![],
                env: HashMap::new(),
                credentials: vec!["ANTHROPIC_API_KEY".into()],
                harness_compatible: true,
            },
        );

        reg.frameworks.insert(
            "gemini-cli".into(),
            Framework {
                name: "gemini-cli".into(),
                description: "Google Gemini CLI".into(),
                command: "gemini".into(),
                args: vec![],
                nix_packages: vec![],
                env: HashMap::new(),
                credentials: vec!["GOOGLE_API_KEY".into()],
                harness_compatible: true,
            },
        );

        reg.frameworks.insert(
            "aider".into(),
            Framework {
                name: "aider".into(),
                description: "Aider AI pair programming".into(),
                command: "aider".into(),
                args: vec!["--yes".into()],
                nix_packages: vec!["python311".into()],
                env: HashMap::new(),
                credentials: vec!["ANTHROPIC_API_KEY".into()],
                harness_compatible: false,
            },
        );

        reg
    }

    #[test]
    fn test_generate_dev_agent() {
        let reg = test_registry();
        let spec = AgentSpec {
            name: "backend-1".into(),
            personality: "developer".into(),
            capabilities: vec!["code-writing".into(), "testing".into()],
            model: "balanced".into(),
            framework: "claude-code".into(),
            extra_instructions: None,
            overrides: AgentOverrides::default(),
        };

        let def = reg.generate(&spec).unwrap();

        assert_eq!(def.name, "backend-1");
        assert!(def.system_prompt.contains("senior software developer"));
        assert!(def.tools.contains(&"bash".into()));
        assert!(def.tools.contains(&"github_mcp".into()));
        assert_eq!(def.llm.model, "claude-sonnet-4-6");
        assert!(def.credentials.contains(&"GITHUB_TOKEN".into()));
        assert!(def.credentials.contains(&"ANTHROPIC_API_KEY".into()));
        assert_eq!(def.stall_threshold_minutes, 20);
    }

    #[test]
    fn test_generate_qa_agent() {
        let reg = test_registry();
        let spec = AgentSpec {
            name: "qa-1".into(),
            personality: "qa".into(),
            capabilities: vec!["code-writing".into(), "testing".into()],
            model: "balanced".into(),
            framework: "claude-code".into(),
            extra_instructions: Some("Focus on edge cases and error paths.".into()),
            overrides: AgentOverrides::default(),
        };

        let def = reg.generate(&spec).unwrap();

        assert_eq!(def.name, "qa-1");
        assert!(def.system_prompt.contains("QA engineer"));
        assert!(def.system_prompt.contains("edge cases"));
        assert_eq!(def.stall_threshold_minutes, 25);
        assert_eq!(def.nix.env.get("NODE_ENV").unwrap(), "test");
    }

    #[test]
    fn test_generate_architect_with_gemini() {
        let reg = test_registry();
        let spec = AgentSpec {
            name: "arch-1".into(),
            personality: "architect".into(),
            capabilities: vec!["code-writing".into(), "browser-research".into()],
            model: "gemini".into(),
            framework: "gemini-cli".into(),
            extra_instructions: None,
            overrides: AgentOverrides::default(),
        };

        let def = reg.generate(&spec).unwrap();

        assert_eq!(def.llm.model, "gemini-2.5-pro");
        assert!(def.tools.contains(&"browser".into()));
        assert!(def.nix.packages.contains(&"chromium".into()));
        assert!(def.credentials.contains(&"GOOGLE_API_KEY".into()));
        assert_eq!(def.stall_threshold_minutes, 30);
        assert_eq!(def.max_review_cycles, 2);
    }

    #[test]
    fn test_capability_merging_deduplicates() {
        let reg = test_registry();
        let spec = AgentSpec {
            name: "test".into(),
            personality: "developer".into(),
            capabilities: vec!["code-writing".into(), "testing".into()],
            model: "balanced".into(),
            framework: "claude-code".into(),
            extra_instructions: None,
            overrides: AgentOverrides::default(),
        };

        let def = reg.generate(&spec).unwrap();

        // "bash" is in both code-writing and testing — should appear once
        let bash_count = def.tools.iter().filter(|t| *t == "bash").count();
        assert_eq!(bash_count, 1);
    }

    #[test]
    fn test_overrides_applied() {
        let reg = test_registry();
        let spec = AgentSpec {
            name: "custom".into(),
            personality: "developer".into(),
            capabilities: vec!["code-writing".into()],
            model: "balanced".into(),
            framework: "claude-code".into(),
            extra_instructions: None,
            overrides: AgentOverrides {
                stall_threshold_minutes: Some(10),
                max_review_cycles: Some(5),
                extra_tools: vec!["custom_tool".into()],
                extra_nix_packages: vec!["postgresql_16".into()],
                extra_credentials: vec!["DATABASE_URL".into()],
                extra_env: HashMap::from([("DB_HOST".into(), "localhost".into())]),
            },
        };

        let def = reg.generate(&spec).unwrap();

        assert_eq!(def.stall_threshold_minutes, 10);
        assert_eq!(def.max_review_cycles, 5);
        assert!(def.tools.contains(&"custom_tool".into()));
        assert!(def.nix.packages.contains(&"postgresql_16".into()));
        assert!(def.credentials.contains(&"DATABASE_URL".into()));
        assert_eq!(def.nix.env.get("DB_HOST").unwrap(), "localhost");
    }

    #[test]
    fn test_unknown_personality_error() {
        let reg = test_registry();
        let spec = AgentSpec {
            name: "test".into(),
            personality: "nonexistent".into(),
            capabilities: vec![],
            model: "balanced".into(),
            framework: "claude-code".into(),
            extra_instructions: None,
            overrides: AgentOverrides::default(),
        };

        let result = reg.generate(&spec);
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("Unknown personality"));
    }

    #[test]
    fn test_unknown_capability_error() {
        let reg = test_registry();
        let spec = AgentSpec {
            name: "test".into(),
            personality: "developer".into(),
            capabilities: vec!["nonexistent".into()],
            model: "balanced".into(),
            framework: "claude-code".into(),
            extra_instructions: None,
            overrides: AgentOverrides::default(),
        };

        let result = reg.generate(&spec);
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("Unknown capability"));
    }

    #[test]
    fn test_registry_serialization_roundtrip() {
        let reg = test_registry();
        let json = serde_json::to_string_pretty(&reg).unwrap();
        let restored: AgentRegistry = serde_json::from_str(&json).unwrap();
        assert_eq!(restored.personalities.len(), reg.personalities.len());
        assert_eq!(restored.capabilities.len(), reg.capabilities.len());
        assert_eq!(restored.models.len(), reg.models.len());
        assert_eq!(restored.frameworks.len(), reg.frameworks.len());
    }

    #[test]
    fn test_agent_spec_yaml() {
        let yaml = r#"
name: backend-1
personality: developer
capabilities:
  - code-writing
  - testing
  - noether
model: balanced
framework: claude-code
extra_instructions: "Always write integration tests, not just unit tests."
overrides:
  extra_nix_packages:
    - postgresql_16
  extra_credentials:
    - DATABASE_URL
"#;

        let spec: AgentSpec = serde_yaml::from_str(yaml).unwrap();
        assert_eq!(spec.name, "backend-1");
        assert_eq!(spec.capabilities.len(), 3);
        assert!(spec.extra_instructions.unwrap().contains("integration tests"));
    }
}
