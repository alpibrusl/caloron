use std::collections::HashMap;

use serde::{Deserialize, Serialize};

/// Top-level configuration loaded from caloron.toml.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CaloronConfig {
    pub project: ProjectConfig,
    pub github: GitHubConfig,
    #[serde(default)]
    pub noether: NoetherConfig,
    #[serde(default)]
    pub supervisor: SupervisorConfig,
    #[serde(default)]
    pub retro: RetroConfig,
    #[serde(default)]
    pub nix: NixBuildConfig,
    pub llm: LlmGlobalConfig,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProjectConfig {
    pub name: String,
    pub repo: String,
    pub meta_repo: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GitHubConfig {
    #[serde(default = "default_token_env")]
    pub token_env: String,
    #[serde(default = "default_polling_interval")]
    pub polling_interval_seconds: u32,
    #[serde(default)]
    pub webhook_enabled: bool,
    #[serde(default = "default_webhook_port")]
    pub webhook_port: u16,
    #[serde(default)]
    pub webhook_secret_env: Option<String>,
}

fn default_token_env() -> String {
    "GITHUB_TOKEN".into()
}

fn default_polling_interval() -> u32 {
    5
}

fn default_webhook_port() -> u16 {
    9443
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NoetherConfig {
    #[serde(default)]
    pub enabled: bool,
    #[serde(default = "default_noether_endpoint")]
    pub endpoint: String,
    /// Path to the noether binary (default: "noether" from PATH)
    #[serde(default = "default_noether_binary")]
    pub binary: String,
}

fn default_noether_binary() -> String {
    "noether".into()
}

impl Default for NoetherConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            endpoint: default_noether_endpoint(),
            binary: default_noether_binary(),
        }
    }
}

fn default_noether_endpoint() -> String {
    "http://localhost:8080".into()
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SupervisorConfig {
    #[serde(default = "default_stall_threshold")]
    pub stall_default_threshold_minutes: u32,
    #[serde(default = "default_max_review_cycles")]
    pub max_review_cycles: u32,
    #[serde(default = "default_escalation_method")]
    pub escalation_method: String,
}

impl Default for SupervisorConfig {
    fn default() -> Self {
        Self {
            stall_default_threshold_minutes: default_stall_threshold(),
            max_review_cycles: default_max_review_cycles(),
            escalation_method: default_escalation_method(),
        }
    }
}

fn default_stall_threshold() -> u32 {
    20
}

fn default_max_review_cycles() -> u32 {
    3
}

fn default_escalation_method() -> String {
    "github_issue".into()
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RetroConfig {
    #[serde(default = "default_true")]
    pub enabled: bool,
    #[serde(default = "default_true")]
    pub auto_run: bool,
    #[serde(default = "default_output_format")]
    pub output_format: String,
}

impl Default for RetroConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            auto_run: true,
            output_format: default_output_format(),
        }
    }
}

fn default_true() -> bool {
    true
}

fn default_output_format() -> String {
    "markdown".into()
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct NixBuildConfig {
    #[serde(default = "default_true")]
    pub enabled: bool,
    #[serde(default)]
    pub extra_nixpkgs_config: String,
    #[serde(default)]
    pub cache_url: String,
}

/// Global LLM configuration with model alias support (Addendum R2).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LlmGlobalConfig {
    #[serde(default = "default_api_key_env")]
    pub api_key_env: String,
    /// Model aliases: agent definitions reference aliases like "default",
    /// which are resolved to concrete model IDs via this map.
    #[serde(default)]
    pub aliases: HashMap<String, String>,
}

fn default_api_key_env() -> String {
    "ANTHROPIC_API_KEY".into()
}

impl LlmGlobalConfig {
    /// Resolve a model reference: if it matches an alias, return the concrete model ID.
    /// Otherwise, treat it as a literal model ID.
    pub fn resolve_model(&self, model_ref: &str) -> String {
        self.aliases
            .get(model_ref)
            .cloned()
            .unwrap_or_else(|| model_ref.to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_config_parsing() {
        let toml_str = r#"
[project]
name = "my-project"
repo = "owner/repo"
meta_repo = "owner/caloron-meta"

[github]
token_env = "GITHUB_TOKEN"
polling_interval_seconds = 5

[llm]
api_key_env = "ANTHROPIC_API_KEY"

[llm.aliases]
default = "claude-sonnet-4-6"
fast = "claude-haiku-4-5"
strong = "claude-opus-4-6"
"#;
        let config: CaloronConfig = toml::from_str(toml_str).unwrap();
        assert_eq!(config.project.name, "my-project");
        assert_eq!(config.github.polling_interval_seconds, 5);
        assert_eq!(
            config.llm.resolve_model("default"),
            "claude-sonnet-4-6"
        );
        assert_eq!(
            config.llm.resolve_model("claude-opus-4-6"),
            "claude-opus-4-6"
        );
        // Defaults
        assert_eq!(config.supervisor.stall_default_threshold_minutes, 20);
        assert!(config.retro.enabled);
    }

    #[test]
    fn test_model_alias_resolution() {
        let config = LlmGlobalConfig {
            api_key_env: "ANTHROPIC_API_KEY".into(),
            aliases: HashMap::from([
                ("default".into(), "claude-sonnet-4-6".into()),
                ("strong".into(), "claude-opus-4-6".into()),
            ]),
        };

        assert_eq!(config.resolve_model("default"), "claude-sonnet-4-6");
        assert_eq!(config.resolve_model("strong"), "claude-opus-4-6");
        // Literal model ID passes through
        assert_eq!(
            config.resolve_model("claude-haiku-4-5"),
            "claude-haiku-4-5"
        );
    }

    #[test]
    fn test_default_polling_interval() {
        let toml_str = r#"
[project]
name = "test"
repo = "owner/repo"
meta_repo = "owner/meta"

[github]

[llm]
"#;
        let config: CaloronConfig = toml::from_str(toml_str).unwrap();
        assert_eq!(config.github.polling_interval_seconds, 5);
    }

    #[test]
    fn test_webhook_config() {
        let toml_str = r#"
[project]
name = "test"
repo = "owner/repo"
meta_repo = "owner/meta"

[github]
webhook_enabled = true
webhook_port = 8443
webhook_secret_env = "CALORON_WEBHOOK_SECRET"

[llm]
"#;
        let config: CaloronConfig = toml::from_str(toml_str).unwrap();
        assert!(config.github.webhook_enabled);
        assert_eq!(config.github.webhook_port, 8443);
    }
}
