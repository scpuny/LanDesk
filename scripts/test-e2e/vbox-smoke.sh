#!/usr/bin/env bash
# ============================================================================
# LanDesk 双机 E2E 冒烟编排（VBoxManage，pre-release 门禁）
#
# 目标：发版前在真实双机环境验证「发现 → 连接 → 画面 → 键鼠」，替代"发版靠装机踩坑"。
# 原则：只做 VBoxManage 命令 + 宿主侧可观察的探测；**不进 guest 内部**（不依赖
#       Guest Additions / 共享目录 / guest 内账号密码），guest 内的 GUI 操作以
#       明确的【人工步骤 + 判据】列出，由执行者在 VM 窗口完成并打勾。
#
# 为什么不全自动：控制端 CLI（apps/controller）是早期骨架，设备发现/连接/画面
#       编排只在 Tauri GUI 里，没有可编程驱动入口。全自动需新增 headless 测试命令，
#       属独立改动。本脚本先把"可自动的部分"自动化 + 把"必须人工的部分"标准化，
#       不造假绿灯。
#
# 用法：
#   ./scripts/test-e2e/vbox-smoke.sh                 # 默认两台 VM 都起、跑宿主侧探测
#   ./scripts/test-e2e/vbox-smoke.sh --no-start      # VM 已在跑，只做探测+出清单
#   ./scripts/test-e2e/vbox-smoke.sh --shutdown      # 结束后关停（savestate）VM
#   VM_CONTROLLED="windows 10" VM_OBSERVER="Xubuntu2204" ./scripts/test-e2e/vbox-smoke.sh
#
# 产物：scripts/test-e2e/reports/smoke-YYYYmmdd-HHMMSS.md（宿主侧证据 + 人工打勾清单）
# ============================================================================
set -uo pipefail

VBOX="${VBOX:-VBoxManage}"
VM_CONTROLLED="${VM_CONTROLLED:-windows 10}"   # 被控端 VM（跑 LanDesk 受控服务）
VM_OBSERVER="${VM_OBSERVER:-Xubuntu2204}"      # 控制端 VM（观察画面/操作键鼠）
MAC_CONTROLLED="${MAC_CONTROLLED:-08002776F719}"
MAC_OBSERVER="${MAC_OBSERVER:-080027AC70C1}"
PORT="${PORT:-52000}"
WAIT_NET="${WAIT_NET:-90}"                      # 起 VM 后等网络就绪秒数
START_VM=1
SHUTDOWN=0

for a in "$@"; do
  case "$a" in
    --no-start) START_VM=0 ;;
    --shutdown) SHUTDOWN=1 ;;
    -h|--help)  sed -n '2,28p' "$0"; exit 0 ;;
    *) echo "未知参数: $a" >&2; exit 2 ;;
  esac
done

REPORT_DIR="$(cd "$(dirname "$0")" && pwd)/reports"
mkdir -p "$REPORT_DIR"
REPORT="$REPORT_DIR/smoke-$(date +%Y%m%d-%H%M%S).md"

pass() { echo "  [PASS] $*"; echo "- [PASS] $*" >> "$REPORT"; }
fail() { echo "  [FAIL] $*"; echo "- [FAIL] $*" >> "$REPORT"; }
info() { echo "  [INFO] $*"; echo "- INFO: $*" >> "$REPORT"; }
section() { echo; echo "== $* =="; { echo; echo "## $*"; } >> "$REPORT"; }

{
  echo "# LanDesk 双机 E2E 冒烟报告"
  echo
  echo "- 时间: $(date '+%Y-%m-%d %H:%M:%S %z')"
  echo "- 被控端 VM: $VM_CONTROLLED ($MAC_CONTROLLED)"
  echo "- 控制端 VM: $VM_OBSERVER ($MAC_OBSERVER)"
  echo "- 宿主: $(hostname)"
  echo
  echo "> 说明：本报告的 [PASS]/[FAIL] 为宿主侧自动探测结果；【人工】项为 VM 窗口内操作，"
  echo "> 需执行者按判据核对后手动打勾。两类缺一不可，未勾满不得 promote 正式版。"
} > "$REPORT"

echo "报告: $REPORT"

command -v "$VBOX" >/dev/null 2>&1 || { echo "缺少 $VBOX" >&2; exit 1; }

section "前置检查"
for vm in "$VM_CONTROLLED" "$VM_OBSERVER"; do
  if "$VBOX" showvminfo "$vm" >/dev/null 2>&1; then pass "VM 存在: $vm"; else fail "VM 不存在: $vm"; exit 1; fi
done

section "启动 VM"
if [[ $START_VM -eq 1 ]]; then
  for vm in "$VM_CONTROLLED" "$VM_OBSERVER"; do
    state="$("$VBOX" showvminfo "$vm" --machinereadable 2>/dev/null | sed -n 's/^VMState="\(.*\)"/\1/p')"
    case "$state" in
      running) pass "已运行: $vm" ;;
      saved|poweroff) "$VBOX" startvm "$vm" --type headless >/dev/null 2>&1 && pass "已启动: $vm ($state→running)" || fail "启动失败: $vm" ;;
      *) info "VM $vm 状态=$state，跳过启动" ;;
    esac
  done
  echo "  等待网络就绪（${WAIT_NET}s）…"
  sleep "$WAIT_NET"
else
  info "--no-start：跳过启动与等待"
fi

section "宿主侧网络探活（按 MAC 查 arp）"
resolve_ip() {
  local mac="$1" up mac_colon ip
  up="$(printf '%s' "$mac" | sed 's/../&:/g; s/:$//' | tr 'a-f' 'A-F')"
  mac_colon="$(printf '%s' "$mac" | sed 's/../&:/g; s/:$//')"
  ip="$(arp -an 2>/dev/null | grep -i "$mac_colon" | sed -n 's/.*(\([0-9.]*\)).*/\1/p' | head -1)"
  [[ -z "$ip" ]] && ip="$(arp -an 2>/dev/null | grep -i "$up" | sed -n 's/.*(\([0-9.]*\)).*/\1/p' | head -1)"
  printf '%s' "$ip"
}
ip_c="$(resolve_ip "$MAC_CONTROLLED")"
ip_o="$(resolve_ip "$MAC_OBSERVER")"
[[ -n "$ip_c" ]] && pass "被控端 VM IP: $ip_c" || fail "被控端 VM 未取得 IP（VM 未起/网卡非桥接/未通信）"
[[ -n "$ip_o" ]] && pass "控制端 VM IP: $ip_o" || fail "控制端 VM 未取得 IP"

section "受控端口探测（UDP $PORT，仅提示）"
if [[ -n "$ip_c" ]]; then
  # UDP 无连接，nc -z 不可靠；用宿主 nmap 探测（若装），否则提示人工 ss 核查。
  if command -v nmap >/dev/null 2>&1; then
    nmap -sU -p "$PORT" -T4 --open "$ip_c" 2>/dev/null | grep -q "open" \
      && pass "被控端 UDP $PORT 开放" \
      || info "被控端 UDP $PORT 未探到开放（受控服务未在 guest 内启动，或防火墙拦截——见人工项）"
  else
    info "宿主无 nmap，跳过 UDP 探测；在 VM 内执行 ss -lunp | grep $PORT 人工确认"
  fi
fi

section "【人工】被控端 VM 内准备（$VM_CONTROLLED）"
cat >> "$REPORT" <<EOF
- [ ] 受控端 LanDesk 已安装并启动，受控服务显示"监听 UDP $PORT"
- [ ] 已设置访问密码（或临时码通道开），设备码已记录: ______
- [ ] 屏幕处于已登录交互桌面（非锁屏）——若测 Windows 黑屏修复项
- [ ] Windows 防火墙 / macOS 应用防火墙已放行 LanDesk 入站（若测防火墙项）
- [ ] 日志级别设为 info
EOF

section "【人工】控制端 VM 内验证（$VM_OBSERVER）"
cat >> "$REPORT" <<EOF
- [ ] 设备列表能发现被控端（名称/IP 正确）—— A1
- [ ] 输入访问密码后连接成功，状态栏 conn_mode=内网直连 —— A2
- [ ] 画面为被控端**真实桌面**（非红块测试图、非纯黑）—— 问题2/5 回归
- [ ] 若对端无屏幕录制权限/锁屏：显示采集降级 banner 而非误认真桌面 —— B3
- [ ] 鼠标移动、点击、键盘打字在被控端实时响应 —— A3
- [ ] 关闭会话窗后重新连接，直接进入密码页（非"选择设备"）—— 问题3 回归
- [ ] 会话窗顶栏 macOS 红绿灯不挤出右侧按钮 —— 问题1 回归
- [ ] 连接失败时文案含 macOS 防火墙排障提示（若测防火墙项）—— 问题4 回归
- [ ] 到日志中心导出诊断包，文件非空且含环境快照 + 日志 —— 本期诊断能力
EOF

section "证据与结论"
cat >> "$REPORT" <<EOF
- 诊断包附件路径: ______
- 截图/录屏路径: ______
- 未通过项（必填原因）: ______
- 结论: [ ] 全部通过，可 promote 正式版  [ ] 有未决项，不得发布
EOF

if [[ $SHUTDOWN -eq 1 ]]; then
  section "关停 VM"
  for vm in "$VM_CONTROLLED" "$VM_OBSERVER"; do
    "$VBOX" controlvm "$vm" savestate >/dev/null 2>&1 && pass "已 savestate: $vm" || info "关停跳过/失败: $vm"
  done
fi

echo
echo "完成。请打开报告逐项打勾：$REPORT"
echo "只有【人工】项全部勾选且无未决项，才允许 pre-release promote 正式版。"
