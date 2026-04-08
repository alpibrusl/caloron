use std::process::Command;

use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};

/// Client for the Noether verified composition platform.
/// Wraps the `noether` CLI (ACLI-compliant) to provide stage search,
/// composition, and execution capabilities to Caloron agents.
pub struct NoetherClient {
    /// Path to the noether binary
    binary: String,
    /// Optional remote registry URL
    registry: Option<String>,
}

/// ACLI envelope returned by Noether CLI.
#[derive(Debug, Deserialize)]
struct AcliEnvelope {
    pub ok: bool,
    pub data: Option<serde_json::Value>,
    pub error: Option<AcliError>,
}

#[derive(Debug, Deserialize)]
struct AcliError {
    pub message: String,
}

/// A stage from the Noether store.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StageInfo {
    pub id: String,
    pub description: String,
    #[serde(default)]
    pub input_type: Option<serde_json::Value>,
    #[serde(default)]
    pub output_type: Option<serde_json::Value>,
    #[serde(default)]
    pub effects: Vec<String>,
    #[serde(default)]
    pub lifecycle: Option<String>,
}

/// Search result from semantic stage search.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SearchResult {
    pub stage_id: String,
    pub score: f64,
    #[serde(default)]
    pub description: Option<String>,
}

/// Store statistics.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StoreStats {
    #[serde(default)]
    pub total_stages: u64,
    #[serde(default)]
    pub active_stages: u64,
    #[serde(default)]
    pub deprecated_stages: u64,
}

/// Result of running a composition.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CompositionResult {
    pub output: serde_json::Value,
    #[serde(default)]
    pub trace_id: Option<String>,
    #[serde(default)]
    pub stages_executed: u32,
    #[serde(default)]
    pub total_cost_cents: Option<f64>,
}

impl NoetherClient {
    pub fn new(binary: &str, registry: Option<String>) -> Self {
        Self {
            binary: binary.to_string(),
            registry,
        }
    }

    /// Create a client with default settings.
    /// Looks for `noether` in PATH.
    pub fn default() -> Self {
        Self::new("noether", None)
    }

    /// Check if Noether is available and responding.
    pub fn health_check(&self) -> Result<bool> {
        let output = self.run_command(&["version"])?;
        Ok(output.ok)
    }

    /// Search for stages by semantic query.
    pub fn search_stages(&self, query: &str) -> Result<Vec<SearchResult>> {
        let envelope = self.run_command(&["stage", "search", query])?;
        let data = envelope
            .data
            .context("No data in search response")?;

        // The search result may be nested under a "results" key
        let results = if let Some(arr) = data.as_array() {
            serde_json::from_value(serde_json::Value::Array(arr.clone()))?
        } else if let Some(obj) = data.get("results") {
            serde_json::from_value(obj.clone())?
        } else {
            vec![]
        };

        Ok(results)
    }

    /// List all stages in the store.
    pub fn list_stages(&self) -> Result<Vec<StageInfo>> {
        let envelope = self.run_command(&["stage", "list"])?;
        let data = envelope.data.context("No data in list response")?;

        let stages = if let Some(arr) = data.as_array() {
            serde_json::from_value(serde_json::Value::Array(arr.clone()))?
        } else if let Some(obj) = data.get("stages") {
            serde_json::from_value(obj.clone())?
        } else {
            vec![]
        };

        Ok(stages)
    }

    /// Get a specific stage by hash ID.
    pub fn get_stage(&self, hash: &str) -> Result<StageInfo> {
        let envelope = self.run_command(&["stage", "get", hash])?;
        let data = envelope.data.context("No data in get response")?;
        let stage: StageInfo = serde_json::from_value(data)?;
        Ok(stage)
    }

    /// Get store statistics.
    pub fn store_stats(&self) -> Result<StoreStats> {
        let envelope = self.run_command(&["store", "stats"])?;
        let data = envelope.data.context("No data in stats response")?;
        let stats: StoreStats = serde_json::from_value(data)?;
        Ok(stats)
    }

    /// Compose a solution from a natural language problem description.
    /// Returns the composition result including output and trace.
    pub fn compose(
        &self,
        problem: &str,
        input: Option<&str>,
        dry_run: bool,
    ) -> Result<serde_json::Value> {
        let mut args = vec!["compose", problem];

        if dry_run {
            args.push("--dry-run");
        }

        // Build owned string for input to avoid lifetime issues
        let input_flag;
        if let Some(inp) = input {
            input_flag = format!("{}", inp);
            args.push("--input");
            args.push(&input_flag);
        }

        let envelope = self.run_command(&args)?;
        envelope.data.context("No data in compose response")
    }

    /// Execute a composition graph from a file.
    pub fn run_graph(
        &self,
        graph_path: &str,
        input: Option<&str>,
        dry_run: bool,
        budget_cents: Option<u64>,
    ) -> Result<serde_json::Value> {
        let mut args = vec!["run".to_string(), graph_path.to_string()];

        if dry_run {
            args.push("--dry-run".into());
        }

        if let Some(inp) = input {
            args.push("--input".into());
            args.push(inp.to_string());
        }

        if let Some(budget) = budget_cents {
            args.push("--budget-cents".into());
            args.push(budget.to_string());
        }

        let str_args: Vec<&str> = args.iter().map(|s| s.as_str()).collect();
        let envelope = self.run_command(&str_args)?;
        envelope.data.context("No data in run response")
    }

    /// Retrieve an execution trace by composition ID.
    pub fn get_trace(&self, composition_id: &str) -> Result<serde_json::Value> {
        let envelope = self.run_command(&["trace", composition_id])?;
        envelope.data.context("No data in trace response")
    }

    /// Run a noether CLI command and parse the ACLI envelope.
    fn run_command(&self, args: &[&str]) -> Result<AcliEnvelope> {
        let mut cmd = Command::new(&self.binary);
        cmd.args(args);

        if let Some(registry) = &self.registry {
            cmd.arg("--registry").arg(registry);
        }

        let output = cmd
            .output()
            .with_context(|| format!("Failed to execute: {} {}", self.binary, args.join(" ")))?;

        let stdout = String::from_utf8_lossy(&output.stdout);

        if stdout.trim().is_empty() {
            if !output.status.success() {
                let stderr = String::from_utf8_lossy(&output.stderr);
                bail!("noether command failed: {stderr}");
            }
            // Some commands may not produce output
            return Ok(AcliEnvelope {
                ok: true,
                data: None,
                error: None,
            });
        }

        let envelope: AcliEnvelope = serde_json::from_str(stdout.trim())
            .with_context(|| format!("Failed to parse noether output as ACLI envelope: {stdout}"))?;

        if !envelope.ok {
            let msg = envelope
                .error
                .map(|e| e.message)
                .unwrap_or_else(|| "Unknown error".into());
            bail!("noether error: {msg}");
        }

        Ok(envelope)
    }
}

/// Noether service lifecycle management for the daemon.
pub struct NoetherService {
    client: NoetherClient,
}

impl NoetherService {
    pub fn new(endpoint: &str) -> Self {
        let registry = if endpoint.is_empty() {
            None
        } else {
            Some(endpoint.to_string())
        };

        Self {
            client: NoetherClient::new("noether", registry),
        }
    }

    /// Check if Noether is available.
    pub fn is_available(&self) -> bool {
        self.client.health_check().unwrap_or(false)
    }

    /// Get the client for agent use.
    pub fn client(&self) -> &NoetherClient {
        &self.client
    }

    /// Get store stats for status display.
    pub fn stats(&self) -> Result<StoreStats> {
        self.client.store_stats()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_noether_client_creation() {
        let client = NoetherClient::default();
        assert_eq!(client.binary, "noether");
        assert!(client.registry.is_none());
    }

    #[test]
    fn test_noether_client_with_registry() {
        let client = NoetherClient::new("noether", Some("http://localhost:3000".into()));
        assert_eq!(client.registry.as_deref(), Some("http://localhost:3000"));
    }

    #[test]
    fn test_acli_envelope_parsing() {
        let json = r#"{"ok": true, "data": {"stages": []}}"#;
        let envelope: AcliEnvelope = serde_json::from_str(json).unwrap();
        assert!(envelope.ok);
        assert!(envelope.data.is_some());
    }

    #[test]
    fn test_acli_error_envelope_parsing() {
        let json = r#"{"ok": false, "error": {"message": "Stage not found"}}"#;
        let envelope: AcliEnvelope = serde_json::from_str(json).unwrap();
        assert!(!envelope.ok);
        assert_eq!(envelope.error.unwrap().message, "Stage not found");
    }

    #[test]
    fn test_noether_service_creation() {
        let service = NoetherService::new("http://localhost:8080");
        assert_eq!(
            service.client().registry.as_deref(),
            Some("http://localhost:8080")
        );
    }

    // Integration tests (require noether binary to be available):
    //
    // #[test]
    // fn test_health_check() {
    //     let client = NoetherClient::default();
    //     assert!(client.health_check().is_ok());
    // }
    //
    // #[test]
    // fn test_list_stages() {
    //     let client = NoetherClient::default();
    //     let stages = client.list_stages().unwrap();
    //     assert!(!stages.is_empty()); // stdlib should have 76 stages
    // }
    //
    // #[test]
    // fn test_search_stages() {
    //     let client = NoetherClient::default();
    //     let results = client.search_stages("parse json").unwrap();
    //     assert!(!results.is_empty());
    // }
}
