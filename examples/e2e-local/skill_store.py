"""
Skill/MCP Store — registry of abilities that can be assigned to agents.

Each skill has:
- What it does (description)
- How it's provided (MCP server, built-in, CLI tool)
- Which frameworks support it
- What Nix packages it needs
- What credentials it requires
"""
import json
import os
from pathlib import Path

STORE_FILE = os.path.join(
    os.environ.get("WORK", "/tmp/caloron-full-loop"),
    "skill_store.json"
)


class Skill:
    def __init__(self, name: str, data: dict):
        self.name = name
        self.type = data.get("type", "skill")  # skill, mcp, tool
        self.description = data.get("description", "")
        self.frameworks = data.get("frameworks", [])
        self.mcp_url = data.get("mcp_url", "")
        self.nix_packages = data.get("nix_packages", [])
        self.credentials = data.get("credentials", [])
        self.tags = data.get("tags", [])

    def to_dict(self) -> dict:
        return {
            "type": self.type,
            "description": self.description,
            "frameworks": self.frameworks,
            "mcp_url": self.mcp_url,
            "nix_packages": self.nix_packages,
            "credentials": self.credentials,
            "tags": self.tags,
        }

    def supports_framework(self, framework: str) -> bool:
        return framework in self.frameworks


class SkillStore:
    def __init__(self, path: str = STORE_FILE):
        self.path = path
        self.skills: dict[str, Skill] = {}
        self._load()
        if not self.skills:
            self._load_defaults()

    def _load(self):
        if os.path.exists(self.path):
            content = Path(self.path).read_text().strip()
            if content:
                data = json.loads(content)
                for name, sdata in data.items():
                    self.skills[name] = Skill(name, sdata)

    def _save(self):
        Path(self.path).parent.mkdir(parents=True, exist_ok=True)
        data = {name: skill.to_dict() for name, skill in self.skills.items()}
        Path(self.path).write_text(json.dumps(data, indent=2))

    def register(self, name: str, data: dict):
        self.skills[name] = Skill(name, data)
        self._save()

    def get(self, name: str) -> Skill | None:
        return self.skills.get(name)

    def search(self, query: str = "", tags: list[str] = None, framework: str = None) -> list[Skill]:
        results = list(self.skills.values())
        if query:
            q = query.lower()
            results = [s for s in results if q in s.name.lower() or q in s.description.lower()]
        if tags:
            results = [s for s in results if any(t in s.tags for t in tags)]
        if framework:
            results = [s for s in results if s.supports_framework(framework)]
        return results

    def list_all(self) -> list[Skill]:
        return list(self.skills.values())

    def _load_defaults(self):
        """Load built-in skills."""
        defaults = {
            # ── Git / Code Management ──────────────────────────────
            "github-pr-management": {
                "type": "mcp",
                "description": "Create/review PRs, manage issues, read repos on GitHub/Gitea",
                "frameworks": ["claude-code", "cursor-cli", "gemini-cli"],
                "mcp_url": "https://github.mcp.claude.com/mcp",
                "credentials": ["GITHUB_TOKEN"],
                "tags": ["git", "code", "collaboration"],
            },
            "git-operations": {
                "type": "skill",
                "description": "Git clone, branch, commit, push, merge, rebase",
                "frameworks": ["claude-code", "cursor-cli", "gemini-cli", "aider", "codex-cli"],
                "nix_packages": ["git"],
                "tags": ["git", "code"],
            },

            # ── Code Writing ───────────────────────────────────────
            "python-development": {
                "type": "skill",
                "description": "Write Python code with type hints, use pip/poetry",
                "frameworks": ["claude-code", "cursor-cli", "gemini-cli", "aider", "codex-cli"],
                "nix_packages": ["python311"],
                "tags": ["code", "python"],
            },
            "rust-development": {
                "type": "skill",
                "description": "Write Rust code, use cargo, clippy",
                "frameworks": ["claude-code", "cursor-cli", "aider"],
                "nix_packages": ["rustc", "cargo", "clippy"],
                "tags": ["code", "rust"],
            },
            "typescript-development": {
                "type": "skill",
                "description": "Write TypeScript/JavaScript, use npm/pnpm",
                "frameworks": ["claude-code", "cursor-cli", "gemini-cli", "aider"],
                "nix_packages": ["nodejs_20"],
                "tags": ["code", "typescript", "javascript", "frontend"],
            },

            # ── Testing ────────────────────────────────────────────
            "pytest-testing": {
                "type": "skill",
                "description": "Write and run pytest tests with parametrize, fixtures, mocks",
                "frameworks": ["claude-code", "cursor-cli", "gemini-cli", "aider"],
                "nix_packages": ["python311"],
                "tags": ["testing", "python"],
            },
            "jest-testing": {
                "type": "skill",
                "description": "Write and run Jest tests for TypeScript/JavaScript",
                "frameworks": ["claude-code", "cursor-cli"],
                "nix_packages": ["nodejs_20"],
                "tags": ["testing", "typescript", "javascript"],
            },

            # ── Data / ML ─────────────────────────────────────────
            "data-analysis-pandas": {
                "type": "skill",
                "description": "Load, transform, analyze data with pandas, numpy",
                "frameworks": ["claude-code", "cursor-cli", "gemini-cli"],
                "nix_packages": ["python311"],
                "tags": ["data", "python", "analytics"],
            },
            "sql-database": {
                "type": "mcp",
                "description": "Query and modify PostgreSQL/SQLite databases",
                "frameworks": ["claude-code", "cursor-cli"],
                "mcp_url": "postgresql://...",
                "nix_packages": ["postgresql_16"],
                "credentials": ["DATABASE_URL"],
                "tags": ["data", "database"],
            },

            # ── Web / API ──────────────────────────────────────────
            "web-search": {
                "type": "skill",
                "description": "Search the web for information and documentation",
                "frameworks": ["claude-code", "gemini-cli"],
                "tags": ["research", "web"],
            },
            "browser-automation": {
                "type": "skill",
                "description": "Browse websites, extract content, take screenshots",
                "frameworks": ["claude-code", "cursor-cli"],
                "tags": ["research", "web", "browser"],
            },
            "rest-api-development": {
                "type": "skill",
                "description": "Build REST APIs with FastAPI/Flask/Express",
                "frameworks": ["claude-code", "cursor-cli", "gemini-cli", "aider"],
                "nix_packages": ["python311"],
                "tags": ["api", "web", "backend"],
            },

            # ── Infrastructure ─────────────────────────────────────
            "docker-management": {
                "type": "skill",
                "description": "Write Dockerfiles, docker-compose, manage containers",
                "frameworks": ["claude-code", "cursor-cli"],
                "nix_packages": [],
                "tags": ["infra", "devops", "docker"],
            },
            "kubernetes-management": {
                "type": "skill",
                "description": "Write K8s manifests, Helm charts, kubectl operations",
                "frameworks": ["claude-code", "cursor-cli"],
                "nix_packages": [],
                "tags": ["infra", "devops", "kubernetes"],
            },

            # ── Noether ────────────────────────────────────────────
            "noether-compose": {
                "type": "mcp",
                "description": "Compose and run verified computation stages via Noether",
                "frameworks": ["claude-code"],
                "mcp_url": "http://localhost:8080/mcp",
                "tags": ["computation", "noether", "verification"],
            },

            # ── Domain: OTA ────────────────────────────────────────
            "ota-pricing-analysis": {
                "type": "skill",
                "description": "Analyze hotel rates, detect anomalies, seasonal decomposition",
                "frameworks": ["claude-code", "gemini-cli"],
                "nix_packages": ["python311"],
                "tags": ["ota", "pricing", "analytics", "domain"],
            },

            # ── Domain: Electromobility ────────────────────────────
            "charging-optimization": {
                "type": "skill",
                "description": "Optimize charging schedules, SoC management, fleet scheduling",
                "frameworks": ["claude-code", "gemini-cli"],
                "nix_packages": ["python311"],
                "tags": ["electromobility", "optimization", "domain"],
            },

            # ── Communication ──────────────────────────────────────
            "slack-messaging": {
                "type": "mcp",
                "description": "Send messages, read channels, search Slack",
                "frameworks": ["claude-code"],
                "mcp_url": "slack://...",
                "credentials": ["SLACK_TOKEN"],
                "tags": ["communication", "slack"],
            },
            "jira-management": {
                "type": "mcp",
                "description": "Create/update Jira tickets, manage sprints",
                "frameworks": ["claude-code"],
                "mcp_url": "https://jira.company.com/mcp",
                "credentials": ["JIRA_TOKEN"],
                "tags": ["project-management", "jira"],
            },
        }

        for name, data in defaults.items():
            self.skills[name] = Skill(name, data)
        self._save()


def print_store(store: SkillStore, framework: str = None):
    skills = store.search(framework=framework) if framework else store.list_all()
    by_tag: dict[str, list[Skill]] = {}
    for s in skills:
        tag = s.tags[0] if s.tags else "other"
        by_tag.setdefault(tag, []).append(s)

    for tag in sorted(by_tag):
        print(f"\n  [{tag}]")
        for s in sorted(by_tag[tag], key=lambda x: x.name):
            fws = ", ".join(s.frameworks[:3])
            icon = "⚡" if s.type == "mcp" else "🔧"
            print(f"    {icon} {s.name:<30} {s.description[:50]}")
            print(f"       frameworks: [{fws}]")


if __name__ == "__main__":
    store = SkillStore("/tmp/test_skills.json")
    print("=== All Skills ===")
    print_store(store)
    print(f"\n  Total: {len(store.list_all())} skills")

    print("\n=== Skills for claude-code ===")
    print_store(store, framework="claude-code")

    print("\n=== Skills for aider ===")
    print_store(store, framework="aider")

    print("\n=== Search: 'database' ===")
    for s in store.search("database"):
        print(f"  {s.name}: {s.description}")
