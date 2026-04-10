"""
Agent Configurator — translates HR Agent skill assignments into
framework-specific configuration files.

For each agent, generates:
- Claude Code: --mcp-config JSON + CLAUDE.md with skill instructions
- Cursor CLI: .cursorrules + MCP config
- Gemini CLI: system instruction file
- Aider: .aider.conf.yml

These files are written to the agent's worktree before it runs.
"""
import json
import os
from pathlib import Path


def configure_agent(
    worktree: str,
    task: dict,
    framework: str = "claude-code",
) -> dict:
    """Configure the agent's worktree with skill-specific files.

    Returns a dict with any extra CLI flags to pass to the framework.
    """
    skills = task.get("skills", [])
    mcp_urls = task.get("mcp_urls", [])
    model = task.get("model", "balanced")
    nix_packages = task.get("nix_packages", [])

    extra_flags = []

    if framework == "claude-code":
        extra_flags = configure_claude_code(worktree, task, skills, mcp_urls)
    elif framework == "cursor-cli":
        configure_cursor(worktree, task, skills, mcp_urls)
    elif framework == "gemini-cli":
        configure_gemini(worktree, task, skills)
    elif framework == "aider":
        configure_aider(worktree, task, skills)

    return {"extra_flags": extra_flags}


def configure_claude_code(
    worktree: str,
    task: dict,
    skills: list[str],
    mcp_urls: list[dict],
) -> list[str]:
    """Configure Claude Code with MCP servers and CLAUDE.md."""
    extra_flags = []

    # ── MCP config ──────────────────────────────────────────────────────
    if mcp_urls:
        mcp_config = {"mcpServers": {}}
        for mcp in mcp_urls:
            name = mcp["name"]
            url = mcp["url"]
            # Different MCP server types
            if url.startswith("postgresql://") or url.startswith("postgres://"):
                mcp_config["mcpServers"][name] = {
                    "command": "npx",
                    "args": ["-y", "@modelcontextprotocol/server-postgres", url],
                }
            elif url.startswith("http://") or url.startswith("https://"):
                mcp_config["mcpServers"][name] = {
                    "command": "npx",
                    "args": ["-y", "mcp-remote", url],
                }
            elif url.startswith("slack://"):
                mcp_config["mcpServers"][name] = {
                    "command": "npx",
                    "args": ["-y", "@anthropic-ai/mcp-slack"],
                    "env": {"SLACK_TOKEN": "${SLACK_TOKEN}"},
                }

        config_path = os.path.join(worktree, ".mcp.json")
        Path(config_path).write_text(json.dumps(mcp_config, indent=2))
        extra_flags.extend(["--mcp-config", config_path])

    # ── CLAUDE.md — skill-specific instructions ─────────────────────────
    claude_md_lines = ["# Agent Configuration", ""]
    claude_md_lines.append(f"Task: {task.get('title', '')}")
    claude_md_lines.append(f"Skills: {', '.join(skills)}")
    claude_md_lines.append("")

    for skill_name in skills:
        instructions = SKILL_INSTRUCTIONS.get(skill_name)
        if instructions:
            claude_md_lines.append(f"## {skill_name}")
            claude_md_lines.append(instructions)
            claude_md_lines.append("")

    claude_md_path = os.path.join(worktree, "CLAUDE.md")
    Path(claude_md_path).write_text("\n".join(claude_md_lines))

    return extra_flags


def configure_cursor(
    worktree: str,
    task: dict,
    skills: list[str],
    mcp_urls: list[dict],
):
    """Configure Cursor CLI with .cursorrules and MCP config."""
    # ── .cursorrules ────────────────────────────────────────────────────
    rules_lines = []
    rules_lines.append(f"# Task: {task.get('title', '')}")
    rules_lines.append(f"# Skills: {', '.join(skills)}")
    rules_lines.append("")

    for skill_name in skills:
        instructions = SKILL_INSTRUCTIONS.get(skill_name)
        if instructions:
            rules_lines.append(f"# {skill_name}")
            rules_lines.append(instructions)
            rules_lines.append("")

    Path(os.path.join(worktree, ".cursorrules")).write_text("\n".join(rules_lines))

    # ── MCP config for Cursor ───────────────────────────────────────────
    if mcp_urls:
        mcp_config = {"mcpServers": {}}
        for mcp in mcp_urls:
            mcp_config["mcpServers"][mcp["name"]] = {
                "url": mcp["url"],
            }
        Path(os.path.join(worktree, ".cursor", "mcp.json")).parent.mkdir(parents=True, exist_ok=True)
        Path(os.path.join(worktree, ".cursor", "mcp.json")).write_text(
            json.dumps(mcp_config, indent=2))


def configure_gemini(
    worktree: str,
    task: dict,
    skills: list[str],
):
    """Configure Gemini CLI with system instruction file."""
    instructions = [f"Task: {task.get('title', '')}"]
    instructions.append(f"Skills: {', '.join(skills)}")
    instructions.append("")

    for skill_name in skills:
        inst = SKILL_INSTRUCTIONS.get(skill_name)
        if inst:
            instructions.append(inst)

    Path(os.path.join(worktree, ".gemini_instructions")).write_text("\n".join(instructions))


def configure_aider(
    worktree: str,
    task: dict,
    skills: list[str],
):
    """Configure Aider with .aider.conf.yml."""
    config = {
        "auto-commits": False,
        "yes": True,
    }

    conventions = []
    for skill_name in skills:
        inst = SKILL_INSTRUCTIONS.get(skill_name)
        if inst:
            conventions.append(inst)

    if conventions:
        config["conventions"] = "\n".join(conventions)

    import yaml
    try:
        Path(os.path.join(worktree, ".aider.conf.yml")).write_text(
            yaml.dump(config, default_flow_style=False))
    except ImportError:
        # No yaml module — write as JSON
        Path(os.path.join(worktree, ".aider.conf.json")).write_text(
            json.dumps(config, indent=2))


# ── Skill-specific instructions per framework ──────────────────────────────

SKILL_INSTRUCTIONS = {
    "python-development": (
        "Use Python 3.11+. Always add type hints to function signatures. "
        "Use pathlib for file operations. Prefer dataclasses over plain dicts for structured data."
    ),
    "rust-development": (
        "Use idiomatic Rust. Prefer Result<T, E> over panics. "
        "Run cargo fmt and cargo clippy before committing."
    ),
    "typescript-development": (
        "Use strict TypeScript. Define interfaces for all data structures. "
        "Use async/await, not callbacks."
    ),
    "pytest-testing": (
        "Use pytest with parametrize for multiple test cases. "
        "Test edge cases: empty input, None, boundary values, error conditions. "
        "Use fixtures for shared setup. Aim for >80% coverage."
    ),
    "jest-testing": (
        "Use Jest with describe/it blocks. Mock external dependencies. "
        "Test both success and error paths."
    ),
    "data-analysis-pandas": (
        "Use pandas for data loading and transformation. "
        "Always validate DataFrame columns before processing. "
        "Handle missing values explicitly (don't silently drop). "
        "Use .copy() when modifying DataFrames to avoid SettingWithCopyWarning."
    ),
    "sql-database": (
        "Use parameterized queries (never f-strings for SQL). "
        "Always close connections/cursors. Use connection pooling for web services. "
        "Add database migrations for schema changes."
    ),
    "rest-api-development": (
        "Use FastAPI with Pydantic models for request/response validation. "
        "Add OpenAPI documentation. Return proper HTTP status codes. "
        "Add health check endpoint at GET /health."
    ),
    "docker-management": (
        "Use multi-stage builds. Pin base image versions. "
        "Don't run as root. Use .dockerignore."
    ),
    "kubernetes-management": (
        "Define resource requests and limits. Use readiness/liveness probes. "
        "Use ConfigMaps for config, Secrets for credentials."
    ),
    "ota-pricing-analysis": (
        "When analyzing hotel rates, consider seasonality (weekday vs weekend, "
        "holiday periods). Use rolling windows for baseline calculation. "
        "Z-score threshold of 2.0 is standard; flag but don't auto-correct."
    ),
    "charging-optimization": (
        "Validate SoC bounds (0-100%). Cap charging at battery capacity. "
        "Consider charger power limits. Handle impossible constraints gracefully "
        "(raise ValueError with clear message, don't silently return bad results)."
    ),
    "github-pr-management": (
        "Create PRs with clear descriptions linking to the issue. "
        "Use conventional commit messages. Keep PRs focused on one change."
    ),
    "git-operations": (
        "Commit frequently with descriptive messages. "
        "Don't commit generated files or secrets."
    ),
    "web-search": (
        "Search for documentation and examples before implementing. "
        "Cite sources when using external patterns."
    ),
    "noether-compose": (
        "Use Noether stages for any computation that could be reused. "
        "Search existing stages before creating new ones."
    ),
}


def print_config_summary(worktree: str, framework: str):
    """Show what config files were generated."""
    configs = {
        "CLAUDE.md": os.path.exists(os.path.join(worktree, "CLAUDE.md")),
        ".mcp.json": os.path.exists(os.path.join(worktree, ".mcp.json")),
        ".cursorrules": os.path.exists(os.path.join(worktree, ".cursorrules")),
        ".gemini_instructions": os.path.exists(os.path.join(worktree, ".gemini_instructions")),
    }
    generated = [k for k, v in configs.items() if v]
    if generated:
        print(f"    config: [{', '.join(generated)}]")


# ── Self-test ───────────────────────────────────────────────────────────────

if __name__ == "__main__":
    import tempfile

    with tempfile.TemporaryDirectory() as worktree:
        task = {
            "id": "api-task",
            "title": "Build FastAPI endpoint with PostgreSQL",
            "skills": ["rest-api-development", "sql-database", "pytest-testing", "github-pr-management"],
            "mcp_urls": [
                {"name": "sql-database", "url": "postgresql://localhost:5432/mydb"},
                {"name": "github-pr-management", "url": "https://github.mcp.claude.com/mcp"},
            ],
            "model": "balanced",
        }

        print("=== Claude Code Configuration ===")
        result = configure_agent(worktree, task, "claude-code")
        print(f"  Extra flags: {result['extra_flags']}")
        print()

        print("--- CLAUDE.md ---")
        print(Path(os.path.join(worktree, "CLAUDE.md")).read_text())

        print("--- .mcp.json ---")
        print(Path(os.path.join(worktree, ".mcp.json")).read_text())

        print()
        print("=== Cursor Configuration ===")
        configure_agent(worktree, task, "cursor-cli")
        print("--- .cursorrules ---")
        print(Path(os.path.join(worktree, ".cursorrules")).read_text()[:300])
