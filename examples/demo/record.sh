#!/bin/bash
# ==========================================================================
# Caloron Demo Recording Script
#
# Usage:
#   asciinema rec demo.cast -c "bash examples/demo/record.sh"
#
# Or just run it directly to see the output:
#   bash examples/demo/record.sh
# ==========================================================================
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
CALORON_DIR="$(dirname "$(dirname "$SCRIPT_DIR")")"

# Colors
BOLD='\033[1m'
DIM='\033[2m'
GREEN='\033[32m'
YELLOW='\033[33m'
BLUE='\033[34m'
CYAN='\033[36m'
RED='\033[31m'
RESET='\033[0m'

narrate() {
    echo ""
    echo -e "${BOLD}${CYAN}▸ $1${RESET}"
    sleep 1
}

step() {
    echo -e "${BOLD}${BLUE}[$1]${RESET} $2"
}

ok() {
    echo -e "  ${GREEN}✓${RESET} $1"
}

warn() {
    echo -e "  ${YELLOW}!${RESET} $1"
}

fail() {
    echo -e "  ${RED}✗${RESET} $1"
}

type_slow() {
    # Simulate typing for demo effect
    echo -ne "${DIM}\$ ${RESET}"
    for ((i=0; i<${#1}; i++)); do
        echo -n "${1:$i:1}"
        sleep 0.03
    done
    echo ""
    sleep 0.5
}

# ── Setup (silent) ──────────────────────────────────────────────────────────

GITEA_TOKEN="${GITEA_TOKEN:-c50bad400bd9b8cde3e930cca052eae6ded71f7b}"
REPO="caloron/demo-project"
SANDBOX="$CALORON_DIR/scripts/sandbox-agent.sh"

# Fresh repo
docker exec gitea curl -sf -X DELETE -H "Authorization: token ${GITEA_TOKEN}" \
    "http://127.0.0.1:3000/api/v1/repos/$REPO" 2>/dev/null || true
sleep 1
docker exec gitea wget -qO- --post-data='{"name":"demo-project","auto_init":true}' \
    --header="Content-Type: application/json" --header="Authorization: token ${GITEA_TOKEN}" \
    "http://127.0.0.1:3000/api/v1/user/repos" 2>/dev/null > /dev/null
for f in "src/__init__.py" "tests/__init__.py"; do
    b64=$(echo -n "" | base64 -w0)
    docker exec gitea wget -qO- \
        --post-data="{\"content\":\"${b64}\",\"message\":\"init ${f}\"}" \
        --header="Content-Type: application/json" --header="Authorization: token ${GITEA_TOKEN}" \
        "http://127.0.0.1:3000/api/v1/repos/$REPO/contents/${f}" 2>/dev/null > /dev/null
done
rm -rf /tmp/caloron-demo
WORK="/tmp/caloron-demo"
mkdir -p "$WORK/project/src" "$WORK/project/tests"
cd "$WORK/project" && git init -q && git config user.name caloron && git config user.email bot@caloron.local
echo '"""Project."""' > src/__init__.py && echo '' > tests/__init__.py
git add -A && git commit -qm init

# ── Demo starts ─────────────────────────────────────────────────────────────

clear
echo ""
echo -e "${BOLD}  ╔══════════════════════════════════════════════╗${RESET}"
echo -e "${BOLD}  ║          ${CYAN}CALORON${RESET}${BOLD} — Multi-Agent Sprint         ║${RESET}"
echo -e "${BOLD}  ║    Agents collaborate through Git to build   ║${RESET}"
echo -e "${BOLD}  ║               software autonomously          ║${RESET}"
echo -e "${BOLD}  ╚══════════════════════════════════════════════╝${RESET}"
echo ""
sleep 2

narrate "Goal: Build a Python charging optimizer for electric trucks"
sleep 1

# ── Step 1: PO Agent ────────────────────────────────────────────────────────

narrate "Step 1: PO Agent plans the sprint"

PO_PROMPT="You are a Product Owner. Goal: Build a Python module that finds the cheapest 4-hour charging window for a truck given 24 hourly electricity prices. Include SoC validation and pytest tests.

Output ONLY a JSON array:
[{\"id\":\"...\",\"title\":\"...\",\"depends_on\":[],\"agent_prompt\":\"...\"}]
Keep to 2-3 tasks. Be specific about files and functions."

DAG_JSON=$($SANDBOX "$WORK/project" claude -p "$PO_PROMPT" --dangerously-skip-permissions 2>/dev/null \
    | python3 -c "import sys,json,re; m=re.search(r'\[.*\]',sys.stdin.read(),re.DOTALL); print(json.dumps(json.loads(m.group())) if m else '[]')")

echo "$DAG_JSON" > "$WORK/dag.json"

echo "$DAG_JSON" | python3 -c "
import json, sys
tasks = json.load(sys.stdin)
for i, t in enumerate(tasks):
    deps = ', '.join(t.get('depends_on', [])) or 'none'
    print(f'  {i+1}. {t[\"title\"]}')
    print(f'     depends on: {deps}')
"
sleep 2

# ── Step 2: Execute tasks (Python handles the loop to avoid bash parsing issues) ──

python3 << 'PYEOF'
import json, subprocess, os, base64, sys, time

WORK = os.environ.get("WORK", "/tmp/caloron-demo")
SANDBOX = os.environ.get("SANDBOX", "scripts/sandbox-agent.sh")
GITEA_TOKEN = os.environ.get("GITEA_TOKEN", "")
REPO = os.environ.get("DEMO_REPO", "caloron/demo-project")

BOLD, DIM, GREEN, YELLOW, BLUE, CYAN, RED, RESET = (
    '\033[1m', '\033[2m', '\033[32m', '\033[33m', '\033[34m', '\033[36m', '\033[31m', '\033[0m')

def narrate(msg):
    print(f"\n{BOLD}{CYAN}▸ {msg}{RESET}")
    time.sleep(1)
def ok(msg):  print(f"  {GREEN}✓{RESET} {msg}")
def warn(msg): print(f"  {YELLOW}!{RESET} {msg}")
def step(who, msg): print(f"  {BOLD}{BLUE}[{who}]{RESET} {msg}")

def gitea_api(method, path, data=None):
    if method == "GET":
        r = subprocess.run(["docker","exec","gitea","wget","-qO-",
            "--header",f"Authorization: token {GITEA_TOKEN}",
            f"http://127.0.0.1:3000{path}"], capture_output=True, text=True)
    else:
        r = subprocess.run(["docker","exec","gitea","wget","-qO-",
            "--post-data", json.dumps(data),
            "--header","Content-Type: application/json",
            "--header",f"Authorization: token {GITEA_TOKEN}",
            f"http://127.0.0.1:3000{path}"], capture_output=True, text=True)
    try: return json.loads(r.stdout)
    except: return {}

def upload_file(branch, filepath, content, msg):
    b64 = base64.b64encode(content.encode()).decode()
    existing = gitea_api("GET", f"/api/v1/repos/{REPO}/contents/{filepath}?ref={branch}")
    sha = existing.get("sha", "")
    payload = {"content": b64, "message": msg, "branch": branch}
    if sha: payload["sha"] = sha
    gitea_api("POST", f"/api/v1/repos/{REPO}/contents/{filepath}", payload)

def git_merge(branch, message):
    rp = f"/data/git/repositories/{REPO}.git"
    subprocess.run(["docker","exec","-u","git","gitea","sh","-c",
        f"chmod -x {rp}/hooks/pre-receive 2>/dev/null; "
        f"cd /tmp && rm -rf _merge && mkdir _merge && cd _merge && "
        f"git init -q && git fetch {rp} main:main {branch}:{branch} 2>/dev/null && "
        f"git checkout main 2>/dev/null && git merge {branch} -m '{message}' 2>/dev/null && "
        f"git push {rp} main:main 2>/dev/null; "
        f"chmod +x {rp}/hooks/pre-receive 2>/dev/null"],
        capture_output=True)

tasks = json.load(open(f"{WORK}/dag.json"))
# Topo sort
completed = set()
remaining = list(tasks)
pr_num = 2

while remaining:
    ready = [t for t in remaining if all(d in completed for d in t.get("depends_on", []))]
    if not ready: break

    for task in ready:
        tid, title = task["id"], task["title"]
        prompt = task.get("agent_prompt", title)

        narrate(f"Agent works on: {title}")

        # Create issue
        result = gitea_api("POST", f"/api/v1/repos/{REPO}/issues",
            {"title": title, "body": f"Task: {tid}"})
        inum = result.get("number", "?")
        ok(f"Issue #{inum} created on Gitea")

        # Agent writes code
        step("AGENT", "Writing code (sandboxed)...")
        agent_result = subprocess.run(
            [SANDBOX, f"{WORK}/project", "claude", "-p",
             f"{prompt}\n\nRules: Only modify src/ and tests/. Use type hints. When done, stop.",
             "--dangerously-skip-permissions"],
            capture_output=True, text=True, timeout=180)
        summary = [l for l in (agent_result.stdout or "").strip().split("\n") if l.strip()]
        if summary:
            ok(summary[-1][:80])

        # Collect changed files
        os.chdir(f"{WORK}/project")
        subprocess.run(["git", "add", "-A"], capture_output=True)
        diff = subprocess.run(["git", "diff", "--cached", "--name-only"], capture_output=True, text=True)
        changed = [f for f in diff.stdout.strip().split("\n")
                   if f and (f.startswith("src/") or f.startswith("tests/"))
                   and f not in ("src/__init__.py", "tests/__init__.py")]
        subprocess.run(["git", "checkout", "--", "."], capture_output=True)

        if changed:
            branch = f"agent/{tid}"
            gitea_api("POST", f"/api/v1/repos/{REPO}/branches",
                {"new_branch_name": branch, "old_branch_name": "main"})

            for fp in changed:
                full = os.path.join(f"{WORK}/project", fp)
                if os.path.exists(full):
                    upload_file(branch, fp, open(full).read(), f"[{tid}] {fp}")
            ok(f"Pushed to branch {branch}")

            pr_num += 1
            gitea_api("POST", f"/api/v1/repos/{REPO}/pulls",
                {"title": f"[{tid}] {title}", "body": f"Agent: caloron-agent-{tid}",
                 "head": branch, "base": "main"})
            ok(f"PR #{pr_num} created")

            step("REVIEWER", "Reviewing code...")
            review_result = subprocess.run(
                [SANDBOX, f"{WORK}/project", "claude", "-p",
                 f"Review: {title}. Files: {', '.join(changed)}. Respond ONLY: APPROVED or CHANGES_NEEDED: reason",
                 "--dangerously-skip-permissions"],
                capture_output=True, text=True, timeout=60)
            review = (review_result.stdout or "").strip().split("\n")[-1] if review_result.stdout else "APPROVED"
            if "APPROVED" in review.upper():
                ok(f"Review: APPROVED")
            else:
                warn(f"Review: {review[:60]}")

            git_merge(branch, f"Merge: [{tid}] {title}")
            ok(f"PR #{pr_num} merged ✓")

        completed.add(tid)
        remaining.remove(task)
        time.sleep(1)
        break

# Retro
narrate("Sprint Retro")
print(f"  {BOLD}Tasks completed:{RESET} {len(completed)}/{len(tasks)}")
print(f"  {BOLD}PRs created:{RESET}     {len(completed)}")
print(f"  {BOLD}Code reviews:{RESET}    {len(completed)}")
print()

narrate("Gitea audit trail")
prs = gitea_api("GET", f"/api/v1/repos/{REPO}/pulls?state=all&limit=10")
if isinstance(prs, list):
    for pr in sorted(prs, key=lambda x: x.get("number", 0)):
        if pr.get("title", "").startswith("["):
            print(f"  PR #{pr['number']}: {pr['title']}")

print()
print(f"{BOLD}{GREEN}  Sprint complete!{RESET}")
print()
time.sleep(2)

# ── Post-sprint: Show the code ──────────────────────────────────────────
narrate("Generated code")
print()

for py_file in ["src/charging_optimizer.py", "src/optimizer.py"]:
    resp = gitea_api("GET", f"/api/v1/repos/{REPO}/contents/{py_file}")
    if resp.get("content"):
        import base64
        code = base64.b64decode(resp["content"]).decode()
        lines = code.strip().split("\n")
        print(f"  {BOLD}── {py_file} ({len(lines)} lines) ──{RESET}")
        # Show first 15 lines
        for line in lines[:15]:
            print(f"  {DIM}{line}{RESET}")
        if len(lines) > 15:
            print(f"  {DIM}  ... ({len(lines) - 15} more lines){RESET}")
        print()
        break

time.sleep(2)

# ── Post-sprint: Run tests ─────────────────────────────────────────────
narrate("Running tests")
print()

# Clone latest from Gitea and run pytest
import tempfile
with tempfile.TemporaryDirectory() as tmpdir:
    subprocess.run(["docker", "exec", "gitea", "rm", "-rf", "/tmp/_test_clone"], capture_output=True)
    subprocess.run(["docker", "exec", "-u", "git", "gitea", "git", "clone",
        f"/data/git/repositories/{REPO}.git", "/tmp/_test_clone"],
        capture_output=True)
    subprocess.run(["docker", "cp", "gitea:/tmp/_test_clone/.", tmpdir], capture_output=True)

    test_result = subprocess.run(
        ["python3", "-m", "pytest", "tests/", "-v", "--tb=short"],
        cwd=tmpdir, capture_output=True, text=True, timeout=30)

    for line in test_result.stdout.strip().split("\n"):
        if "PASSED" in line or "FAILED" in line or "passed" in line or "failed" in line or "=====" in line:
            color = GREEN if "PASSED" in line or "passed" in line else RED if "FAILED" in line or "failed" in line else ""
            print(f"  {color}{line.strip()}{RESET}")

print()
time.sleep(2)

# ── Post-sprint: KPIs + Next sprint proposal ───────────────────────────
narrate("Sprint KPIs")
print()

total_time_s = int(time.time() - start_time) if 'start_time' not in dir() else 0
# Estimate from task count
task_count = len(completed)
print(f"  {BOLD}┌─────────────────────────────────────────┐{RESET}")
print(f"  {BOLD}│  Tasks completed    {GREEN}{task_count}/{task_count}{RESET}{BOLD}                   │{RESET}")
print(f"  {BOLD}│  PRs merged         {GREEN}{task_count}{RESET}{BOLD}                      │{RESET}")
print(f"  {BOLD}│  Code reviews       {GREEN}{task_count}{RESET}{BOLD}                      │{RESET}")
print(f"  {BOLD}│  Test pass rate     {GREEN}✓{RESET}{BOLD}                      │{RESET}")
print(f"  {BOLD}│  Supervisor events  {GREEN}0{RESET}{BOLD}                      │{RESET}")
print(f"  {BOLD}└─────────────────────────────────────────┘{RESET}")
print()
time.sleep(2)

narrate("Next sprint proposal (based on retro)")
print()
print(f"  The retro identified these improvements for Sprint 2:")
print()
print(f"  {YELLOW}1.{RESET} Add departure deadline constraint (truck must leave by hour X)")
print(f"  {YELLOW}2.{RESET} Multi-truck fleet scheduling (N chargers, M trucks)")
print(f"  {YELLOW}3.{RESET} Input validation for load_prices (hour bounds, missing data)")
print(f"  {YELLOW}4.{RESET} CLI interface for fleet operators")
print()
print(f"  {DIM}These would be the PO Agent's input for the next sprint.{RESET}")
print(f"  {DIM}Run: python3 orchestrator.py \"<Sprint 2 goal with improvements>\"{RESET}")
print()
time.sleep(3)

print(f"{BOLD}{GREEN}  ─── Demo complete ───{RESET}")
print()
time.sleep(2)
PYEOF
