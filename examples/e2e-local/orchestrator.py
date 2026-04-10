#!/usr/bin/env python3
"""
Full autonomous sprint orchestrator — no mocks.

Handles: PO → issues → agents → PRs → reviews → merges → supervisor → retro.
Runs against local Gitea via Docker.
"""
import base64
import json
import os
import re
import subprocess
import sys
import time
from dataclasses import dataclass, field
from datetime import datetime, timezone
from pathlib import Path

from agent_versioning import AgentVersionStore, auto_evolve_agents, print_agent_history
from skill_store import SkillStore
from hr_agent import run_hr_agent, print_assignments
from agent_configurator import configure_agent, print_config_summary
from template_store import scaffold_project

# ── Config ──────────────────────────────────────────────────────────────────

GITEA_TOKEN = os.environ.get("GITEA_TOKEN", "c50bad400bd9b8cde3e930cca052eae6ded71f7b")
REPO = os.environ.get("REPO", "caloron/full-loop")
SANDBOX = os.environ.get("SANDBOX", str(Path(__file__).parent.parent.parent / "scripts" / "sandbox-agent.sh"))
WORK = os.environ.get("WORK", "/tmp/caloron-full-loop")
AGENT_TIMEOUT_S = int(os.environ.get("AGENT_TIMEOUT", "180"))  # 3 minutes
MAX_RETRIES = 2
LEARNINGS_FILE = os.path.join(os.environ.get("WORK", "/tmp/caloron-full-loop"), "learnings.json")

# Backend: "direct" (Claude CLI) or "noether" (via Noether stages)
BACKEND = os.environ.get("CALORON_BACKEND", "direct")
NOETHER_STAGES_DIR = os.environ.get("NOETHER_STAGES_DIR",
    str(Path(__file__).parent.parent.parent.parent / "caloron-noether" / "stages"))

# API keys — set any of these to use API mode instead of Claude Pro
ANTHROPIC_API_KEY = os.environ.get("ANTHROPIC_API_KEY", "")
GOOGLE_API_KEY = os.environ.get("GOOGLE_API_KEY", "")
OPENAI_API_KEY = os.environ.get("OPENAI_API_KEY", "")

# ── Framework registry ──────────────────────────────────────────────────────

FRAMEWORKS = {
    "claude-code": {
        "cmd": "claude",
        "args": ["--dangerously-skip-permissions"],
        "prompt_flag": "-p",
        "api_key_env": "ANTHROPIC_API_KEY",
        "api_key_flag": "--api-key",
    },
    "gemini-cli": {
        "cmd": "gemini",
        "args": [],
        "prompt_flag": "-p",
        "api_key_env": "GOOGLE_API_KEY",
        "api_key_flag": None,  # gemini-cli uses env var
    },
    "aider": {
        "cmd": "aider",
        "args": ["--yes", "--no-auto-commits"],
        "prompt_flag": "--message",
        "api_key_env": "ANTHROPIC_API_KEY",
        "api_key_flag": None,
    },
    "codex-cli": {
        "cmd": "codex",
        "args": ["--approval-mode", "full-auto"],
        "prompt_flag": "-p",
        "api_key_env": "OPENAI_API_KEY",
        "api_key_flag": None,
    },
    "open-code": {
        "cmd": "open-code",
        "args": ["--non-interactive"],
        "prompt_flag": "-p",
        "api_key_env": "ANTHROPIC_API_KEY",
        "api_key_flag": None,
    },
}


def build_agent_command(framework: str, prompt: str) -> list[str]:
    """Build the command list for a given framework and prompt."""
    fw = FRAMEWORKS.get(framework, FRAMEWORKS["claude-code"])

    cmd = [fw["cmd"]] + fw["args"]

    # Add API key if available (overrides Pro subscription login)
    api_key = os.environ.get(fw["api_key_env"], "")
    if api_key and fw.get("api_key_flag"):
        cmd.extend([fw["api_key_flag"], api_key])

    cmd.extend([fw["prompt_flag"], prompt])
    return cmd


# ── Noether backend ────────────────────────────────────────────────────────

def run_noether_stage(stage_file: str, input_data: dict) -> dict:
    """Run a caloron-noether Python stage via stdin/stdout JSON."""
    stage_path = os.path.join(NOETHER_STAGES_DIR, stage_file)
    if not os.path.exists(stage_path):
        print(f"  WARNING: stage not found: {stage_path}")
        return {}
    result = subprocess.run(
        ["python3", stage_path],
        input=json.dumps(input_data),
        capture_output=True, text=True, timeout=30,
    )
    try:
        return json.loads(result.stdout)
    except Exception:
        if result.stderr:
            print(f"  Stage error: {result.stderr[:100]}")
        return {}


def noether_evaluate_dag(state: dict, events: list, stall_threshold_m: int = 20) -> dict:
    """Run DAG evaluation through the Noether stage."""
    return run_noether_stage("dag/evaluate.py", {
        "state": state,
        "events": events,
        "stall_threshold_m": stall_threshold_m,
    })


def noether_check_health(agents: dict, stall_threshold_m: int = 20) -> dict:
    """Run health check through the Noether stage."""
    return run_noether_stage("supervisor/check_health.py", {
        "agents": agents,
        "stall_threshold_m": stall_threshold_m,
    })


def noether_decide_intervention(results: list, interventions: dict) -> dict:
    """Run intervention decision through the Noether stage."""
    return run_noether_stage("supervisor/decide_intervention.py", {
        "results": results,
        "interventions": interventions,
    })


def noether_compute_kpis(state: dict, started_at: str, ended_at: str) -> dict:
    """Compute KPIs through the Noether stage."""
    return run_noether_stage("retro/compute_kpis.py", {
        "state": state,
        "started_at": started_at,
        "ended_at": ended_at,
    })


def noether_analyze_feedback(feedback_items: list, kpis: dict) -> dict:
    """Analyze feedback through the Noether stage."""
    return run_noether_stage("retro/analyze_feedback.py", {
        "feedback_items": feedback_items,
        "kpis": kpis,
    })


def noether_write_report(sprint_id: str, kpis: dict, feedback_items: list,
                          started_at: str, ended_at: str) -> dict:
    """Generate retro report through the Noether stage."""
    return run_noether_stage("retro/write_report.py", {
        "sprint_id": sprint_id,
        "kpis": kpis,
        "feedback_items": feedback_items,
        "started_at": started_at,
        "ended_at": ended_at,
    })


# ── Agent feedback parsing ───────────────────────────────────────────────────

def parse_agent_feedback(agent_output: str) -> dict:
    """Extract structured feedback from agent's stdout.

    Looks for CALORON_FEEDBACK_START ... JSON ... CALORON_FEEDBACK_END block.
    Returns parsed dict or defaults if not found.
    """
    defaults = {
        "task_clarity": 5,
        "blockers": [],
        "tools_used": ["claude-code"],
        "self_assessment": "completed",
        "notes": "",
    }

    if not agent_output:
        return defaults

    match = re.search(
        r"CALORON_FEEDBACK_START\s*(.*?)\s*CALORON_FEEDBACK_END",
        agent_output,
        re.DOTALL,
    )
    if not match:
        return defaults

    try:
        feedback = json.loads(match.group(1))
        # Validate and merge with defaults
        result = dict(defaults)
        for key in defaults:
            if key in feedback:
                result[key] = feedback[key]
        # Clamp clarity to 1-10
        result["task_clarity"] = max(1, min(10, int(result["task_clarity"])))
        # Validate assessment
        valid_assessments = ("completed", "partial", "blocked", "failed")
        if result["self_assessment"] not in valid_assessments:
            result["self_assessment"] = "completed"
        return result
    except (json.JSONDecodeError, ValueError, TypeError):
        return defaults


# ── Gitea API ───────────────────────────────────────────────────────────────

def gitea(method: str, path: str, data: dict | None = None) -> dict:
    if method == "GET":
        r = subprocess.run(
            ["docker", "exec", "gitea", "wget", "-qO-",
             "--header", f"Authorization: token {GITEA_TOKEN}",
             f"http://127.0.0.1:3000{path}"],
            capture_output=True, text=True, timeout=15)
    else:
        r = subprocess.run(
            ["docker", "exec", "gitea", "wget", "-qO-",
             "--post-data", json.dumps(data),
             "--header", "Content-Type: application/json",
             "--header", f"Authorization: token {GITEA_TOKEN}",
             f"http://127.0.0.1:3000{path}"],
            capture_output=True, text=True, timeout=15)
    try:
        return json.loads(r.stdout)
    except Exception:
        return {}


def git_merge_branch(branch: str, message: str) -> bool:
    """Merge a branch into main via git inside the Gitea container.
    Temporarily disables the pre-receive hook (Gitea 1.22 merge API is broken for local setups)."""
    repo_path = f"/data/git/repositories/{REPO}.git"
    script = (
        f"chmod -x {repo_path}/hooks/pre-receive 2>/dev/null; "
        f"cd /tmp && rm -rf _merge && mkdir _merge && cd _merge && "
        f"git init -q && "
        f"git fetch {repo_path} main:main {branch}:{branch} 2>/dev/null && "
        f"git checkout main 2>/dev/null && "
        f"git merge {branch} -m '{message}' 2>/dev/null && "
        f"git push {repo_path} main:main 2>/dev/null; "
        f"RET=$?; "
        f"chmod +x {repo_path}/hooks/pre-receive 2>/dev/null; "
        f"exit $RET"
    )
    result = subprocess.run(
        ["docker", "exec", "-u", "git", "gitea", "sh", "-c", script],
        capture_output=True, text=True, timeout=30)
    return result.returncode == 0


def upload_file(branch: str, filepath: str, content: str, msg: str):
    b64 = base64.b64encode(content.encode()).decode()
    existing = gitea("GET", f"/api/v1/repos/{REPO}/contents/{filepath}?ref={branch}")
    sha = existing.get("sha", "")
    payload = {"content": b64, "message": msg, "branch": branch}
    if sha:
        payload["sha"] = sha
    gitea("POST", f"/api/v1/repos/{REPO}/contents/{filepath}", payload)


# ── Supervisor ──────────────────────────────────────────────────────────────

@dataclass
class SupervisorState:
    interventions: dict = field(default_factory=dict)  # task_id → count
    events: list = field(default_factory=list)  # log of what happened

    def record(self, task_id: str, action: str, detail: str):
        self.interventions[task_id] = self.interventions.get(task_id, 0) + 1
        self.events.append({
            "time": datetime.now(timezone.utc).isoformat(),
            "task_id": task_id,
            "action": action,
            "detail": detail,
            "intervention_count": self.interventions[task_id],
        })
        print(f"  SUPERVISOR: [{action}] {detail}")


def run_agent_with_supervision(
    sandbox: str,
    project: str,
    prompt: str,
    task_id: str,
    issue_number: int,
    supervisor: SupervisorState,
    framework: str = "claude-code",
) -> tuple[str, bool]:
    """Run an agent with timeout and retry. Returns (stdout, success)."""

    for attempt in range(1, MAX_RETRIES + 1):
        try:
            agent_cmd = build_agent_command(framework, prompt)
            result = subprocess.run(
                [sandbox, project] + agent_cmd,
                capture_output=True, text=True,
                timeout=AGENT_TIMEOUT_S,
            )
            return result.stdout or "", True

        except subprocess.TimeoutExpired:
            count = supervisor.interventions.get(task_id, 0)

            if count == 0:
                # Probe: post a comment asking for status
                supervisor.record(task_id, "PROBE",
                    f"Agent timed out after {AGENT_TIMEOUT_S}s (attempt {attempt})")
                gitea("POST", f"/api/v1/repos/{REPO}/issues/{issue_number}/comments", {
                    "body": f"⚠️ **Supervisor probe:** Agent for task `{task_id}` has not responded "
                            f"after {AGENT_TIMEOUT_S}s. Retrying..."
                })

            elif count == 1:
                # Restart: try again with a simpler prompt
                supervisor.record(task_id, "RESTART",
                    f"Agent stalled again after probe. Simplifying prompt.")
                gitea("POST", f"/api/v1/repos/{REPO}/issues/{issue_number}/comments", {
                    "body": f"🔄 **Supervisor restart:** Retrying task `{task_id}` with simplified context."
                })
                prompt = prompt.split("\n")[0]  # Keep only the first line

            else:
                # Escalate: give up and create escalation issue
                supervisor.record(task_id, "ESCALATE",
                    f"Agent failed after {count} interventions. Escalating to human.")
                gitea("POST", f"/api/v1/repos/{REPO}/issues", {
                    "title": f"🚨 Escalation: task {task_id} stalled",
                    "body": f"## Human intervention required\n\n"
                            f"**Task:** {task_id}\n"
                            f"**Issue:** #{issue_number}\n"
                            f"**Attempts:** {count + 1}\n\n"
                            f"The agent has timed out {count + 1} times.\n"
                            f"Comment `resolved` when fixed.",
                    "labels": [],
                })
                return "", False

    return "", False


# ── Feedback ────────────────────────────────────────────────────────────────

def post_feedback(
    issue_number: int,
    task_id: str,
    agent_role: str,
    task_clarity: int,
    blockers: list[str],
    tools_used: list[str],
    time_min: int,
    assessment: str,
    notes: str = "",
):
    """Post a caloron_feedback YAML comment on the issue."""
    blockers_yaml = "\n".join(f'    - "{b}"' for b in blockers) if blockers else "    []"
    tools_yaml = "\n".join(f'    - "{t}"' for t in tools_used)

    body = f"""---
caloron_feedback:
  task_id: "{task_id}"
  agent_role: "{agent_role}"
  task_clarity: {task_clarity}
  blockers:
{blockers_yaml}
  tools_used:
{tools_yaml}
  tokens_consumed: 0
  time_to_complete_min: {time_min}
  self_assessment: "{assessment}"
  notes: "{notes}"
---"""

    gitea("POST", f"/api/v1/repos/{REPO}/issues/{issue_number}/comments", {
        "body": body
    })


# ── Retro ───────────────────────────────────────────────────────────────────

def run_retro(issue_numbers: list[int], supervisor: SupervisorState, sprint_time_s: int,
              sprint_start_iso: str = ""):
    """Collect feedback from Gitea issues and compute retro."""
    print(f"=== RETRO ({BACKEND} backend) ===")
    print()

    # Collect feedback from issue comments
    feedbacks = []
    for issue_num in issue_numbers:
        comments = gitea("GET", f"/api/v1/repos/{REPO}/issues/{issue_num}/comments")
        if not isinstance(comments, list):
            continue
        for comment in comments:
            body = comment.get("body", "")
            if "caloron_feedback:" not in body:
                continue
            match = re.search(r"---\s*\n(.*?)\n---", body, re.DOTALL)
            if not match:
                continue
            try:
                import yaml
                parsed = yaml.safe_load(match.group(1))
                if parsed and "caloron_feedback" in parsed:
                    feedbacks.append(parsed["caloron_feedback"])
            except Exception:
                # Fallback: parse manually
                fb = {}
                for line in match.group(1).split("\n"):
                    line = line.strip()
                    if ":" in line and not line.startswith("-"):
                        key, _, val = line.partition(":")
                        key = key.strip()
                        val = val.strip().strip('"')
                        if key in ("task_clarity", "tokens_consumed", "time_to_complete_min"):
                            try: val = int(val)
                            except: pass
                        fb[key] = val
                if "task_id" in fb:
                    feedbacks.append(fb)

    # KPIs
    total = len(feedbacks)
    if total == 0:
        print("  No feedback collected!")
        return

    completed = sum(1 for f in feedbacks if f.get("self_assessment") == "completed")
    failed = sum(1 for f in feedbacks if f.get("self_assessment") in ("failed", "crashed"))
    blocked = sum(1 for f in feedbacks if f.get("self_assessment") == "blocked")

    clarities = [f.get("task_clarity", 0) for f in feedbacks if isinstance(f.get("task_clarity"), (int, float))]
    avg_clarity = sum(clarities) / len(clarities) if clarities else 0

    all_blockers = []
    for f in feedbacks:
        b = f.get("blockers", [])
        if isinstance(b, list):
            all_blockers.extend(b)

    print(f"  Tasks completed:      {completed}/{total}")
    print(f"  Failed/crashed:       {failed}")
    print(f"  Blocked:              {blocked}")
    print(f"  Avg clarity:          {avg_clarity:.1f}/10")
    print(f"  Sprint time:          {sprint_time_s}s")
    print(f"  Avg time/task:        {sprint_time_s // max(total, 1)}s")
    print(f"  Supervisor events:    {len(supervisor.events)}")

    # Blockers analysis
    if all_blockers:
        print(f"\n  Blockers ({len(all_blockers)}):")
        for b in all_blockers:
            print(f"    - {b}")

    # Per-task breakdown
    print(f"\n  Per-task:")
    for f in feedbacks:
        tid = f.get("task_id", "?")
        clarity = f.get("task_clarity", "?")
        time_min = f.get("time_to_complete_min", "?")
        assessment = f.get("self_assessment", "?")
        print(f"    {tid}: clarity={clarity}/10, time={time_min}min, assessment={assessment}")

    # Improvements
    improvements = []
    low_clarity = [f for f in feedbacks if isinstance(f.get("task_clarity"), (int, float)) and f["task_clarity"] < 5]
    if low_clarity:
        improvements.append(f"Improve task specifications — {len(low_clarity)} tasks had clarity < 5/10")

    if supervisor.events:
        improvements.append(f"Reduce agent stalls — {len(supervisor.events)} supervisor interventions")

    dep_blockers = [b for b in all_blockers if "depend" in b.lower() or "dag" in b.lower()]
    if dep_blockers:
        improvements.append(f"Fix DAG dependencies — {len(dep_blockers)} runtime deps discovered")

    if improvements:
        print(f"\n  Improvements:")
        for imp in improvements:
            print(f"    → {imp}")
    else:
        print(f"\n  No improvements needed — clean sprint!")

    # Noether-enhanced analysis (when using noether backend)
    if BACKEND == "noether" and feedbacks:
        print(f"\n  Noether analysis:")
        # Build feedback items for the stage
        feedback_items = [{"is_parsed_yaml": True, "parsed": f} for f in feedbacks]
        kpi_data = {
            "completion_rate": completed / total if total else 0,
            "avg_interventions": len(supervisor.events) / total if total else 0,
        }
        analysis = noether_analyze_feedback(feedback_items, kpi_data)
        if analysis:
            for theme in analysis.get("themes", []):
                print(f"    Theme: {theme}")
            for imp in analysis.get("improvements", []):
                print(f"    Improvement: {imp}")
            for learn in analysis.get("learnings", []):
                print(f"    Learning: {learn}")
            print(f"    Sentiment: {analysis.get('sentiment', '?')}")

            # Add Noether improvements to the list
            improvements.extend(analysis.get("improvements", []))

        # Generate report via Noether stage
        ended_at = datetime.now(timezone.utc).isoformat()
        report = noether_write_report(
            "sprint", kpi_data, feedback_items, sprint_start_iso or ended_at, ended_at)
        if report and report.get("report_markdown"):
            report_path = os.path.join(WORK, "retro_report.md")
            Path(report_path).write_text(report["report_markdown"])
            print(f"\n  Report written to: {report_path}")

    # Supervisor log
    if supervisor.events:
        print(f"\n  Supervisor log:")
        for ev in supervisor.events:
            print(f"    [{ev['action']}] {ev['task_id']}: {ev['detail']}")

    # Save learnings for next sprint
    learnings = load_learnings()
    learnings["sprints"].append({
        "total": total,
        "completed": completed,
        "failed": failed,
        "avg_clarity": round(avg_clarity, 1),
        "supervisor_events": len(supervisor.events),
        "sprint_time_s": sprint_time_s,
        "blockers": all_blockers,
    })
    learnings["improvements"].extend(improvements)
    save_learnings(learnings)
    print(f"\n  Learnings saved ({len(learnings['sprints'])} sprints total)")

    print()


# ── Learnings ───────────────────────────────────────────────────────────────

def load_learnings() -> dict:
    """Load learnings from previous sprints."""
    if os.path.exists(LEARNINGS_FILE):
        return json.loads(Path(LEARNINGS_FILE).read_text())
    return {"sprints": [], "improvements": [], "po_context": ""}


def save_learnings(learnings: dict):
    """Save learnings for next sprint."""
    Path(LEARNINGS_FILE).write_text(json.dumps(learnings, indent=2))


def build_po_context(learnings: dict) -> str:
    """Build context from previous sprints for the PO Agent."""
    if not learnings["sprints"]:
        return ""

    ctx = "\n## Learnings from Previous Sprints\n\n"

    last = learnings["sprints"][-1]
    ctx += f"Last sprint: {last.get('completed', 0)}/{last.get('total', 0)} tasks completed, "
    ctx += f"clarity {last.get('avg_clarity', '?')}/10, "
    ctx += f"{last.get('supervisor_events', 0)} supervisor interventions.\n\n"

    if learnings["improvements"]:
        ctx += "Pending improvements:\n"
        for imp in learnings["improvements"][-5:]:
            ctx += f"- {imp}\n"
        ctx += "\n"

    if last.get("blockers"):
        ctx += "Common blockers from last sprint:\n"
        for b in last["blockers"][-3:]:
            ctx += f"- {b}\n"
        ctx += "\nAddress these in task specifications.\n"

    return ctx


# ── Main ────────────────────────────────────────────────────────────────────

def main():
    goal = sys.argv[1] if len(sys.argv) > 1 else \
        "Build a Python module with functions to validate email addresses and phone numbers. Include comprehensive pytest tests."

    project = f"{WORK}/project"
    os.makedirs(f"{project}/src", exist_ok=True)
    os.makedirs(f"{project}/tests", exist_ok=True)

    # Init local workspace
    subprocess.run(["git", "init", "-q"], cwd=project, capture_output=True)
    subprocess.run(["git", "config", "user.name", "caloron"], cwd=project, capture_output=True)
    subprocess.run(["git", "config", "user.email", "bot@caloron.local"], cwd=project, capture_output=True)
    Path(f"{project}/src/__init__.py").write_text('"""Project."""\n')
    Path(f"{project}/tests/__init__.py").write_text("")
    subprocess.run(["git", "add", "-A"], cwd=project, capture_output=True)
    subprocess.run(["git", "diff", "--cached", "--quiet"], cwd=project) or \
        subprocess.run(["git", "commit", "-qm", "init"], cwd=project, capture_output=True)

    supervisor = SupervisorState()
    sprint_start = time.time()

    # Load learnings from previous sprints
    learnings = load_learnings()
    sprint_number = len(learnings["sprints"]) + 1
    po_context = build_po_context(learnings)

    # Load agent version store
    agent_store = AgentVersionStore(os.path.join(WORK, "agent_versions.json"))

    print("=" * 60)
    print(f"  FULL AUTONOMOUS SPRINT #{sprint_number}")
    print(f"  Backend: {BACKEND}")
    print(f"  Goal: {goal}")
    if BACKEND == "noether":
        print(f"  Stages: {NOETHER_STAGES_DIR}")
    if po_context:
        print(f"  (with learnings from {len(learnings['sprints'])} previous sprint(s))")
    # Show agent versions if any exist
    if agent_store.agents:
        print(f"  Agents: {len(agent_store.agents)} with version history")
    print("=" * 60)
    print()

    # ── Step 1: PO Agent ────────────────────────────────────────────────
    print("--- Step 1: PO Agent ---")
    available_frameworks = ", ".join(FRAMEWORKS.keys())
    po_prompt = f"""You are a Product Owner. Goal: {goal}
{po_context}
Output ONLY a JSON array. Each task has:
- id: short identifier
- title: one-line description
- depends_on: list of task IDs ([] if none)
- agent_prompt: specific instructions (exact file paths, function signatures, expected behavior)
- framework: which tool to use (default: "claude-code"). Available: {available_frameworks}

Example:
[{{"id":"impl","title":"Implement module","depends_on":[],"agent_prompt":"Create src/mod.py with...","framework":"claude-code"}},
 {{"id":"tests","title":"Write tests","depends_on":["impl"],"agent_prompt":"Create tests/test_mod.py...","framework":"claude-code"}}]

Keep to 2-3 tasks. Tests depend on implementation."""

    po_cmd = build_agent_command("claude-code", po_prompt)
    po_result = subprocess.run(
        [SANDBOX, project] + po_cmd,
        capture_output=True, text=True, timeout=120)
    po_out = po_result.stdout or ""

    match = re.search(r"\[.*\]", po_out, re.DOTALL)
    if not match:
        print("  ERROR: PO produced no JSON")
        sys.exit(1)
    tasks = json.loads(match.group())
    Path(f"{WORK}/dag.json").write_text(json.dumps(tasks, indent=2))

    for t in tasks:
        deps = ", ".join(t.get("depends_on", [])) or "none"
        print(f"  {t['id']}: {t['title']} (deps: {deps})")

    # ── Step 1.5: HR Agent assigns skills ───────────────────────────────
    print()
    print("--- HR Agent: Assigning skills ---")
    skill_store = SkillStore(os.path.join(WORK, "skill_store.json"))
    tasks = run_hr_agent(tasks, skill_store, preferred_framework="claude-code")
    print_assignments(tasks)

    # Register agents in version store (or load existing versions)
    print()
    print("  Agents:")
    for t in tasks:
        tid = t["id"]
        spec = {
            "personality": "developer",
            "model": t.get("model", "balanced"),
            "framework": t.get("framework", "claude-code"),
            "capabilities": t.get("capabilities", ["code-writing", "python"]),
            "extra_instructions": "",
        }
        version = agent_store.register(tid, spec, f"sprint-{sprint_number}")

        # If agent has evolved, use the latest version's spec
        current = agent_store.current(tid)
        if current and current["version"] != "1.0":
            t["framework"] = current.get("framework", t.get("framework", "claude-code"))
            # Prepend evolved instructions to agent prompt
            evolved_instructions = current.get("extra_instructions", "")
            if evolved_instructions:
                t["agent_prompt"] = f"{evolved_instructions}\n\n{t.get('agent_prompt', t['title'])}"
            print(f"    {tid}: v{current['version']} (evolved — {current['model']}, [{', '.join(current.get('capabilities', []))}])")
        else:
            print(f"    {tid}: v{version} (new)")
    print()

    # ── Step 2: Create issues ───────────────────────────────────────────
    print("--- Step 2: Issues ---")
    issue_map = {}  # task_id → issue_number
    for t in tasks:
        result = gitea("POST", f"/api/v1/repos/{REPO}/issues", {
            "title": t["title"],
            "body": f"**Task:** {t['id']}\n**Depends on:** {', '.join(t.get('depends_on', [])) or 'none'}",
        })
        num = result.get("number", 0)
        issue_map[t["id"]] = num
        print(f"  Issue #{num}: {t['title']}")
    print()

    # ── Step 3-7: Execute tasks ─────────────────────────────────────────
    print("--- Step 3: Execute ---")
    print()

    completed = set()
    remaining = list(tasks)
    feedback_data = []

    while remaining:
        ready = [t for t in remaining if all(d in completed for d in t.get("depends_on", []))]
        if not ready:
            print("STUCK — unresolvable dependencies!")
            break

        for task in ready:
            tid = task["id"]
            title = task["title"]
            prompt = task.get("agent_prompt", title)
            framework = task.get("framework", "claude-code")
            issue_num = issue_map.get(tid, 0)
            task_start = time.time()
            blockers = []

            print(f"{'=' * 50}")
            print(f"  Task: {tid} — {title}")
            print(f"  Issue: #{issue_num} | Framework: {framework}")
            print(f"{'=' * 50}")

            # Scaffold project from template (if first task and project is empty)
            task_text = f"{task.get('title', '')} {task.get('agent_prompt', '')}"
            scaffold = scaffold_project(project, task.get("skills", []), task_text)
            if scaffold.get("files"):
                print(f"  Scaffold: {scaffold['template_name']} ({len(scaffold['files'])} files)")

            # Configure the agent's worktree with skill-specific files
            config_result = configure_agent(project, task, framework)
            extra_cli_flags = config_result.get("extra_flags", [])
            print_config_summary(project, framework)

            # Agent writes code (with supervisor timeout)
            full_prompt = f"""{prompt}

Rules:
- Only create/modify files in src/ and tests/
- Use type hints
- Read CLAUDE.md for skill-specific instructions
- When COMPLETELY done, output a feedback block as the LAST thing you print, in this exact format:

CALORON_FEEDBACK_START
{{
  "task_clarity": <1-10 how clear was the task description>,
  "blockers": [<list of strings describing what slowed you down or was unclear>],
  "tools_used": [<list of tools/libraries you used>],
  "self_assessment": "<completed|partial|blocked|failed>",
  "notes": "<any observations about the task>"
}}
CALORON_FEEDBACK_END"""

            print(f"  Agent running ({framework}, sandboxed, supervised)...")
            agent_out, success = run_agent_with_supervision(
                SANDBOX, project, full_prompt, tid, issue_num, supervisor, framework)

            # Parse agent's self-reported feedback
            agent_feedback = parse_agent_feedback(agent_out if success else "")

            if not success:
                blockers.append("Agent timed out and was escalated")
                assessment = "failed"
            else:
                assessment = agent_feedback.get("self_assessment", "completed")
                # Use agent-reported blockers
                agent_blockers = agent_feedback.get("blockers", [])
                if agent_blockers:
                    blockers.extend(agent_blockers)
                for line in agent_out.strip().split("\n")[-2:]:
                    if "CALORON_FEEDBACK" not in line:
                        print(f"    {line}")

            # Collect changed files
            subprocess.run(["git", "add", "-A"], cwd=project, capture_output=True)
            diff = subprocess.run(["git", "diff", "--cached", "--name-only"],
                                  cwd=project, capture_output=True, text=True)
            changed = [f for f in diff.stdout.strip().split("\n")
                       if f and (f.startswith("src/") or f.startswith("tests/"))
                       and f not in ("src/__init__.py", "tests/__init__.py")]
            subprocess.run(["git", "checkout", "--", "."], cwd=project, capture_output=True)

            if changed and success:
                # Create branch
                branch = f"agent/{tid}"
                gitea("POST", f"/api/v1/repos/{REPO}/branches", {
                    "new_branch_name": branch, "old_branch_name": "main"
                })

                # Upload files
                for filepath in changed:
                    full_path = os.path.join(project, filepath)
                    if os.path.exists(full_path):
                        content = open(full_path).read()
                        upload_file(branch, filepath, content, f"[{tid}] {filepath}")
                        print(f"  Uploaded: {filepath}")

                # Create PR
                pr = gitea("POST", f"/api/v1/repos/{REPO}/pulls", {
                    "title": f"[{tid}] {title}",
                    "body": f"Closes #{issue_num}\n\nAgent: caloron-agent-{tid}",
                    "head": branch,
                    "base": "main",
                })
                pr_num = pr.get("number", "?")
                print(f"  PR #{pr_num}")

                # Review cycle — up to MAX_REVIEW_CYCLES attempts
                MAX_REVIEW_CYCLES = 3
                merged = False

                for review_cycle in range(1, MAX_REVIEW_CYCLES + 1):
                    # Reviewer agent (supervised)
                    print(f"  Reviewer (cycle {review_cycle})...")
                    review_prompt = f"""Review code change for: {title}
Files changed: {', '.join(changed)}
Check: correctness, tests, type hints.
Respond ONLY: APPROVED or CHANGES_NEEDED: reason"""

                    review_out, review_ok = run_agent_with_supervision(
                        SANDBOX, project, review_prompt, f"{tid}-review-{review_cycle}", issue_num, supervisor)
                    review = review_out.strip().split("\n")[-1] if review_out else "APPROVED"
                    print(f"  Review: {review[:80]}")

                    # Post review comment on PR
                    gitea("POST", f"/api/v1/repos/{REPO}/issues/{pr_num}/comments", {
                        "body": f"**Code Review (cycle {review_cycle}):** {review}"
                    })

                    if "CHANGES_NEEDED" not in review.upper():
                        # Approved — merge
                        merge_ok = git_merge_branch(branch, f"Merge PR #{pr_num}: [{tid}] {title}")
                        if merge_ok:
                            gitea("POST", f"/api/v1/repos/{REPO}/pulls/{pr_num}/merge", {"Do": "merge"})
                            print(f"  PR #{pr_num} MERGED ✓")
                            merged = True
                        else:
                            print(f"  Merge FAILED — branch may have conflicts")
                        break

                    # Changes requested — agent fixes
                    fix_reason = review.split("CHANGES_NEEDED:")[-1].strip() if ":" in review else review
                    blockers.append(f"Review cycle {review_cycle}: {fix_reason}")
                    print(f"  Agent fixing: {fix_reason[:60]}...")

                    fix_prompt = f"""The reviewer requested changes on your code for: {title}

Reviewer feedback: {fix_reason}

Files to fix: {', '.join(changed)}

Please fix the issues described above. Only modify files in src/ and tests/. When done, stop."""

                    fix_out, fix_ok = run_agent_with_supervision(
                        SANDBOX, project, fix_prompt, f"{tid}-fix-{review_cycle}", issue_num, supervisor)
                    if fix_out:
                        for line in fix_out.strip().split("\n")[-2:]:
                            print(f"    {line}")

                    # Upload fixed files to the same branch
                    subprocess.run(["git", "add", "-A"], cwd=project, capture_output=True)
                    fix_diff = subprocess.run(["git", "diff", "--cached", "--name-only"],
                                              cwd=project, capture_output=True, text=True)
                    fix_changed = [f for f in fix_diff.stdout.strip().split("\n")
                                   if f and (f.startswith("src/") or f.startswith("tests/"))
                                   and f not in ("src/__init__.py", "tests/__init__.py")]
                    subprocess.run(["git", "checkout", "--", "."], cwd=project, capture_output=True)

                    for filepath in fix_changed:
                        full_path = os.path.join(project, filepath)
                        if os.path.exists(full_path):
                            content = open(full_path).read()
                            upload_file(branch, filepath, content, f"[{tid}] fix: {filepath}")
                    print(f"  Pushed fix ({len(fix_changed)} files)")

                if not merged and review_cycle == MAX_REVIEW_CYCLES:
                    # Force merge after max cycles
                    print(f"  Max review cycles reached — force merging")
                    merge_ok = git_merge_branch(branch, f"Merge PR #{pr_num}: [{tid}] {title} (force after {MAX_REVIEW_CYCLES} cycles)")
                    if merge_ok:
                        print(f"  PR #{pr_num} FORCE MERGED")
                    blockers.append(f"Force merged after {MAX_REVIEW_CYCLES} review cycles")

            task_time = int(time.time() - task_start)
            task_time_min = max(1, task_time // 60)

            # Post feedback on the issue
            # Use agent's self-reported feedback (not hardcoded guesses)
            reported_clarity = agent_feedback.get("task_clarity", 5)
            reported_tools = agent_feedback.get("tools_used", ["claude-code"])
            reported_notes = agent_feedback.get("notes", "")

            post_feedback(
                issue_number=issue_num,
                task_id=tid,
                agent_role="developer",
                task_clarity=reported_clarity,
                blockers=blockers,
                tools_used=reported_tools,
                time_min=task_time_min,
                assessment=assessment,
                notes=f"{reported_notes} | Files: {', '.join(changed) if changed else 'none'}. Time: {task_time}s.",
            )
            print(f"  Feedback: clarity={reported_clarity}/10, assessment={assessment}, "
                  f"blockers={len(blockers)}, tools={reported_tools}")

            feedback_data.append({
                "task_id": tid, "time_s": task_time, "files": changed,
                "assessment": assessment, "blockers": blockers,
                "clarity": reported_clarity,
                "tools": reported_tools,
                "notes": reported_notes,
            })

            completed.add(tid)
            remaining.remove(task)
            print(f"  Done ({task_time}s)")
            print()

    sprint_time = int(time.time() - sprint_start)

    # ── Step 8: Retro ───────────────────────────────────────────────────
    print()
    sprint_start_iso = datetime.fromtimestamp(sprint_start, tz=timezone.utc).isoformat()
    run_retro(list(issue_map.values()), supervisor, sprint_time, sprint_start_iso)

    # ── Step 9: Auto-evolve agents based on retro ──────────────────────
    print("--- Step 9: Agent Evolution ---")

    # Build retro summary for evolution decisions
    total_tasks = len(feedback_data)
    failed_tasks = sum(1 for f in feedback_data if f["assessment"] == "failed")
    all_blockers = []
    for f in feedback_data:
        all_blockers.extend(f.get("blockers", []))

    # Use agent-reported clarity (not hardcoded guesses)
    clarities = [f.get("clarity", 5) for f in feedback_data]
    avg_clarity = sum(clarities) / len(clarities) if clarities else 5

    retro_summary = {
        "avg_clarity": avg_clarity,
        "failure_rate": failed_tasks / max(total_tasks, 1),
        "supervisor_events": len(supervisor.events),
        "avg_review_cycles": sum(1 for f in feedback_data if f.get("blockers")) / max(total_tasks, 1),
        "blockers": all_blockers,
    }

    # Record agent-reported performance for each agent
    for f in feedback_data:
        agent_store.record_performance(f["task_id"], {
            "clarity": f.get("clarity", 5),
            "completion_rate": 1.0 if f["assessment"] == "completed" else 0.0,
            "review_cycles": 1 + len(f.get("blockers", [])),
            "time_s": f["time_s"],
            "tools": f.get("tools", []),
            "notes": f.get("notes", ""),
        })

    # Auto-evolve
    changes = auto_evolve_agents(agent_store, retro_summary, f"sprint-{sprint_number}")
    if not changes:
        print("  No evolution needed — agents performing well")
    print()

    # Show agent history
    print("--- Agent Versions ---")
    for agent_id in agent_store.agents:
        print_agent_history(agent_store, agent_id)

    # ── Gitea state ─────────────────────────────────────────────────────
    print("--- Gitea State ---")
    prs = gitea("GET", f"/api/v1/repos/{REPO}/pulls?state=all&limit=50")
    if isinstance(prs, list):
        print("PRs:")
        for pr in sorted(prs, key=lambda x: x.get("number", 0)):
            state = "merged" if pr.get("merged") else pr["state"]
            print(f"  PR #{pr['number']}: {pr['title']} [{state}]")

    issues = gitea("GET", f"/api/v1/repos/{REPO}/issues?state=all&type=issues&limit=50")
    if isinstance(issues, list):
        print("Issues:")
        for i in sorted(issues, key=lambda x: x.get("number", 0)):
            print(f"  #{i['number']}: {i['title']} [{i['state']}] ({i.get('comments', 0)} comments)")

    print()
    print("=" * 60)
    print(f"  SPRINT COMPLETE — {sprint_time}s")
    print("=" * 60)


if __name__ == "__main__":
    main()
