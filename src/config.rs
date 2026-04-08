use std::path::Path;

use anyhow::{Context, Result};
use caloron_types::config::CaloronConfig;

/// Load and validate configuration from a TOML file.
pub fn load_config(path: &Path) -> Result<CaloronConfig> {
    let content =
        std::fs::read_to_string(path).with_context(|| format!("Failed to read {}", path.display()))?;

    let config: CaloronConfig =
        toml::from_str(&content).with_context(|| format!("Failed to parse {}", path.display()))?;

    validate_config(&config)?;

    Ok(config)
}

/// Validate that required environment variables and resources are accessible.
fn validate_config(config: &CaloronConfig) -> Result<()> {
    // Check GitHub token env var exists
    if std::env::var(&config.github.token_env).is_err() {
        tracing::warn!(
            env_var = config.github.token_env,
            "GitHub token environment variable not set"
        );
    }

    // Check LLM API key env var exists
    if std::env::var(&config.llm.api_key_env).is_err() {
        tracing::warn!(
            env_var = config.llm.api_key_env,
            "LLM API key environment variable not set"
        );
    }

    // Validate webhook config consistency
    if config.github.webhook_enabled && config.github.webhook_secret_env.is_none() {
        anyhow::bail!(
            "Webhook is enabled but webhook_secret_env is not configured in [github]"
        );
    }

    Ok(())
}

/// Load an agent definition from a YAML file, resolving model aliases.
pub fn load_agent_definition(
    path: &Path,
    config: &CaloronConfig,
) -> Result<caloron_types::agent::AgentDefinition> {
    let content =
        std::fs::read_to_string(path).with_context(|| format!("Failed to read {}", path.display()))?;

    let mut def: caloron_types::agent::AgentDefinition = serde_yaml::from_str(&content)
        .with_context(|| format!("Failed to parse agent definition {}", path.display()))?;

    // Resolve model alias
    def.llm.model = config.llm.resolve_model(&def.llm.model);

    Ok(def)
}
