#!/usr/bin/env bash
# ============================================================================
# LanDesk 双机 E2E 冒烟编排（VBoxManage + controller headless，pre-release 门禁）
#
# 目标：发版前在真实双机环境验证「发现 → 连接 → 画面 → 键鼠」，替代"发版靠装机踩坑"。
# 分层：
#   [自动] VBoxManage 起停 VM + arp 探活 + controller device-list/probe（画面判据）；
#   [人工] VM 窗口内目视/键鼠操作与窗口布局，按清单打勾。
# 原则：不进 guest 内部（不依赖 Guest Additions / 共享目录 / guest 账号），
#       没有凭据就跳过 probe 并明确标注，**不造假绿灯**。
#
# 用法：
#   ./scripts/test-e2e/vbox-smoke.sh                          # 起 VM + 宿主探测 + 人工清单
#   ./scripts/test-e2e/vbox-smoke.sh --no-start               # VM 已在跑，只探测
#   ./scripts/test-e2e/vbox-smoke.sh --shutdown               # 结束后 savestate VM
#   # 全自动画面探针（凭据优先走环境变量，避免 ps 泄露密码）：
#   CONTROLLER_BIN=target/debug/controller \
#   LAN_DESK_DEVICE="windows 10" LAN_DESK_PASSWORD='xxx' \
#     ./scripts/test-e2e/vbox-smoke.sh --no-start
#   VM_CONTROLLED="windows 10" VM_OBSERVER="Xubuntu2204" ./scripts/test-e2e/vbox-smoke.sh
#
# 依赖：VBoxManage、node（仓库前端已有，用于解析 JSON）；nmap 可选。
# 产物：scripts/test-e2e/reports/smoke-YYYYmmdd-HHMMSS.md
# 退出码：0=自动项全过；1=有自动项 FAIL（不得 promote 正式版）。
# ============================================================================
set -uo pipefail

ROOT="$(cd "$(dirname "$0")/../.." && pwd)"
VBOX="${VBOX:-VBoxManage}"
VM_CONTROLLED="${VM_CONTROLLED:-windows 10}"   # 被控端 VM（跑 LanDesk 受控服务）
VM_OBSERVER="${VM_OBSERVER:-Xubuntu2204}"      # 控制端 VM（观察画面/操作键鼠）
MAC_CONTROLLED="${MAC_CONTROLLED:-08002776F719}"
MAC_OBSERVER="${MAC_OBSERVER:-080027AC70C1}"
PORT="${PORT:-52000}"
WAIT_NET="${WAIT_NET:-90}"                      # 起 VM 后等网络就绪秒数
DISCOVER_SECS="${DISCOVER_SECS:-3}"
PROBE_SECS="${PROBE_SECS:-6}"
CONTROLLER_BIN="${CONTROLLER_BIN:-$ROOT/target/debug/controller}"
LAN_DESK_DEVICE="${LAN_DESK_DEVICE:-}"
LAN_DESK_PASSWORD="${LAN_DESK_PASSWORD:-}"
START_VM=1
SHUTDOWN=0
FAILED=0

while [[ $# -gt 0 ]]; do
  case "$1" in
    --no-start)    START_VM=0; shift ;;
    --shutdown)    SHUTDOWN=1; shift ;;
    --controller)  CONTROLLER_BIN="$2"; shift 2 ;;
    --device)      LAN_DESK_DEVICE="$2"; shift 2 ;;
    -h|--help)     sed -n '2,33p' "$0"; exit 0 ;;
    *) echo "未知参数: $1" >&2; exit 2 ;;
  esac
done

REPORT_DIR="$(dirname "$0")/reports"
mkdir -p "$REPORT_DIR"
REPORT="$REPORT_DIR/smoke-$(date +%Y%m%d-%H%M%S).md"

pass() { echo "  [PASS] $*"; echo "- [PASS] $*" >> "$REPORT"; }
fail() { echo "  [FAIL] $*"; echo "- [FAIL] $*" >> "$REPORT"; FAILED=$((FAILED + 1)); }
info() { echo "  [INFO] $*"; echo "- INFO: $*" >> "$REPORT"; }
section() { echo; echo "== $* =="; { echo; echo "## $*"; } >> "$REPORT"; }

# 从 stdin 的 JSON 里取一个顶层字段；解析失败输出空。
json_field() {
  node -e '
    const key = process.argv[1];
    let s = "";
    process.stdin.on("data", d => s += d).on("end", () => {
      try { const v = JSON.parse(s)[key]; process.stdout.write(v === undefined ? "" : String(v)); }
      catch (_) { process.stdout.write("__PARSE_ERROR__"); }
    });
  ' "$1"
}

{
  echo "# LanDesk 双机 E2E 冒烟报告"
  echo
  echo "- 时间: $(date '+%Y-%m-%d %H:%M:%S %z')"
  echo "- 被控端 VM: $VM_CONTROLLED ($MAC_CONTROLLED)"
  echo "- 控制端 VM: $VM_OBSERVER ($MAC_OBSERVER)"
  echo "- 宿主: $(hostname)"
  echo "- controller: $CONTROLLER_BIN"
  echo "- 自动探针目标: ${LAN_DESK_DEVICE:-未指定}"
  echo
  echo "> 说明：[PASS]/[FAIL] 为自动探测结果；【人工】项需在 VM 窗口按判据目视核对。"
  echo "> 自动项 PASS 不替代人工目视（布局/真实桌面观感仍需人看）。任一项未过不得 promote 正式版。"
  echo "> 密码不写入报告。"
} > "$REPORT"

echo "报告: $REPORT"

command -v "$VBOX" >/dev/null 2>&1 || { echo "缺少 $VBOX" >&2; exit 1; }
command -v node >/dev/null 2>&1 || { echo "缺少 node（用于解析 controller JSON 输出）" >&2; exit 1; }

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
  if command -v nmap >/dev/null 2>&1; then
    nmap -sU -p "$PORT" -T4 --open "$ip_c" 2>/dev/null | grep -q "open" \
      && pass "被控端 UDP $PORT 开放" \
      || info "被控端 UDP $PORT 未探到开放（受控服务未启动或防火墙拦截——见人工项）"
  else
    info "宿主无 nmap，跳过 UDP 探测；在 VM 内执行 ss -lunp | grep $PORT 人工确认"
  fi
fi

section "自动 headless 探测（controller device-list / probe）"
if [[ -x "$CONTROLLER_BIN" ]]; then
  pass "controller 二进制可用: $CONTROLLER_BIN"

  dl_out="$("$CONTROLLER_BIN" device-list --wait-secs "$DISCOVER_SECS" 2>/dev/null)"
  dev_count="$(printf '%s' "$dl_out" | json_field length)"
  if [[ "$dev_count" == "__PARSE_ERROR__" ]]; then
    fail "device-list 输出非 JSON（输出: ${dl_out:0:120}）"
  elif [[ "$dev_count" =~ ^[0-9]+$ ]] && (( dev_count > 0 )); then
    pass "mDNS 自动发现 $dev_count 台设备（FR-01）"
    echo "\`\`\`json" >> "$REPORT"; printf '%s\n' "$dl_out" >> "$REPORT"; echo "\`\`\`" >> "$REPORT"
  else
    fail "mDNS 未发现任何设备（受控端未广播 / 防火墙拦 mDNS / 非同网段）"
  fi

  if [[ -n "$LAN_DESK_DEVICE" && -n "$LAN_DESK_PASSWORD" ]]; then
    probe_out="$("$CONTROLLER_BIN" probe \
      --device "$LAN_DESK_DEVICE" --password "$LAN_DESK_PASSWORD" \
      --discover-secs "$DISCOVER_SECS" --duration-secs "$PROBE_SECS" 2>/dev/null)"
    probe_rc=$?
    verdict="$(printf '%s' "$probe_out" | json_field verdict)"
    reason="$(printf '%s' "$probe_out" | json_field reason)"
    echo "\`\`\`json" >> "$REPORT"; printf '%s\n' "$probe_out" >> "$REPORT"; echo "\`\`\`" >> "$REPORT"
    if [[ $probe_rc -eq 0 && "$verdict" == "PASS" ]]; then
      pass "画面探针 PASS（设备 $LAN_DESK_DEVICE，reason=$reason）—— 连接+关键帧+非降级"
    else
      fail "画面探针 FAIL（reason=$reason，exit=$probe_rc）—— 见上表 JSON 证据"
      case "$reason" in
        device_not_found) info "排查：设备码/名称是否与 device-list 一致，mDNS 是否可见" ;;
        connect_failed)   info "排查：访问密码是否正确；对端防火墙是否拦 UDP $PORT 入站（问题4）" ;;
        no_video_frames)  info "排查：受控端是否起视频泵；是否无屏幕录制权限（问题2）" ;;
        capture_no_permission) info "问题2/5 回归失败：对端无屏幕录制权限，banner 应显示而非测试图" ;;
        capture_blank_frame)   info "问题5 回归失败：对端锁屏/非交互桌面，banner 应显示而非纯黑" ;;
        no_keyframe)      info "排查：编码器是否产出 IDR/SPS-PPS，前端解码器无法出首帧" ;;
      esac
    fi
  else
    info "未同时提供 LAN_DESK_DEVICE 与 LAN_DESK_PASSWORD，跳过 probe（不造假绿灯）"
    info "启用方式：LAN_DESK_DEVICE='<设备码或名称>' LAN_DESK_PASSWORD='<密码>' $0 --no-start"
  fi
else
  fail "controller 二进制不存在: $CONTROLLER_BIN（构建：cargo build -p controller）"
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
- [ ] 设备列表能发现被控端（名称/IP 正确）—— A1（自动 device-list 亦需 PASS）
- [ ] 输入访问密码后连接成功，状态栏 conn_mode=内网直连 —— A2（自动 probe 亦需 PASS）
- [ ] 画面为被控端**真实桌面**（非红块测试图、非纯黑）—— 问题2/5 回归（目视）
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
echo "完成。自动项 FAIL=$FAILED；请打开报告逐项打勾：$REPORT"
if [[ $FAILED -gt 0 ]]; then
  echo "自动项未全过，禁止 promote 正式版。"
  exit 1
fi
echo "自动项全过；仍需【人工】项全部勾选且无未决项，才允许 pre-release promote 正式版。"
