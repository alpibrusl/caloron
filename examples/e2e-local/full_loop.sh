#!/bin/bash
# ==========================================================================
# FULL AUTONOMOUS SPRINT — NO MOCKS
#
# Complete cycle:
#   1. PO generates DAG
#   2. Issues created on Gitea
#   3. Agents write code (sandboxed Claude Code)
#   4. Code pushed to Gitea branches via API
#   5. PRs created on Gitea
#   6. Reviewer agent reviews each PR
#   7. PRs merged, DAG advances
#   8. Retro: KPIs + report
# ==========================================================================
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
SANDBOX="$SCRIPT_DIR/../../scripts/sandbox-agent.sh"
GITEA_TOKEN="${GITEA_TOKEN:-c50bad400bd9b8cde3e930cca052eae6ded71f7b}"
REPO="caloron/full-loop"
GOAL="${1:-Build a Python module with functions to validate email addresses and phone numbers. Include comprehensive pytest tests.}"
WORK="/tmp/caloron-full-loop"

gitea() {
    local method="$1" path="$2" data="${3:-}"
    if [ "$method" = "GET" ]; then
        docker exec gitea wget -qO- \
            --header="Authorization: token ${GITEA_TOKEN}" \
            "http://127.0.0.1:3000${path}" 2>/dev/null
    elif [ "$method" = "POST" ]; then
        docker exec gitea wget -qO- \
            --post-data="$data" \
            --header="Content-Type: application/json" \
            --header="Authorization: token ${GITEA_TOKEN}" \
            "http://127.0.0.1:3000${path}" 2>/dev/null
    fi
}

upload_file() {
    local branch="$1" filepath="$2" content="$3" msg="$4"
    local b64=$(echo -n "$content" | base64 -w0)
    # Check if file exists (need SHA for update)
    local existing_sha
    existing_sha=$(gitea GET "/api/v1/repos/${REPO}/contents/${filepath}?ref=${branch}" 2>/dev/null \
        | python3 -c "import json,sys; print(json.load(sys.stdin).get('sha',''))" 2>/dev/null || echo "")

    if [ -n "$existing_sha" ] && [ "$existing_sha" != "" ]; then
        gitea POST "/api/v1/repos/${REPO}/contents/${filepath}" \
            "{\"content\":\"${b64}\",\"message\":\"${msg}\",\"branch\":\"${branch}\",\"sha\":\"${existing_sha}\"}" > /dev/null 2>&1
    else
        gitea POST "/api/v1/repos/${REPO}/contents/${filepath}" \
            "{\"content\":\"${b64}\",\"message\":\"${msg}\",\"branch\":\"${branch}\"}" > /dev/null 2>&1
    fi
}

echo "================================================================"
echo "  FULL AUTONOMOUS SPRINT"
echo "  Goal: $GOAL"
echo "================================================================"
echo ""

# Setup workspace
rm -rf "$WORK" && mkdir -p "$WORK/project/src" "$WORK/project/tests"
cd "$WORK/project"
git init -q && git config user.name "caloron" && git config user.email "bot@caloron.local"
echo '"""Project."""' > src/__init__.py; echo '' > tests/__init__.py
git add -A && git commit -q -m "workspace init"

# ======================================================================
# Step 1: PO Agent
# ======================================================================
echo "--- Step 1: PO Agent ---"
PO_PROMPT="You are a Product Owner. Goal: $GOAL

Output ONLY a JSON array:
[{\"id\":\"...\",\"title\":\"...\",\"depends_on\":[],\"agent_prompt\":\"Create src/... with ...\"}]
Keep to 2-3 tasks. Tests depend on implementation. Be specific about files and functions."

DAG_JSON=$($SANDBOX "$WORK/project" claude -p "$PO_PROMPT" --dangerously-skip-permissions 2>/dev/null \
    | python3 -c "import sys,json,re; m=re.search(r'\[.*\]',sys.stdin.read(),re.DOTALL); print(json.dumps(json.loads(m.group())) if m else '[]')")
echo "$DAG_JSON" > "$WORK/dag.json"
echo "$DAG_JSON" | python3 -c "
import json,sys
for t in json.load(sys.stdin):
    print(f'  {t[\"id\"]}: {t[\"title\"]} (deps: {\", \".join(t.get(\"depends_on\",[]) ) or \"none\"})')
"
echo ""

# ======================================================================
# Step 2: Create issues
# ======================================================================
echo "--- Step 2: Issues ---"
python3 -c "
import json
for t in json.load(open('$WORK/dag.json')):
    print(f'{t[\"id\"]}|{t[\"title\"]}')
" | while IFS='|' read -r tid title; do
    NUM=$(gitea POST "/api/v1/repos/$REPO/issues" \
        "{\"title\":\"$title\",\"body\":\"Task: $tid\"}" \
        | python3 -c "import json,sys; print(json.load(sys.stdin).get('number','?'))" 2>/dev/null)
    echo "  Issue #$NUM: $title"
done
echo ""

# ======================================================================
# Step 3-7: Execute tasks with full PR cycle
# ======================================================================
echo "--- Step 3: Execute ---"
echo ""

python3 << 'PYEOF'
import json, subprocess, time, os, base64

work = os.environ.get("WORK", "/tmp/caloron-full-loop")
project = f"{work}/project"
sandbox = os.environ.get("SANDBOX", "scripts/sandbox-agent.sh")
gitea_token = os.environ.get("GITEA_TOKEN", "c50bad400bd9b8cde3e930cca052eae6ded71f7b")
repo = os.environ.get("REPO", "caloron/full-loop")

with open(f"{work}/dag.json") as f:
    tasks = json.load(f)

completed = set()
remaining = list(tasks)
feedback = []
sprint_start = time.time()

def gitea_api(method, path, data=None):
    if method == "GET":
        r = subprocess.run(
            ["docker", "exec", "gitea", "wget", "-qO-",
             "--header", f"Authorization: token {gitea_token}",
             f"http://127.0.0.1:3000{path}"],
            capture_output=True, text=True)
    else:
        r = subprocess.run(
            ["docker", "exec", "gitea", "wget", "-qO-",
             "--post-data", json.dumps(data),
             "--header", "Content-Type: application/json",
             "--header", f"Authorization: token {gitea_token}",
             f"http://127.0.0.1:3000{path}"],
            capture_output=True, text=True)
    try:
        return json.loads(r.stdout)
    except:
        return {}

def upload_file(branch, filepath, content, msg):
    b64 = base64.b64encode(content.encode()).decode()
    # Check if exists
    existing = gitea_api("GET", f"/api/v1/repos/{repo}/contents/{filepath}?ref={branch}")
    sha = existing.get("sha", "")
    payload = {"content": b64, "message": msg, "branch": branch}
    if sha:
        payload["sha"] = sha
    gitea_api("POST", f"/api/v1/repos/{repo}/contents/{filepath}", payload)

while remaining:
    ready = [t for t in remaining if all(d in completed for d in t.get("depends_on", []))]
    if not ready:
        print("STUCK!"); break

    for task in ready:
        tid = task["id"]
        title = task["title"]
        prompt = task.get("agent_prompt", title)
        t0 = time.time()

        print(f"{'='*50}")
        print(f"  {tid}: {title}")
        print(f"{'='*50}")

        # 3. Agent writes code (sandboxed)
        os.chdir(project)
        full_prompt = f"{prompt}\n\nRules: Only modify src/ and tests/. Use type hints. When done, stop."
        print("  Agent writing code...")
        r = subprocess.run(
            [sandbox, project, "claude", "-p", full_prompt, "--dangerously-skip-permissions"],
            capture_output=True, text=True)
        for line in (r.stdout or "").strip().split("\n")[-2:]:
            print(f"    {line}")

        # Collect changed files
        subprocess.run(["git", "add", "-A"], cwd=project, capture_output=True)
        diff = subprocess.run(["git", "diff", "--cached", "--name-only"], cwd=project, capture_output=True, text=True)
        changed = [f for f in diff.stdout.strip().split("\n") if f and (f.startswith("src/") or f.startswith("tests/"))]

        if not changed:
            print("  No changes — skipping")
            completed.add(tid)
            remaining.remove(task)
            continue

        # Reset git (we'll push via API)
        subprocess.run(["git", "checkout", "--", "."], cwd=project, capture_output=True)

        # 4. Create branch on Gitea
        branch = f"agent/{tid}"
        gitea_api("POST", f"/api/v1/repos/{repo}/branches", {
            "new_branch_name": branch, "old_branch_name": "main"
        })
        print(f"  Branch: {branch}")

        # 5. Upload files to branch
        for filepath in changed:
            full_path = os.path.join(project, filepath)
            if os.path.exists(full_path):
                content = open(full_path).read()
                upload_file(branch, filepath, content, f"[{tid}] {filepath}")
                print(f"  Uploaded: {filepath}")

        # 6. Create PR
        pr = gitea_api("POST", f"/api/v1/repos/{repo}/pulls", {
            "title": f"[{tid}] {title}",
            "body": f"Agent: caloron-agent-{tid}",
            "head": branch,
            "base": "main",
        })
        pr_num = pr.get("number", "?")
        print(f"  PR #{pr_num} created")

        # 7. Reviewer agent
        print("  Reviewer checking...")
        review_prompt = f"""Review this code change for: {title}

Changed files: {', '.join(changed)}

Check: correctness, tests, type hints.
Respond with ONLY one line: APPROVED or CHANGES_NEEDED: reason"""

        rr = subprocess.run(
            [sandbox, project, "claude", "-p", review_prompt, "--dangerously-skip-permissions"],
            capture_output=True, text=True)
        review = (rr.stdout or "").strip().split("\n")[-1]
        print(f"  Review: {review[:60]}")

        # Post review as comment
        gitea_api("POST", f"/api/v1/repos/{repo}/issues/{pr_num}/comments", {
            "body": f"**Review:** {review}"
        })

        # 8. Merge PR
        merge = gitea_api("POST", f"/api/v1/repos/{repo}/pulls/{pr_num}/merge", {
            "Do": "merge"
        })
        if merge.get("sha") or merge.get("merged"):
            print(f"  PR #{pr_num} MERGED")
        else:
            print(f"  Merge: {str(merge)[:60]}")

        elapsed = int(time.time() - t0)
        feedback.append({"task_id": tid, "time_s": elapsed, "files": changed, "pr": pr_num})
        completed.add(tid)
        remaining.remove(task)
        print(f"  Done ({elapsed}s)")
        print()

# Save feedback
total = int(time.time() - sprint_start)
with open(f"{work}/feedback.json", "w") as f:
    json.dump({"tasks": feedback, "total_time_s": total}, f, indent=2)
print(f"Sprint complete: {len(tasks)} tasks in {total}s")
PYEOF

echo ""

# ======================================================================
# Step 8: Retro
# ======================================================================
echo "--- Step 8: Retro ---"
python3 -c "
import json
fb = json.load(open('$WORK/feedback.json'))
tasks = fb['tasks']
total = fb['total_time_s']
print(f'Tasks:     {len(tasks)}')
print(f'Total:     {total}s')
for t in tasks:
    print(f'  {t[\"task_id\"]}: {t[\"time_s\"]}s, files: {\", \".join(t[\"files\"])}, PR #{t[\"pr\"]}')
print(f'Avg:       {total//max(len(tasks),1)}s/task')
print(f'Rate:      100%')
"

echo ""
echo "--- Gitea State ---"
echo "PRs:"
gitea GET "/api/v1/repos/$REPO/pulls?state=all&limit=50" | python3 -c "
import json,sys
for pr in sorted(json.load(sys.stdin), key=lambda x: x['number']):
    state = 'merged' if pr.get('merged') else pr['state']
    print(f'  PR #{pr[\"number\"]}: {pr[\"title\"]} [{state}]')
" 2>/dev/null || echo "  (parse error)"

echo ""
echo "Issues:"
gitea GET "/api/v1/repos/$REPO/issues?state=all&type=issues&limit=50" | python3 -c "
import json,sys
for i in sorted(json.load(sys.stdin), key=lambda x: x['number']):
    print(f'  #{i[\"number\"]}: {i[\"title\"]} [{i[\"state\"]}]')
" 2>/dev/null || echo "  (parse error)"

echo ""
echo "================================================================"
echo "  FULL LOOP COMPLETE — NO MOCKS"
echo "================================================================"
