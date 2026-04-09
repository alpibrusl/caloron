"""
Agent versioning: tracks agent evolution across sprints.

Each improvement from a retro creates a new agent version.
Tracks: what changed, why, when, and the full agent spec at each version.
Supports rollback if a change degrades performance.
"""
import json
import os
import copy
from datetime import datetime, timezone
from pathlib import Path
from dataclasses import dataclass, field


AGENTS_FILE = os.path.join(
    os.environ.get("WORK", "/tmp/caloron-full-loop"),
    "agent_versions.json"
)


@dataclass
class AgentChange:
    """A single change to an agent."""
    field: str          # what changed: "model", "prompt", "tools", "stall_threshold"
    old_value: str      # previous value
    new_value: str      # new value
    reason: str         # why (from retro)
    sprint_id: str      # which sprint triggered this


@dataclass
class AgentVersion:
    """A versioned snapshot of an agent's configuration."""
    version: str                    # "1.0", "1.1", etc.
    created_at: str                 # ISO timestamp
    sprint_id: str                  # sprint that created this version
    personality: str
    model: str
    framework: str
    capabilities: list[str]
    extra_instructions: str
    stall_threshold: int
    changes: list[dict]             # changes from previous version
    performance: dict = field(default_factory=dict)  # KPIs at this version


class AgentVersionStore:
    """Persistent store of agent versions."""

    def __init__(self, path: str = AGENTS_FILE):
        self.path = path
        self.agents: dict[str, list[dict]] = {}  # agent_id → list of versions
        self._load()

    def _load(self):
        if os.path.exists(self.path):
            content = Path(self.path).read_text().strip()
            if content:
                self.agents = json.loads(content)

    def _save(self):
        Path(self.path).parent.mkdir(parents=True, exist_ok=True)
        Path(self.path).write_text(json.dumps(self.agents, indent=2))

    def register(self, agent_id: str, spec: dict, sprint_id: str) -> str:
        """Register a new agent or return existing version."""
        if agent_id not in self.agents:
            version = "1.0"
            self.agents[agent_id] = [{
                "version": version,
                "created_at": datetime.now(timezone.utc).isoformat(),
                "sprint_id": sprint_id,
                "personality": spec.get("personality", "developer"),
                "model": spec.get("model", "balanced"),
                "framework": spec.get("framework", "claude-code"),
                "capabilities": spec.get("capabilities", []),
                "extra_instructions": spec.get("extra_instructions", ""),
                "stall_threshold": spec.get("stall_threshold", 20),
                "changes": [{"field": "initial", "reason": "Created by PO Agent"}],
                "performance": {},
            }]
            self._save()
            return version
        return self.agents[agent_id][-1]["version"]

    def current(self, agent_id: str) -> dict | None:
        """Get the current (latest) version of an agent."""
        if agent_id not in self.agents or not self.agents[agent_id]:
            return None
        return self.agents[agent_id][-1]

    def history(self, agent_id: str) -> list[dict]:
        """Get all versions of an agent."""
        return self.agents.get(agent_id, [])

    def evolve(self, agent_id: str, changes: list[dict], sprint_id: str) -> str:
        """Create a new version with the given changes applied.

        changes: list of {"field": "model", "new_value": "strong", "reason": "high failure rate"}
        Returns the new version string.
        """
        if agent_id not in self.agents or not self.agents[agent_id]:
            raise ValueError(f"Agent {agent_id} not registered")

        current = copy.deepcopy(self.agents[agent_id][-1])

        # Bump version
        major, minor = current["version"].split(".")
        new_version = f"{major}.{int(minor) + 1}"

        # Apply changes
        change_records = []
        for change in changes:
            fld = change["field"]
            new_val = change["new_value"]
            reason = change.get("reason", "")

            old_val = current.get(fld, "")

            if fld == "model":
                current["model"] = new_val
            elif fld == "extra_instructions":
                # Append, don't replace
                if current["extra_instructions"]:
                    current["extra_instructions"] += f"\n{new_val}"
                else:
                    current["extra_instructions"] = new_val
            elif fld == "capabilities":
                if isinstance(new_val, str):
                    new_val = [new_val]
                current["capabilities"] = list(set(current["capabilities"] + new_val))
            elif fld == "stall_threshold":
                current["stall_threshold"] = int(new_val)
            elif fld == "framework":
                current["framework"] = new_val

            change_records.append({
                "field": fld,
                "old_value": str(old_val) if not isinstance(old_val, str) else old_val,
                "new_value": str(new_val) if not isinstance(new_val, str) else new_val,
                "reason": reason,
                "sprint_id": sprint_id,
            })

        current["version"] = new_version
        current["created_at"] = datetime.now(timezone.utc).isoformat()
        current["sprint_id"] = sprint_id
        current["changes"] = change_records
        current["performance"] = {}  # reset until measured

        self.agents[agent_id].append(current)
        self._save()

        return new_version

    def record_performance(self, agent_id: str, kpis: dict):
        """Record performance metrics for the current version."""
        if agent_id in self.agents and self.agents[agent_id]:
            self.agents[agent_id][-1]["performance"] = kpis
            self._save()

    def rollback(self, agent_id: str) -> str | None:
        """Rollback to the previous version. Returns the version rolled back to."""
        if agent_id not in self.agents or len(self.agents[agent_id]) < 2:
            return None
        removed = self.agents[agent_id].pop()
        self._save()
        return self.agents[agent_id][-1]["version"]

    def should_evolve(self, agent_id: str, retro_data: dict) -> list[dict]:
        """Analyze retro data and suggest changes for an agent.

        Returns a list of changes to apply, or empty if no evolution needed.
        """
        suggestions = []
        current = self.current(agent_id)
        if not current:
            return suggestions

        perf = current.get("performance", {})
        blockers = retro_data.get("blockers", [])
        clarity = retro_data.get("avg_clarity", 10)
        failure_rate = retro_data.get("failure_rate", 0)
        interventions = retro_data.get("supervisor_events", 0)
        review_cycles = retro_data.get("avg_review_cycles", 0)

        # Rule 1: High failure rate → stronger model
        if failure_rate > 0.3 and current["model"] != "strong":
            suggestions.append({
                "field": "model",
                "new_value": "strong",
                "reason": f"Failure rate {failure_rate:.0%} exceeds 30% threshold",
            })

        # Rule 2: Low clarity → add specification instructions
        if clarity < 5:
            suggestions.append({
                "field": "extra_instructions",
                "new_value": "If the task description is unclear, ask for clarification via an issue comment before starting work.",
                "reason": f"Average clarity {clarity}/10 — agent needs to handle ambiguity better",
            })

        # Rule 3: Frequent reviewer rejections → add self-review instruction
        if review_cycles > 2:
            suggestions.append({
                "field": "extra_instructions",
                "new_value": "Before submitting a PR, review your own code for: missing tests, missing input validation, and missing type hints.",
                "reason": f"Average {review_cycles} review cycles — too many rejections",
            })

        # Rule 4: Frequent stalls → reduce threshold
        if interventions > 1 and current["stall_threshold"] > 10:
            suggestions.append({
                "field": "stall_threshold",
                "new_value": str(max(10, current["stall_threshold"] - 5)),
                "reason": f"{interventions} supervisor interventions — tighter monitoring needed",
            })

        # Rule 5: Missing tool blocker → add capability
        for blocker in blockers:
            bl = blocker.lower()
            if "tool" in bl and "unavailable" in bl:
                # Try to extract tool name
                for tool in ["noether", "browser-research", "rust", "nodejs"]:
                    if tool in bl and tool not in current["capabilities"]:
                        suggestions.append({
                            "field": "capabilities",
                            "new_value": tool,
                            "reason": f"Blocker: {blocker}",
                        })

        return suggestions


def print_agent_history(store: AgentVersionStore, agent_id: str):
    """Print the full evolution history of an agent."""
    history = store.history(agent_id)
    if not history:
        print(f"  No history for {agent_id}")
        return

    print(f"  Agent: {agent_id} ({len(history)} versions)")
    print()

    for v in history:
        version = v["version"]
        sprint = v["sprint_id"]
        model = v["model"]
        caps = ", ".join(v["capabilities"]) if v["capabilities"] else "default"
        perf = v.get("performance", {})

        print(f"  v{version} (sprint: {sprint})")
        print(f"    model: {model} | capabilities: [{caps}]")

        if v.get("extra_instructions"):
            # Show first 80 chars
            instr = v["extra_instructions"].replace("\n", " ")[:80]
            print(f"    instructions: {instr}...")

        if v["changes"]:
            for c in v["changes"]:
                if c["field"] == "initial":
                    print(f"    [created] {c.get('reason', '')}")
                else:
                    print(f"    [{c['field']}] {c.get('old_value', '?')} → {c['new_value']}")
                    print(f"      reason: {c.get('reason', '')}")

        if perf:
            print(f"    performance: clarity={perf.get('clarity','?')}, "
                  f"completion={perf.get('completion_rate','?')}, "
                  f"review_cycles={perf.get('review_cycles','?')}")

        print()


# ── Integration with orchestrator ───────────────────────────────────────────

def auto_evolve_agents(store: AgentVersionStore, retro_data: dict, sprint_id: str):
    """After a retro, check each agent for needed evolution.

    Returns a summary of changes made.
    """
    changes_made = []

    for agent_id in store.agents:
        suggestions = store.should_evolve(agent_id, retro_data)
        if suggestions:
            old_version = store.current(agent_id)["version"]
            new_version = store.evolve(agent_id, suggestions, sprint_id)
            changes_made.append({
                "agent_id": agent_id,
                "old_version": old_version,
                "new_version": new_version,
                "changes": suggestions,
            })
            print(f"  Agent {agent_id}: v{old_version} → v{new_version}")
            for s in suggestions:
                print(f"    [{s['field']}] → {s['new_value']} ({s['reason']})")

    return changes_made


# ── Self-test ───────────────────────────────────────────────────────────────

if __name__ == "__main__":
    import tempfile

    # Demo: simulate agent evolution across 3 sprints
    with tempfile.NamedTemporaryFile(suffix=".json", delete=False) as f:
        store = AgentVersionStore(f.name)

    print("=== Agent Versioning Demo ===\n")

    # Sprint 1: PO creates agents
    store.register("dev-1", {
        "personality": "developer",
        "model": "balanced",
        "framework": "claude-code",
        "capabilities": ["code-writing", "python"],
        "extra_instructions": "Use type hints.",
    }, "sprint-1")
    store.record_performance("dev-1", {
        "clarity": 5.5, "completion_rate": 1.0, "review_cycles": 2
    })
    print("Sprint 1: Agent created")
    print_agent_history(store, "dev-1")

    # Sprint 1 retro: low clarity, high review cycles
    print("Sprint 1 retro → auto-evolve:")
    auto_evolve_agents(store, {
        "avg_clarity": 4,
        "failure_rate": 0,
        "supervisor_events": 0,
        "avg_review_cycles": 3,
        "blockers": [],
    }, "sprint-1")
    print()

    # Sprint 2: agent performs better
    store.record_performance("dev-1", {
        "clarity": 7.0, "completion_rate": 1.0, "review_cycles": 1
    })

    # Sprint 2 retro: failure rate spiked
    print("Sprint 2 retro → auto-evolve:")
    auto_evolve_agents(store, {
        "avg_clarity": 7,
        "failure_rate": 0.4,
        "supervisor_events": 2,
        "avg_review_cycles": 1,
        "blockers": ["noether tool unavailable for data transform"],
    }, "sprint-2")
    print()

    # Sprint 3: too many changes, rollback
    print("Sprint 3: Performance degraded → rollback")
    rolled = store.rollback("dev-1")
    print(f"  Rolled back to v{rolled}\n")

    # Full history
    print("=== Full Agent History ===\n")
    print_agent_history(store, "dev-1")
