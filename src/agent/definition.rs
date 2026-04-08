use std::path::Path;

use anyhow::{Context, Result, bail};

use caloron_types::agent::AgentDefinition;
use caloron_types::config::CaloronConfig;

/// Known tool names that agents can reference.
const KNOWN_TOOLS: &[&str] = &[
    "github_mcp",
    "noether",
    "bash",
    "browser",
    "filesystem",
];

/// Validation result with warnings and errors.
#[derive(Debug)]
pub struct ValidationResult {
    pub errors: Vec<String>,
    pub warnings: Vec<String>,
}

impl ValidationResult {
    pub fn is_valid(&self) -> bool {
        self.errors.is_empty()
    }
}

/// Load and validate an agent definition from a YAML file.
pub fn load_and_validate(
    path: &Path,
    config: &CaloronConfig,
) -> Result<(AgentDefinition, ValidationResult)> {
    let content =
        std::fs::read_to_string(path).with_context(|| format!("Failed to read {}", path.display()))?;

    let mut def: AgentDefinition = serde_yaml::from_str(&content)
        .with_context(|| format!("Failed to parse {}", path.display()))?;

    // Resolve model alias
    def.llm.model = config.llm.resolve_model(&def.llm.model);

    let result = validate(&def);

    Ok((def, result))
}

/// Validate an agent definition's fields.
pub fn validate(def: &AgentDefinition) -> ValidationResult {
    let mut errors = Vec::new();
    let mut warnings = Vec::new();

    // Required fields
    if def.name.is_empty() {
        errors.push("Agent name is required".into());
    }

    if def.version.is_empty() {
        errors.push("Agent version is required".into());
    }

    if def.system_prompt.is_empty() {
        errors.push("System prompt is required".into());
    }

    if def.tools.is_empty() {
        errors.push("At least one tool must be configured".into());
    }

    if def.llm.model.is_empty() {
        errors.push("LLM model is required".into());
    }

    // Validate tool names against known registry
    for tool in &def.tools {
        if !KNOWN_TOOLS.contains(&tool.as_str()) {
            warnings.push(format!(
                "Tool '{tool}' is not in the known tool registry — ensure it is available at runtime"
            ));
        }
    }

    // Validate MCP configs
    for mcp in &def.mcps {
        if mcp.name.is_empty() {
            errors.push("MCP config must have a name".into());
        }
        if mcp.url.is_empty() {
            errors.push(format!("MCP '{}' must have a URL", mcp.name));
        }
    }

    // Validate Nix packages don't contain obviously invalid names
    for pkg in &def.nix.packages {
        if pkg.contains(' ') || pkg.contains('/') {
            errors.push(format!("Invalid Nix package name: '{pkg}'"));
        }
    }

    // Temperature range
    if def.llm.temperature < 0.0 || def.llm.temperature > 2.0 {
        errors.push(format!(
            "Temperature {} is out of range [0.0, 2.0]",
            def.llm.temperature
        ));
    }

    // Stall threshold sanity
    if def.stall_threshold_minutes < 5 {
        warnings.push(format!(
            "Stall threshold of {} minutes is very short — may cause false stall detections",
            def.stall_threshold_minutes
        ));
    }

    // Credentials check
    if def.credentials.is_empty() {
        warnings.push(
            "No credentials configured — agent will not have access to GitHub or LLM APIs".into(),
        );
    }

    ValidationResult { errors, warnings }
}

/// Print validation results to stdout for the CLI.
pub fn print_validation(def: &AgentDefinition, result: &ValidationResult) {
    println!("Agent: {} v{}", def.name, def.version);
    println!("Model: {}", def.llm.model);
    println!("Tools: {}", def.tools.join(", "));
    println!(
        "Nix packages: {}",
        if def.nix.packages.is_empty() {
            "(none)".to_string()
        } else {
            def.nix.packages.join(", ")
        }
    );
    println!(
        "Credentials: {}",
        if def.credentials.is_empty() {
            "(none)".to_string()
        } else {
            def.credentials.join(", ")
        }
    );
    println!();

    if result.is_valid() && result.warnings.is_empty() {
        println!("Validation: PASSED");
    } else {
        for err in &result.errors {
            println!("  ERROR: {err}");
        }
        for warn in &result.warnings {
            println!("  WARN:  {warn}");
        }
        println!();
        if result.is_valid() {
            println!("Validation: PASSED (with warnings)");
        } else {
            println!("Validation: FAILED");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use caloron_types::agent::{LlmConfig, NixConfig};
    use std::collections::HashMap;

    fn valid_def() -> AgentDefinition {
        AgentDefinition {
            name: "test-agent".into(),
            version: "1.0".into(),
            description: "Test agent".into(),
            llm: LlmConfig {
                model: "claude-sonnet-4-6".into(),
                max_tokens: 8192,
                temperature: 0.2,
            },
            system_prompt: "You are a test agent.".into(),
            tools: vec!["bash".into()],
            mcps: vec![],
            nix: NixConfig::default(),
            credentials: vec!["GITHUB_TOKEN".into()],
            stall_threshold_minutes: 20,
            max_review_cycles: 3,
        }
    }

    #[test]
    fn test_valid_definition() {
        let result = validate(&valid_def());
        assert!(result.is_valid());
    }

    #[test]
    fn test_missing_name() {
        let mut def = valid_def();
        def.name = String::new();
        let result = validate(&def);
        assert!(!result.is_valid());
        assert!(result.errors.iter().any(|e| e.contains("name")));
    }

    #[test]
    fn test_missing_tools() {
        let mut def = valid_def();
        def.tools.clear();
        let result = validate(&def);
        assert!(!result.is_valid());
    }

    #[test]
    fn test_unknown_tool_warning() {
        let mut def = valid_def();
        def.tools.push("custom_tool".into());
        let result = validate(&def);
        assert!(result.is_valid()); // warning, not error
        assert!(result.warnings.iter().any(|w| w.contains("custom_tool")));
    }

    #[test]
    fn test_invalid_temperature() {
        let mut def = valid_def();
        def.llm.temperature = 3.0;
        let result = validate(&def);
        assert!(!result.is_valid());
    }

    #[test]
    fn test_low_stall_threshold_warning() {
        let mut def = valid_def();
        def.stall_threshold_minutes = 2;
        let result = validate(&def);
        assert!(result.is_valid());
        assert!(!result.warnings.is_empty());
    }

    #[test]
    fn test_invalid_nix_package() {
        let mut def = valid_def();
        def.nix.packages.push("invalid package name".into());
        let result = validate(&def);
        assert!(!result.is_valid());
    }

    #[test]
    fn test_no_credentials_warning() {
        let mut def = valid_def();
        def.credentials.clear();
        let result = validate(&def);
        assert!(result.is_valid());
        assert!(result
            .warnings
            .iter()
            .any(|w| w.contains("credentials")));
    }
}
