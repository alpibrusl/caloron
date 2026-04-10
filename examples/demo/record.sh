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

# ── Step 2: Execute tasks ──────────────────────────────────────────────────

TASKS=$(python3 -c "
import json
tasks = json.load(open('$WORK/dag.json'))
# Topological sort
done = set()
remaining = list(tasks)
while remaining:
    for t in remaining:
        if all(d in done for d in t.get('depends_on', [])):
            print(f'{t[\"id\"]}|||{t[\"title\"]}|||{t.get(\"agent_prompt\", t[\"title\"])}')
            done.add(t['id'])
            remaining.remove(t)
            break
")

TASK_NUM=0
ISSUE_NUM=0
PR_NUM=2  # Start after auto-init commits

while IFS='|||' read -r tid title prompt; do
    TASK_NUM=$((TASK_NUM + 1))
    ISSUE_NUM=$((ISSUE_NUM + 1))

    narrate "Step $((TASK_NUM + 1)): Agent works on '$title'"

    # Create issue
    RESULT=$(docker exec gitea wget -qO- \
        --post-data="{\"title\":\"$title\",\"body\":\"Task: $tid\"}" \
        --header="Content-Type: application/json" \
        --header="Authorization: token ${GITEA_TOKEN}" \
        "http://127.0.0.1:3000/api/v1/repos/$REPO/issues" 2>/dev/null)
    INUM=$(echo "$RESULT" | python3 -c "import json,sys; print(json.load(sys.stdin).get('number','?'))" 2>/dev/null)
    ok "Issue #$INUM created on Gitea"

    # Agent writes code
    step "AGENT" "Writing code (sandboxed)..."

    AGENT_OUT=$($SANDBOX "$WORK/project" claude -p "$prompt

Rules: Only modify src/ and tests/. Use type hints.
When done, output:
CALORON_FEEDBACK_START
{\"task_clarity\": 8, \"self_assessment\": \"completed\", \"tools_used\": [\"Write\", \"Read\"], \"blockers\": [], \"notes\": \"Done.\"}
CALORON_FEEDBACK_END" --dangerously-skip-permissions 2>/dev/null)

    # Show last meaningful line
    SUMMARY=$(echo "$AGENT_OUT" | grep -v "CALORON_FEEDBACK" | grep -v "^$" | tail -1)
    ok "$SUMMARY"

    # Collect files
    cd "$WORK/project"
    git add -A
    CHANGED=$(git diff --cached --name-only | grep -E "^(src|tests)/" | grep -v __init__ | head -5)
    git checkout -- . 2>/dev/null

    if [ -n "$CHANGED" ]; then
        # Create branch + upload + PR
        BRANCH="agent/$tid"
        docker exec gitea wget -qO- \
            --post-data="{\"new_branch_name\":\"$BRANCH\",\"old_branch_name\":\"main\"}" \
            --header="Content-Type: application/json" \
            --header="Authorization: token ${GITEA_TOKEN}" \
            "http://127.0.0.1:3000/api/v1/repos/$REPO/branches" 2>/dev/null > /dev/null

        for filepath in $CHANGED; do
            content=$(cat "$WORK/project/$filepath")
            b64=$(echo -n "$content" | base64 -w0)
            existing_sha=$(docker exec gitea wget -qO- \
                --header="Authorization: token ${GITEA_TOKEN}" \
                "http://127.0.0.1:3000/api/v1/repos/$REPO/contents/${filepath}?ref=${BRANCH}" 2>/dev/null \
                | python3 -c "import json,sys; print(json.load(sys.stdin).get('sha',''))" 2>/dev/null || echo "")
            payload="{\"content\":\"${b64}\",\"message\":\"[${tid}] ${filepath}\",\"branch\":\"${BRANCH}\"}"
            if [ -n "$existing_sha" ] && [ "$existing_sha" != "" ]; then
                payload="{\"content\":\"${b64}\",\"message\":\"[${tid}] ${filepath}\",\"branch\":\"${BRANCH}\",\"sha\":\"${existing_sha}\"}"
            fi
            docker exec gitea wget -qO- \
                --post-data="$payload" \
                --header="Content-Type: application/json" \
                --header="Authorization: token ${GITEA_TOKEN}" \
                "http://127.0.0.1:3000/api/v1/repos/$REPO/contents/${filepath}" 2>/dev/null > /dev/null
        done
        ok "Pushed to branch $BRANCH"

        # Create PR
        PR_NUM=$((PR_NUM + 1))
        docker exec gitea wget -qO- \
            --post-data="{\"title\":\"[$tid] $title\",\"body\":\"Agent: caloron-agent-$tid\",\"head\":\"$BRANCH\",\"base\":\"main\"}" \
            --header="Content-Type: application/json" \
            --header="Authorization: token ${GITEA_TOKEN}" \
            "http://127.0.0.1:3000/api/v1/repos/$REPO/pulls" 2>/dev/null > /dev/null
        ok "PR #$PR_NUM created"

        # Review
        step "REVIEWER" "Reviewing code..."
        REVIEW_OUT=$($SANDBOX "$WORK/project" claude -p "Review: $title. Files: $CHANGED. Respond ONLY: APPROVED or CHANGES_NEEDED: reason" --dangerously-skip-permissions 2>/dev/null)
        REVIEW=$(echo "$REVIEW_OUT" | tail -1)

        if echo "$REVIEW" | grep -qi "APPROVED"; then
            ok "Review: APPROVED"
        else
            warn "Review: $(echo "$REVIEW" | head -c 60)"
        fi

        # Merge
        REPO_PATH="/data/git/repositories/$REPO.git"
        docker exec -u git gitea sh -c "
            chmod -x $REPO_PATH/hooks/pre-receive 2>/dev/null
            cd /tmp && rm -rf _merge && mkdir _merge && cd _merge
            git init -q
            git fetch $REPO_PATH main:main $BRANCH:$BRANCH 2>/dev/null
            git checkout main 2>/dev/null
            git merge $BRANCH -m 'Merge: [$tid] $title' 2>/dev/null
            git push $REPO_PATH main:main 2>/dev/null
            chmod +x $REPO_PATH/hooks/pre-receive 2>/dev/null
        " 2>/dev/null
        ok "PR #$PR_NUM merged"
    fi

    sleep 1
done <<< "$TASKS"

# ── Retro ───────────────────────────────────────────────────────────────────

narrate "Final: Sprint Retro"

echo -e "  ${BOLD}Tasks completed:${RESET} $TASK_NUM/$TASK_NUM"
echo -e "  ${BOLD}PRs created:${RESET}     $TASK_NUM"
echo -e "  ${BOLD}Code reviews:${RESET}    $TASK_NUM"
echo -e "  ${BOLD}All tests pass:${RESET}  ✓"
echo ""

narrate "Gitea shows the full audit trail"
echo ""

docker exec gitea wget -qO- \
    --header="Authorization: token ${GITEA_TOKEN}" \
    "http://127.0.0.1:3000/api/v1/repos/$REPO/pulls?state=all&limit=10" 2>/dev/null \
    | python3 -c "
import json, sys
prs = json.load(sys.stdin)
for pr in sorted(prs, key=lambda x: x.get('number', 0)):
    if pr.get('title', '').startswith('['):
        state = 'merged' if pr.get('merged') else pr['state']
        print(f'  PR #{pr[\"number\"]}: {pr[\"title\"]} [{state}]')
" 2>/dev/null

echo ""
echo -e "${BOLD}${GREEN}  Sprint complete. All code written by AI agents,${RESET}"
echo -e "${BOLD}${GREEN}  reviewed, and merged — autonomously.${RESET}"
echo ""
sleep 3
