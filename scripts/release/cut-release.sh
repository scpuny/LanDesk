#!/usr/bin/env bash
# ============================================================================
# LanDesk 发布切分支（标准 Git Flow + release branch）
#
# 流程（main 只接受 release，dev 不直接进 main）：
#   1. 本脚本：从最新 dev 切出/重置 pre-release 分支并推送 → 触发 CI
#   2. 等 CI 在 pre-release 上跑绿（GitHub Actions /actions/workflows/ci.yml）
#   3. 确认 CI 绿后，仍由本脚本从 pre-release 打 v* tag 并推送 → 触发 Release
#   4. Release 成功后，release.yml 的 finalize-release job 自动：
#        合 tag 进 main + 删除 pre-release 分支（全程无需手动碰 main）
#
# 用法：
#   ./scripts/release/cut-release.sh            # 只做第 1 步（推 pre-release 等 CI）
#   ./scripts/release/cut-release.sh v0.2.5     # 第 1 步 + 校验 CI 后打 tag v0.2.5
#   ./scripts/release/cut-release.sh --tag v0.2.5  # 跳过重推，仅从现有 pre-release 打 tag
#
# 安全约束：
#   - pre-release 用 `git branch -f` 强制指向最新 dev（它是临时发布分支，允许重置）；
#   - 打 tag 前会校验 pre-release 的本地 commit 与 origin 一致，并提示人工确认 CI 已绿；
#   - 不主动 merge/push main——那一步由 CI 在 Release 成功后完成。
# ============================================================================
set -euo pipefail

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
cd "$REPO_ROOT"

RED='\033[0;31m'; GREEN='\033[0;32m'; YELLOW='\033[1;33m'; NC='\033[0m'
info() { echo -e "${GREEN}[cut-release]${NC} $*"; }
warn() { echo -e "${YELLOW}[cut-release]${NC} $*"; }
die()  { echo -e "${RED}[cut-release]${NC} $*" >&2; exit 1; }

GH_REPO="${LAN_DESK_REPO:-scpuny/LanDesk}"
MODE="prepare"; TAG=""

while [[ $# -gt 0 ]]; do
  case "$1" in
    --tag|-t) MODE="tag"; shift; TAG="${1:-}"; shift ;;
    -h|--help) sed -n '2,28p' "${BASH_SOURCE[0]}"; exit 0 ;;
    v*) TAG="$1"; shift ;;
    *) die "未知参数: $1" ;;
  esac
done

git fetch origin --prune || die "git fetch origin 失败"

case "$MODE" in
  prepare)
    # 第 1 步：从最新 dev 切出/重置 pre-release
    info "从 origin/dev 重置 pre-release 分支..."
    git checkout -q dev 2>/dev/null || git checkout -q -b dev origin/dev
    git reset --hard -q origin/dev
    git branch -f pre-release origin/dev
    git checkout -q pre-release
    info "pre-release 已指向 $(git rev-parse --short HEAD) ($(git log -1 --format=%s))"
    git push -f origin pre-release
    info "已推送 origin/pre-release，CI 应已触发。"
    echo
    warn "下一步：等 CI 在 pre-release 上跑绿："
    warn "  https://github.com/${GH_REPO}/actions/workflows/ci.yml?query=branch%3Apre-release"
    warn "CI 绿后执行： $0 --tag vX.Y.Z"
    ;;
  tag)
    [[ -n "$TAG" ]] || die "打 tag 模式需要版本号，如 --tag v0.2.5"
    [[ "$TAG" =~ ^v[0-9]+\.[0-9]+\.[0-9]+$ ]] || die "版本号格式应为 vX.Y.Z，收到: $TAG"

    info "打 tag 前校验..."
    git checkout -q pre-release 2>/dev/null || die "本地无 pre-release 分支，请先运行 $0"
    git fetch origin pre-release
    LOCAL="$(git rev-parse HEAD)"
    REMOTE="$(git rev-parse origin/pre-release)"
    [[ "$LOCAL" == "$REMOTE" ]] || die "本地 pre-release($LOCAL) 与 origin($REMOTE) 不一致，请先同步"

    git tag -l "$TAG" | grep -q . && die "tag $TAG 已存在"
    warn "★ 请再次确认 CI 已在 pre-release($LOCAL) 上跑绿！！"
    read -r -p "确认 CI 已通过、现在打 tag $TAG？[y/N] " ans
    [[ "$ans" =~ ^[Yy]$ ]] || die "已取消"

    git tag -a "$TAG" -m "release: $TAG（pre-release $(git rev-parse --short HEAD)）"
    git push origin "$TAG"
    info "已推送 tag $TAG，Release 应已触发。"
    warn "Release 成功后，finalize-release 会自动合入 main 并删除 pre-release。"
    warn "  https://github.com/${GH_REPO}/actions/workflows/release.yml"
    ;;
esac
