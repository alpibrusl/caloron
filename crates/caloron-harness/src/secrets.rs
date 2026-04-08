use std::collections::HashMap;

/// Load secrets from a temporary file and delete it immediately (Addendum R3).
/// The file path is read from CALORON_SECRETS_FILE env var.
/// Format: KEY=VALUE, one per line.
pub fn load_and_delete_secrets() -> HashMap<String, String> {
    let secrets_path = match std::env::var("CALORON_SECRETS_FILE") {
        Ok(path) => path,
        Err(_) => {
            tracing::debug!("CALORON_SECRETS_FILE not set — no secrets to load");
            return HashMap::new();
        }
    };

    let contents = match std::fs::read_to_string(&secrets_path) {
        Ok(c) => c,
        Err(e) => {
            tracing::warn!(path = secrets_path, error = %e, "Failed to read secrets file");
            return HashMap::new();
        }
    };

    // Delete immediately after reading
    if let Err(e) = std::fs::remove_file(&secrets_path) {
        tracing::warn!(path = secrets_path, error = %e, "Failed to delete secrets file");
    } else {
        tracing::debug!(path = secrets_path, "Secrets file deleted after reading");
    }

    // Parse KEY=VALUE lines
    contents
        .lines()
        .filter(|line| !line.is_empty() && !line.starts_with('#'))
        .filter_map(|line| {
            line.split_once('=')
                .map(|(k, v)| (k.trim().to_string(), v.trim().to_string()))
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_load_and_delete_secrets() {
        let dir = tempfile::tempdir().unwrap();
        let secrets_file = dir.path().join("test.env");

        std::fs::write(
            &secrets_file,
            "GITHUB_TOKEN=ghp_abc123\nANTHROPIC_API_KEY=sk-ant-xyz\n# comment\n\nEXTRA=value\n",
        )
        .unwrap();

        // SAFETY: Test is single-threaded (cargo test runs with --test-threads=1 for env var tests)
        unsafe {
            std::env::set_var("CALORON_SECRETS_FILE", secrets_file.to_str().unwrap());
        }

        let secrets = load_and_delete_secrets();

        assert_eq!(secrets.len(), 3);
        assert_eq!(secrets["GITHUB_TOKEN"], "ghp_abc123");
        assert_eq!(secrets["ANTHROPIC_API_KEY"], "sk-ant-xyz");
        assert_eq!(secrets["EXTRA"], "value");

        // File should be deleted
        assert!(!secrets_file.exists());

        unsafe {
            std::env::remove_var("CALORON_SECRETS_FILE");
        }
    }

    #[test]
    fn test_no_secrets_file() {
        unsafe {
            std::env::remove_var("CALORON_SECRETS_FILE");
        }
        let secrets = load_and_delete_secrets();
        assert!(secrets.is_empty());
    }
}
