use std::collections::HashMap;

use caloron_types::agent::McpConfig;
use caloron_types::agent_gen::*;

/// Build the default agent registry with built-in personalities, capabilities,
/// models, and frameworks.
pub fn default_registry() -> AgentRegistry {
    let mut reg = AgentRegistry::default();

    // =========================================================================
    // Personalities
    // =========================================================================

    reg.personalities.insert("developer".into(), Personality {
        role: "developer".into(),
        description: "Implements features, writes tests, and opens PRs for review".into(),
        system_prompt: indoc(r#"
            You are a senior software developer working on this project.
            You receive tasks as GitHub issues assigned to you.

            Your workflow:
            1. Read the assigned issue carefully
            2. Explore the codebase to understand existing patterns
            3. Implement the required changes
            4. Write tests for your changes
            5. Open a pull request with a clear description
            6. When complete, post a caloron_feedback comment on the issue

            Always check existing code patterns before implementing.
            Never make assumptions about external API formats — ask via issue comment if unclear.
        "#),
        stall_threshold_minutes: Some(20),
        max_review_cycles: None,
    });

    reg.personalities.insert("qa".into(), Personality {
        role: "qa".into(),
        description: "Writes comprehensive tests and validates acceptance criteria".into(),
        system_prompt: indoc(r#"
            You are a QA engineer working on this project.
            You receive testing tasks as GitHub issues.

            Your workflow:
            1. Read the assigned issue and linked PRs/features
            2. Understand the acceptance criteria
            3. Write integration tests covering happy paths and edge cases
            4. Write tests for error handling and boundary conditions
            5. Run the full test suite and verify all tests pass
            6. Open a PR with your test changes
            7. Post a caloron_feedback comment

            Focus on testing behavior, not implementation details.
            Always test error paths and edge cases, not just the happy path.
        "#),
        stall_threshold_minutes: Some(25),
        max_review_cycles: None,
    });

    reg.personalities.insert("reviewer".into(), Personality {
        role: "reviewer".into(),
        description: "Reviews PRs for code quality, correctness, and security".into(),
        system_prompt: indoc(r#"
            You are a senior code reviewer.
            You receive pull requests to review.

            Review criteria:
            1. Correctness — does the code do what the issue asks?
            2. Tests — are there adequate tests?
            3. Code quality — follows existing patterns, no unnecessary complexity
            4. Security — no obvious vulnerabilities (injection, XSS, etc.)
            5. Documentation — clear PR description, comments where needed

            If changes are needed, be specific about what to change and why.
            If the PR is good, approve it promptly — don't block on style preferences.
        "#),
        stall_threshold_minutes: Some(30),
        max_review_cycles: Some(3),
    });

    reg.personalities.insert("architect".into(), Personality {
        role: "architect".into(),
        description: "Reviews system design, identifies architectural concerns".into(),
        system_prompt: indoc(r#"
            You are a software architect reviewing this project.
            You focus on system-level design decisions.

            Your concerns:
            1. Scalability — will this approach work at 10x/100x scale?
            2. Maintainability — is the code organized for future changes?
            3. Dependencies — are external dependencies justified and well-abstracted?
            4. Data flow — is the data model consistent and well-typed?
            5. Error handling — are failure modes identified and handled?
            6. Security boundaries — are trust boundaries correctly placed?

            When reviewing, focus on structural issues rather than style.
            Propose concrete alternatives when you identify a problem.
        "#),
        stall_threshold_minutes: Some(30),
        max_review_cycles: Some(2),
    });

    reg.personalities.insert("designer".into(), Personality {
        role: "designer".into(),
        description: "Implements UI/UX designs, creates frontend components".into(),
        system_prompt: indoc(r#"
            You are a frontend designer and developer.
            You implement user interfaces and design systems.

            Your workflow:
            1. Read the design requirements in the issue
            2. Study existing UI patterns and component library
            3. Implement the UI changes with responsive design
            4. Ensure accessibility (ARIA labels, keyboard navigation, contrast)
            5. Write visual regression tests where applicable
            6. Open a PR with screenshots/descriptions of the changes

            Prioritize consistency with the existing design system.
            Use semantic HTML and progressive enhancement.
        "#),
        stall_threshold_minutes: Some(25),
        max_review_cycles: None,
    });

    reg.personalities.insert("ux-researcher".into(), Personality {
        role: "ux-researcher".into(),
        description: "Analyzes user experience, audits usability, proposes improvements".into(),
        system_prompt: indoc(r#"
            You are a UX researcher auditing this project.
            You analyze the user experience and propose improvements.

            Your approach:
            1. Study the current user flows and interfaces
            2. Identify usability issues and friction points
            3. Research best practices for the domain
            4. Document findings with specific, actionable recommendations
            5. Prioritize recommendations by impact and effort
            6. Open issues or PRs with your findings

            Support your recommendations with evidence and rationale.
            Consider accessibility, internationalization, and diverse user needs.
        "#),
        stall_threshold_minutes: Some(40),
        max_review_cycles: None,
    });

    reg.personalities.insert("devops".into(), Personality {
        role: "devops".into(),
        description: "Manages CI/CD, infrastructure, and deployment configuration".into(),
        system_prompt: indoc(r#"
            You are a DevOps engineer working on this project.
            You manage CI/CD pipelines, infrastructure, and deployment.

            Your concerns:
            1. CI/CD — pipelines are reliable and fast
            2. Infrastructure — resources are correctly provisioned
            3. Monitoring — appropriate alerts and dashboards
            4. Security — secrets management, least privilege
            5. Reproducibility — builds are deterministic

            Prefer infrastructure-as-code. Document any manual steps.
            Never commit secrets or credentials.
        "#),
        stall_threshold_minutes: Some(20),
        max_review_cycles: None,
    });

    // =========================================================================
    // Capability Bundles
    // =========================================================================

    reg.capabilities.insert("code-writing".into(), CapabilityBundle {
        name: "code-writing".into(),
        description: "Write, modify, and commit code via GitHub".into(),
        tools: vec!["bash".into(), "github_mcp".into()],
        mcps: vec![McpConfig {
            url: "https://github.mcp.claude.com/mcp".into(),
            name: "github".into(),
        }],
        nix_packages: vec!["git".into()],
        env: HashMap::new(),
        credentials: vec!["GITHUB_TOKEN".into()],
    });

    reg.capabilities.insert("testing".into(), CapabilityBundle {
        name: "testing".into(),
        description: "Run tests, linters, and type checkers".into(),
        tools: vec!["bash".into()],
        mcps: vec![],
        nix_packages: vec![],
        env: HashMap::from([("NODE_ENV".into(), "test".into())]),
        credentials: vec![],
    });

    reg.capabilities.insert("browser-research".into(), CapabilityBundle {
        name: "browser-research".into(),
        description: "Browse the web for research and reference".into(),
        tools: vec!["browser".into()],
        mcps: vec![],
        nix_packages: vec![],
        env: HashMap::new(),
        credentials: vec![],
    });

    reg.capabilities.insert("noether".into(), CapabilityBundle {
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
    });

    reg.capabilities.insert("rust".into(), CapabilityBundle {
        name: "rust".into(),
        description: "Rust development toolchain".into(),
        tools: vec![],
        mcps: vec![],
        nix_packages: vec!["rustc".into(), "cargo".into(), "clippy".into()],
        env: HashMap::new(),
        credentials: vec![],
    });

    reg.capabilities.insert("nodejs".into(), CapabilityBundle {
        name: "nodejs".into(),
        description: "Node.js development toolchain".into(),
        tools: vec![],
        mcps: vec![],
        nix_packages: vec!["nodejs_20".into()],
        env: HashMap::new(),
        credentials: vec![],
    });

    reg.capabilities.insert("python".into(), CapabilityBundle {
        name: "python".into(),
        description: "Python development toolchain".into(),
        tools: vec![],
        mcps: vec![],
        nix_packages: vec!["python311".into()],
        env: HashMap::new(),
        credentials: vec![],
    });

    reg.capabilities.insert("frontend".into(), CapabilityBundle {
        name: "frontend".into(),
        description: "Frontend development (Node.js + browser)".into(),
        tools: vec!["bash".into(), "browser".into()],
        mcps: vec![],
        nix_packages: vec!["nodejs_20".into()],
        env: HashMap::new(),
        credentials: vec![],
    });

    // =========================================================================
    // Models
    // =========================================================================

    reg.models.insert("balanced".into(), ModelConfig {
        model: "claude-sonnet-4-6".into(),
        max_tokens: 8192,
        temperature: 0.2,
    });

    reg.models.insert("strong".into(), ModelConfig {
        model: "claude-opus-4-6".into(),
        max_tokens: 16384,
        temperature: 0.1,
    });

    reg.models.insert("fast".into(), ModelConfig {
        model: "claude-haiku-4-5".into(),
        max_tokens: 4096,
        temperature: 0.3,
    });

    reg.models.insert("gemini-pro".into(), ModelConfig {
        model: "gemini-2.5-pro".into(),
        max_tokens: 8192,
        temperature: 0.2,
    });

    reg.models.insert("gemini-flash".into(), ModelConfig {
        model: "gemini-2.5-flash".into(),
        max_tokens: 8192,
        temperature: 0.3,
    });

    // =========================================================================
    // Frameworks
    // =========================================================================

    reg.frameworks.insert("claude-code".into(), Framework {
        name: "claude-code".into(),
        description: "Anthropic Claude Code — full agentic coding CLI".into(),
        command: "claude".into(),
        args: vec!["--dangerously-skip-permissions".into()],
        nix_packages: vec![],
        env: HashMap::new(),
        credentials: vec!["ANTHROPIC_API_KEY".into()],
        harness_compatible: true,
    });

    reg.frameworks.insert("gemini-cli".into(), Framework {
        name: "gemini-cli".into(),
        description: "Google Gemini CLI".into(),
        command: "gemini".into(),
        args: vec![],
        nix_packages: vec![],
        env: HashMap::new(),
        credentials: vec!["GOOGLE_API_KEY".into()],
        harness_compatible: true,
    });

    reg.frameworks.insert("aider".into(), Framework {
        name: "aider".into(),
        description: "Aider — AI pair programming in terminal".into(),
        command: "aider".into(),
        args: vec!["--yes".into(), "--no-auto-commits".into()],
        nix_packages: vec!["python311".into()],
        env: HashMap::new(),
        credentials: vec!["ANTHROPIC_API_KEY".into()],
        harness_compatible: false,
    });

    reg.frameworks.insert("codex-cli".into(), Framework {
        name: "codex-cli".into(),
        description: "OpenAI Codex CLI".into(),
        command: "codex".into(),
        args: vec!["--approval-mode".into(), "full-auto".into()],
        nix_packages: vec![],
        env: HashMap::new(),
        credentials: vec!["OPENAI_API_KEY".into()],
        harness_compatible: true,
    });

    reg
}

/// Dedent a string (remove common leading whitespace).
fn indoc(s: &str) -> String {
    let lines: Vec<&str> = s.lines().collect();

    // Find minimum indentation (ignoring empty lines)
    let min_indent = lines
        .iter()
        .filter(|l| !l.trim().is_empty())
        .map(|l| l.len() - l.trim_start().len())
        .min()
        .unwrap_or(0);

    lines
        .iter()
        .map(|l| {
            if l.len() >= min_indent {
                &l[min_indent..]
            } else {
                l.trim()
            }
        })
        .collect::<Vec<_>>()
        .join("\n")
        .trim()
        .to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_default_registry_has_all_components() {
        let reg = default_registry();

        assert!(reg.personalities.len() >= 7, "Expected 7+ personalities");
        assert!(reg.capabilities.len() >= 8, "Expected 8+ capabilities");
        assert!(reg.models.len() >= 5, "Expected 5+ models");
        assert!(reg.frameworks.len() >= 4, "Expected 4+ frameworks");
    }

    #[test]
    fn test_generate_all_standard_agents() {
        let reg = default_registry();

        let specs = vec![
            ("dev-1", "developer", vec!["code-writing", "testing", "rust"], "balanced", "claude-code"),
            ("qa-1", "qa", vec!["code-writing", "testing", "nodejs"], "balanced", "claude-code"),
            ("rev-1", "reviewer", vec!["code-writing"], "strong", "claude-code"),
            ("arch-1", "architect", vec!["code-writing", "browser-research"], "strong", "claude-code"),
            ("designer-1", "designer", vec!["code-writing", "frontend"], "balanced", "claude-code"),
            ("ux-1", "ux-researcher", vec!["browser-research"], "balanced", "gemini-cli"),
            ("devops-1", "devops", vec!["code-writing", "testing"], "fast", "claude-code"),
        ];

        for (name, personality, caps, model, framework) in specs {
            let spec = AgentSpec {
                name: name.into(),
                personality: personality.into(),
                capabilities: caps.into_iter().map(|s| s.into()).collect(),
                model: model.into(),
                framework: framework.into(),
                extra_instructions: None,
                overrides: AgentOverrides::default(),
            };

            let def = reg.generate(&spec).unwrap_or_else(|e| {
                panic!("Failed to generate {name}: {e}");
            });

            assert!(!def.system_prompt.is_empty(), "{name} has empty prompt");
            assert!(!def.tools.is_empty() || personality == "ux-researcher",
                "{name} has no tools (personality: {personality})");
            assert!(!def.llm.model.is_empty(), "{name} has empty model");
        }
    }

    #[test]
    fn test_mixed_framework_and_model() {
        let reg = default_registry();

        // Developer using Gemini instead of Claude
        let spec = AgentSpec {
            name: "gemini-dev".into(),
            personality: "developer".into(),
            capabilities: vec!["code-writing".into(), "python".into()],
            model: "gemini-pro".into(),
            framework: "gemini-cli".into(),
            extra_instructions: None,
            overrides: AgentOverrides::default(),
        };

        let def = reg.generate(&spec).unwrap();
        assert_eq!(def.llm.model, "gemini-2.5-pro");
        assert!(def.credentials.contains(&"GOOGLE_API_KEY".into()));
        assert!(!def.credentials.contains(&"ANTHROPIC_API_KEY".into()));
        assert!(def.nix.packages.contains(&"python311".into()));
    }

    #[test]
    fn test_indoc() {
        let result = indoc("
            Hello
            World
        ");
        assert_eq!(result, "Hello\nWorld");
    }
}
