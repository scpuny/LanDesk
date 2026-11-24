#!/usr/bin/env bash
# 一键触发测试包构建并等待结果，输出 artifact 下载指引。
#
# 原理：GitHub 的 workflow_dispatch 只能触发默认分支(main)上的工作流；
# 本工作流 test-build.yml 在 feature 分支，故用 push 到 test-build/** 专用分支触发
# （push 事件读取被推送分支自身的工作流定义，不受默认分支限制）。
# 该分支只是触发器，代码来源就是推送上去的当前 HEAD，完全不碰 dev/main/tag/Release。
#
# 用法：
#   ./scripts/trigger-test-build.sh              # 把当前 HEAD 推到 test-build/latest 触发
#   ./scripts/trigger-test-build.sh my-branch    # 推到 test-build/my-branch 触发
#   ./scripts/trigger-test-build.sh --wait        # 触发后轮询等到三个 job 结束（最多 40 分钟）

set -euo pipefail

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$REPO_ROOT"

SLUG=""
WAIT=0
for arg in "$@"; do
  case "$arg" in
    --wait) WAIT=1 ;;
    -h|--help)
      sed -n '2,14p' "${BASH_SOURCE[0]}" | sed 's/^# \{0,1\}//'
      exit 0 ;;
    *) SLUG="$arg" ;;
  esac
done
SLUG="${SLUG:-latest}"
TRIGGER_BRANCH="test-build/${SLUG}"

# 安全护栏：只允许推 test-build/** 分支
case "$TRIGGER_BRANCH" in
  test-build/*) ;;
  *) echo "拒绝：目标分支必须是 test-build/**" >&2; exit 1 ;;
esac

CURRENT="$(git branch --show-current)"
HEAD_SHORT="$(git rev-parse --short HEAD)"
echo "== 触发测试包构建 =="
echo "   来源分支: ${CURRENT} @ ${HEAD_SHORT}"
echo "   触发分支: ${TRIGGER_BRANCH}（仅作触发，不污染 dev/main）"

# 取远端 slug（owner/repo），用于拼 API/UI 链接
REMOTE_URL="$(git remote get-url origin)"
if [[ "$REMOTE_URL" == git@* ]]; then
  SLUG_REMOTE="${REMOTE_URL#git@*:}"
else
  SLUG_REMOTE="${REMOTE_URL#*github.com/}"
fi
SLUG_REMOTE="${SLUG_REMOTE%.git}"
echo "   仓库: ${SLUG_REMOTE}"

# push 当前 HEAD 到触发分支（force：该分支是纯触发器，每次覆盖即可）
GIT_TERMINAL_PROMPT=0 git push -u -f origin "HEAD:refs/heads/${TRIGGER_BRANCH}"

echo ""
echo "已触发。查看构建进度："
echo "  https://github.com/${SLUG_REMOTE}/actions/workflows/test-build.yml?query=branch%3A${TRIGGER_BRANCH}"
echo ""
echo "完成后在该 run 页面底部 Artifacts 下载 test-macos-<N> / test-windows-<N> / test-linux-<N>。"

if [[ "$WAIT" -eq 1 ]]; then
  command -v curl >/dev/null || { echo "需 curl 才能轮询" >&2; exit 1; }
  TOKEN="$(security find-internet-password -s github.com -w 2>/dev/null || true)"
  AUTH=()
  [[ -n "${TOKEN:-}" ]] && AUTH=(-H "Authorization: Bearer ${TOKEN}")
  echo ""
  echo "== 等待构建（最多 40 分钟，每 30 秒查一次）=="
  DEADLINE=$(( $(date +%s) + 2400 ))
  RUN_ID=""
  while [[ $(date +%s) -lt $DEADLINE ]]; do
    RUNS_JSON="$(curl -sS -m 20 "${AUTH[@]}" -H 'Accept: application/vnd.github+json' \
      "https://api.github.com/repos/${SLUG_REMOTE}/actions/workflows/test-build.yml/runs?branch=${TRIGGER_BRANCH}&per_page=1")"
    RUN_ID="$(printf '%s' "$RUNS_JSON" | python3 -c '
import sys, json
try:
    runs = json.load(sys.stdin).get("workflow_runs", [])
    print(runs[0]["id"] if runs else "")
except Exception:
    print("")
')"
    [[ -n "$RUN_ID" ]] && break
    sleep 30
  done
  [[ -n "$RUN_ID" ]] || { echo "未找到 run，请手动打开上面链接" >&2; exit 1; }
  echo "run id=${RUN_ID}"

  while [[ $(date +%s) -lt $DEADLINE ]]; do
    RUN_JSON="$(curl -sS -m 20 "${AUTH[@]}" -H 'Accept: application/vnd.github+json' \
      "https://api.github.com/repos/${SLUG_REMOTE}/actions/runs/${RUN_ID}")"
    STATE="$(printf '%s' "$RUN_JSON" | python3 -c 'import sys,json; print(json.load(sys.stdin).get("status",""))' 2>/dev/null || true)"
    CONCLUSION="$(printf '%s' "$RUN_JSON" | python3 -c 'import sys,json; print(json.load(sys.stdin).get("conclusion") or "-")' 2>/dev/null || true)"
    echo "  status=${STATE} conclusion=${CONCLUSION}"
    [[ "$STATE" == "completed" ]] && break
    sleep 30
  done
  echo ""
  echo "== 完成 =="
  echo "  https://github.com/${SLUG_REMOTE}/actions/runs/${RUN_ID}"
  echo "注意：GUI 打包是 best-effort，需进各 job 日志确认 GUI artifacts 行非空，再下载。"
fi
