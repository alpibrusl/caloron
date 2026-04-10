"""
HR Agent — assigns skills, framework, and model to each task in the DAG.

Runs after the PO generates the DAG but before agents spawn.
Analyzes each task's requirements and matches them to available skills.
"""
import json
import os
import subprocess
import re
from pathlib import Path

from skill_store import SkillStore, Skill


SANDBOX = os.environ.get("SANDBOX", "scripts/sandbox-agent.sh")
WORK = os.environ.get("WORK", "/tmp/caloron-full-loop")

# Keywords that map to skills
SKILL_KEYWORDS = {
    "python": "python-development",
    "pandas": "data-analysis-pandas",
    "numpy": "data-analysis-pandas",
    "csv": "data-analysis-pandas",
    "test": "pytest-testing",
    "pytest": "pytest-testing",
    "jest": "jest-testing",
    "rust": "rust-development",
    "cargo": "rust-development",
    "typescript": "typescript-development",
    "javascript": "typescript-development",
    "react": "typescript-development",
    "api": "rest-api-development",
    "fastapi": "rest-api-development",
    "flask": "rest-api-development",
    "express": "rest-api-development",
    "database": "sql-database",
    "postgres": "sql-database",
    "sql": "sql-database",
    "docker": "docker-management",
    "kubernetes": "kubernetes-management",
    "k8s": "kubernetes-management",
    "helm": "kubernetes-management",
    "anomaly": "ota-pricing-analysis",
    "hotel": "ota-pricing-analysis",
    "rate": "ota-pricing-analysis",
    "pricing": "ota-pricing-analysis",
    "charging": "charging-optimization",
    "soc": "charging-optimization",
    "fleet": "charging-optimization",
    "truck": "charging-optimization",
    "noether": "noether-compose",
    "slack": "slack-messaging",
    "jira": "jira-management",
    "search": "web-search",
    "browse": "browser-automation",
    "github": "github-pr-management",
    "pr": "github-pr-management",
    "git": "git-operations",
}

# Model selection heuristics
STRONG_MODEL_KEYWORDS = [
    "architect", "design", "complex", "optimization", "algorithm",
    "security", "review", "refactor", "migrate",
]
FAST_MODEL_KEYWORDS = [
    "readme", "docs", "documentation", "comment", "format", "lint",
]


def analyze_task_skills(task: dict, store: SkillStore, preferred_framework: str = "claude-code") -> dict:
    """Analyze a task and determine which skills it needs.

    Returns an enriched task with: skills, framework, model, nix_packages, credentials, mcp_urls.
    """
    title = task.get("title", "").lower()
    prompt = task.get("agent_prompt", "").lower()
    text = f"{title} {prompt}"

    # Match keywords to skills
    matched_skills: dict[str, Skill] = {}
    for keyword, skill_name in SKILL_KEYWORDS.items():
        if keyword in text:
            skill = store.get(skill_name)
            if skill and skill.supports_framework(preferred_framework):
                matched_skills[skill_name] = skill

    # Always include git and github for code tasks
    for base in ["git-operations", "github-pr-management"]:
        skill = store.get(base)
        if skill and skill.supports_framework(preferred_framework):
            matched_skills[base] = skill

    # Determine model
    model = "balanced"
    if any(kw in text for kw in STRONG_MODEL_KEYWORDS):
        model = "strong"
    elif any(kw in text for kw in FAST_MODEL_KEYWORDS):
        model = "fast"

    # Collect all nix packages, credentials, mcp urls
    nix_packages = set()
    credentials = set()
    mcp_urls = []
    for skill in matched_skills.values():
        nix_packages.update(skill.nix_packages)
        credentials.update(skill.credentials)
        if skill.mcp_url:
            mcp_urls.append({"name": skill.name, "url": skill.mcp_url})

    return {
        **task,
        "skills": list(matched_skills.keys()),
        "framework": preferred_framework,
        "model": model,
        "nix_packages": sorted(nix_packages),
        "credentials": sorted(credentials),
        "mcp_urls": mcp_urls,
    }


def run_hr_agent(tasks: list[dict], store: SkillStore,
                 preferred_framework: str = "claude-code",
                 use_llm: bool = False) -> list[dict]:
    """Run the HR Agent on all tasks in the DAG.

    If use_llm=True, asks Claude to validate and improve the skill assignments.
    Otherwise, uses keyword matching (fast, no API call).
    """
    enriched = []

    for task in tasks:
        result = analyze_task_skills(task, store, preferred_framework)

        if use_llm:
            # Ask Claude to validate and improve
            skills_list = ", ".join(result["skills"]) or "none detected"
            hr_prompt = f"""You are an HR agent assigning skills to a developer agent.

Task: {task.get('title', '')}
Description: {task.get('agent_prompt', '')[:200]}

Auto-detected skills: [{skills_list}]
Model: {result['model']}
Framework: {result['framework']}

Available skills in the store:
{json.dumps([s.name for s in store.list_all()], indent=2)}

Review the assignment. If the auto-detection missed a skill, add it.
If the model should be different, change it.

Respond ONLY with a JSON object:
{{"skills": ["skill1", "skill2"], "model": "balanced|strong|fast", "notes": "why"}}"""

            try:
                hr_result = subprocess.run(
                    [SANDBOX, f"{WORK}/project", "claude", "-p", hr_prompt,
                     "--dangerously-skip-permissions"],
                    capture_output=True, text=True, timeout=60)

                match = re.search(r"\{.*\}", hr_result.stdout or "", re.DOTALL)
                if match:
                    llm_assignment = json.loads(match.group())
                    # Merge LLM suggestions
                    for skill_name in llm_assignment.get("skills", []):
                        if skill_name not in result["skills"]:
                            skill = store.get(skill_name)
                            if skill:
                                result["skills"].append(skill_name)
                                result["nix_packages"] = sorted(
                                    set(result["nix_packages"] + skill.nix_packages))
                                result["credentials"] = sorted(
                                    set(result["credentials"] + skill.credentials))
                    if llm_assignment.get("model"):
                        result["model"] = llm_assignment["model"]
                    result["hr_notes"] = llm_assignment.get("notes", "")
            except Exception:
                pass  # Fall back to keyword matching

        enriched.append(result)

    return enriched


def print_assignments(tasks: list[dict]):
    """Print the HR Agent's skill assignments."""
    print()
    for t in tasks:
        tid = t.get("id", "?")
        title = t.get("title", "?")
        skills = t.get("skills", [])
        model = t.get("model", "?")
        framework = t.get("framework", "?")
        nix = t.get("nix_packages", [])
        creds = t.get("credentials", [])
        mcps = t.get("mcp_urls", [])

        print(f"  {tid}: {title}")
        print(f"    framework: {framework} | model: {model}")
        print(f"    skills:    [{', '.join(skills)}]")
        if nix:
            print(f"    nix:       [{', '.join(nix)}]")
        if creds:
            print(f"    creds:     [{', '.join(creds)}]")
        if mcps:
            for mcp in mcps:
                print(f"    mcp:       {mcp['name']} → {mcp['url']}")
        if t.get("hr_notes"):
            print(f"    notes:     {t['hr_notes']}")
        print()


# ── Self-test ───────────────────────────────────────────────────────────────

if __name__ == "__main__":
    store = SkillStore("/tmp/test_skills.json")

    # Simulate tasks from different domains
    tasks = [
        {
            "id": "impl",
            "title": "Implement hotel rate anomaly detector with pandas",
            "agent_prompt": "Create src/anomaly_detector.py using pandas for CSV loading, rolling z-score for anomaly detection.",
        },
        {
            "id": "api",
            "title": "Build FastAPI endpoint for anomaly queries",
            "agent_prompt": "Create src/api.py with POST /anomalies endpoint, connect to PostgreSQL database.",
        },
        {
            "id": "tests",
            "title": "Write pytest tests for anomaly detector",
            "agent_prompt": "Create tests/test_anomaly_detector.py with parametrized tests.",
        },
        {
            "id": "charging",
            "title": "Implement multi-truck charging optimizer",
            "agent_prompt": "Create src/optimizer.py with sliding window algorithm, SoC validation, fleet scheduling.",
        },
        {
            "id": "docs",
            "title": "Write README documentation",
            "agent_prompt": "Create README.md with usage examples and installation.",
        },
    ]

    print("=== HR Agent: Skill Assignments ===")
    enriched = run_hr_agent(tasks, store)
    print_assignments(enriched)

    print("=== Summary ===")
    all_skills = set()
    for t in enriched:
        all_skills.update(t.get("skills", []))
    print(f"  Tasks: {len(enriched)}")
    print(f"  Unique skills needed: {len(all_skills)}")
    print(f"  Skills: {', '.join(sorted(all_skills))}")
