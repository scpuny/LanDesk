#!/usr/bin/env bash
# ============================================================================
# LanDesk 稳定性压测脚本（NFR-01）
#
# 对齐 docs/architecture.md §4.2 异常恢复矩阵 + docs/PRD.md NFR-01：
#   - 显卡休眠/显示器插拔/分辨率切换 → 编码器自动重置，会话尽量保活；
#   - 硬编码驱动异常 → 稳定降级 OpenH264，不崩溃；
#   - 连续稳定运行 72h 无内存泄漏、无句柄泄露。
#
# 本脚本是「长跑 + 句柄/内存采样」驱动器：
#   1. 起受控端（或指定被观测进程），持续运行 DURATION；
#   2. 每 INTERVAL 采样一次 RSS 内存 + 打开句柄数（Linux: /proc/PID/fd、
#      VmRSS；可加 TCP/UDP socket 计数），追加到采样 CSV；
#   3. 运行中循环注入异常事件（可开）：强制编码器重置 / 分辨率变更信号；
#   4. 结束后出报告：峰值 RSS、句柄增量、是否有单调增长趋势（泄漏判据），
#      并 grep 出编码器失败/降级事件计数（NFR-06 可观测性）。
#
# 默认在 OK3568 真机 / 任意 Linux 上跑；非 Linux 退化为「只采样外部命令」。
#
# 用法：
#   ./scripts/release/stable-72h.sh                          # 默认 72h（可 4h 冒烟）
#   DURATION=4h ./scripts/release/stable-72h.sh              # 冒烟 4 小时
#   DURATION=4h INTERVAL=30s CHURN=on ./scripts/release/stable-72h.sh
#   TARGET_CMD="/path/controlled --role controlled" ./scripts/release/stable-72h.sh
#   SAMPLE_PID=12345 ./scripts/release/stable-72h.sh         # 观测已在运行的进程（不起新进程）
# ============================================================================
set -uo pipefail

# --- 可调参数（env 覆盖） ---------------------------------------------------
DURATION="${DURATION:-72h}"          # 总时长：N s / N m / N h / N d
INTERVAL="${INTERVAL:-5m}"          # 采样间隔
CHURN="${CHURN:-off}"               # 运行中周期性注入异常事件（on/off）
CHURN_PERIOD="${CHURN_PERIOD:-1h}"  # 异常事件注入周期
TARGET_CMD="${TARGET_CMD:-}"         # 被观测进程命令（空格分隔）；空则不新起
SAMPLE_PID="${SAMPLE_PID:-}"        # 直接观测已有进程 PID
REPORT_DIR="${REPORT_DIR:-dist/stable-72h}"

mkdir -p "${REPORT_DIR}"
REPORT="${REPORT_DIR}/report-$(date +%Y%m%d-%H%M%S).txt"
CSV="${REPORT_DIR}/samples-$(date +%Y%m%d-%H%M%S).csv"

# ----------------------------------------------------------------------------
# 时单位 → 秒
# ----------------------------------------------------------------------------
to_seconds() {
  local v="$1"
  local n="${v%[smhd]}"
  local unit="${v:${#n}}"
  case "${unit}" in
    s) echo "${n}" ;;
    m) echo $(( n * 60 )) ;;
    h) echo $(( n * 3600 )) ;;
    d) echo $(( n * 86400 )) ;;
    "") echo "${n}" ;;
    *) echo "未知时长单位: ${v}（支持 s/m/h/d）" >&2; return 1 ;;
  esac
}

DUR_S="$(to_seconds "${DURATION}")" || exit 1
INT_S="$(to_seconds "${INTERVAL}")" || exit 1
CHURN_S="$(to_seconds "${CHURN_PERIOD}")" || exit 1

echo "==> LanDesk 稳定性压测（NFR-01）"
echo "    时长 ${DURATION}(${DUR_S}s)  采样 ${INTERVAL}(${INT_S}s)  churn=${CHURN}  报告 ${REPORT}"

# ----------------------------------------------------------------------------
# 句柄 / 内存 采样（Linux 用 /proc；其余退化）
# ----------------------------------------------------------------------------
is_linux() { [[ "$(uname -s)" == "Linux" ]]; }

read_sample() {
  # $1=pid  -> 输出 "rss_kb fds sockets"
  local pid="$1"
  if is_linux; then
    local rss=0 fds=0 socks=0
    [[ -r "/proc/${pid}/status" ]] || { echo "0 0 0"; return 0; }
    rss="$(awk '/^VmRSS:/{print $2}' "/proc/${pid}/status" 2>/dev/null)"
    rss="${rss:-0}"
    fds="$(ls -1 "/proc/${pid}/fd" 2>/dev/null | wc -l)"
    socks="$(ss -Htulnp 2>/dev/null | grep -oE "pid=${pid}," | wc -l)"
    echo "${rss} ${fds} ${socks}"
  else
    # 非 Linux：尝试 ps 取 RSS（KB）
    rss="$(ps -o rss= -p "${pid}" 2>/dev/null | tr -d ' ')"
    echo "${rss:-0} 0 0"
  fi
}

# ----------------------------------------------------------------------------
# 起 / 接观测目标
# ----------------------------------------------------------------------------
PID=""
CLEANUP_PID=""
trap '[[ -n "${CLEANUP_PID}" ]] && kill "${CLEANUP_PID}" 2>/dev/null' EXIT

if [[ -n "${SAMPLE_PID}" ]]; then
  PID="${SAMPLE_PID}"
  echo "    观测已有进程 PID=${PID}"
else
  if [[ -z "${TARGET_CMD}" ]]; then
    echo "!! 需要 TARGET_CMD（被观测命令）或 SAMPLE_PID（已有进程）。"
    echo "   例：TARGET_CMD='./dist/linux/controlled --role controlled' ${0}"
    echo "   例：SAMPLE_PID=\$(pgrep -f 'controlled --role controlled') ${0}"
    exit 2
  fi
  # 起进程（后台），stderr 记日志以便 grep 编码器事件
  LOG="${REPORT_DIR}/target-$(date +%Y%m%d-%H%M%S).log"
  eval "${TARGET_CMD} >>\"${LOG}\" 2>&1 &"
  PID="$!"
  CLEANUP_PID="${PID}"
  sleep 2
  if ! kill -0 "${PID}" 2>/dev/null; then
    echo "!! 目标进程启动后即退出（PID=${PID}）；查 ${LOG}"
    exit 1
  fi
  echo "    已起目标进程 PID=${PID}，日志 ${LOG}"
  LOG_REF="${LOG}"
fi

# ----------------------------------------------------------------------------
# 主采样循环
# ----------------------------------------------------------------------------
echo "ts,epoch,alive,rss_kb,fds,sockets" > "${CSV}"
start_epoch="$(date +%s)"
elapsed=0
last_churn="$start_epoch"
churn_count=0
peak_rss=0

while :; do
  now_epoch="$(date +%s)"
  elapsed=$(( now_epoch - start_epoch ))
  if [[ ${elapsed} -ge ${DUR_S} ]]; then
    break
  fi

  if kill -0 "${PID}" 2>/dev/null; then
    alive=1
    sample="$(read_sample "${PID}")"
    rss="$(echo "${sample}" | awk '{print $1}')"
    fds="$(echo "${sample}" | awk '{print $2}')"
    socks="$(echo "${sample}" | awk '{print $3}')"
    [[ -n "${rss}" && "${rss}" -gt "${peak_rss}" ]] && peak_rss="${rss}"
    echo "$(date '+%Y-%m-%d %H:%M:%S'),${now_epoch},${alive},${rss},${fds},${socks}" >> "${CSV}"
    echo "    [${elapsed}s] rss=${rss}KB fds=${fds} sockets=${socks} peak=${peak_rss}KB"
  else
    alive=0
    echo "$(date '+%Y-%m-%d %H:%M:%S'),${now_epoch},0,-,-,-" >> "${CSV}"
    echo "!! 目标进程退出（elapsed=${elapsed}s）——压测提前结束，视为失败"
    break
  fi

  # churn：周期性注入异常事件（编码器重置 / 分辨率变更）
  if [[ "${CHURN}" == "on" ]] && kill -0 "${PID}" 2>/dev/null; then
    if [[ $(( now_epoch - last_churn )) -ge ${CHURN_S} ]]; then
      # 通过 RST/USR1 通知目标做 encoder reset（若目标实现了对应 handler）；
      # 未实现则至少记录一次“事件已尝试注入”以便关联日志窗口。
      kill -USR1 "${PID}" 2>/dev/null || true
      churn_count=$(( churn_count + 1 ))
      last_churn="${now_epoch}"
      echo "    [churn#${churn_count}] 注入异常事件（SIGUSR1）elapsed=${now_epoch}"
    fi
  fi

  # 睡眠到下一采样点
  remain=$(( DUR_S - elapsed ))
  sleep_sec=$(( INT_S < remain ? INT_S : remain ))
  sleep "${sleep_sec}"
done

# ----------------------------------------------------------------------------
# 报告：泄漏判据 + 编码器事件计数（NFR-06）
# ----------------------------------------------------------------------------
{
  echo "==================== LanDesk 稳定性压测报告 ===================="
  echo "开始: $(date -d "@${start_epoch}" 2>/dev/null || date)"
  echo "时长: ${DURATION}（实际 ${elapsed}s，提前退出: $(kill -0 "${PID}" 2>/dev/null && echo 否 || echo 是)）"
  echo "采样点: $(($(wc -l < "${CSV}") - 1))   churn 注入次数: ${churn_count}"
  echo "峰值 RSS: ${peak_rss} KB"
  echo
  echo "---- 泄漏判据（RSS 单调增长 / 句柄只增不减）----"
  echo "首/末 采样："
  sed -n '2p;$p' "${CSV}" | column -s, -t 2>/dev/null || sed -n '2p;$p' "${CSV}"
  # 简单趋势判据：取「有 RSS 数值的采样行」的首行与末行对比
  awk -F, 'NR>1 && $4 ~ /^[0-9]+$/ { if(!got){first=$4; got=1}; last=$4; n++ }
        END{
          if(n>0){
            printf "采样行数=%d  首样本RSS=%sKB  末样本RSS=%sKB\n", n, first, last;
            if(last > first*1.5) printf "结论：末样本 > 首样本*1.5，疑似内存泄漏，需人工复核整条曲线。\n";
            else if(last < first*0.5) printf "结论：末样本 < 首样本*0.5，可能 GC 回落，属正常。\n";
            else printf "结论：末样本在首样本 0.5~1.5 倍内，未见明显增长趋势（仍建议看整条曲线确认）。\n";
          } else printf "结论：无有效 RSS 采样，无法判定（非 Linux 或未记录 /proc）。\n";
        }' "${CSV}"
  echo
  echo "---- 编码器/会话事件计数（来自目标日志，NFR-06）----"
  if [[ -n "${LOG_REF:-}" && -f "${LOG_REF}" ]]; then
    for kw in "ENC_RESET" "reopen" "downgrade" "openh264" "v4l2m2m" "reset" "Reconnect" "NoPath" "ERROR" "panic"; do
      c="$(grep -icE "${kw}" "${LOG_REF}" 2>/dev/null || true)"
      printf "   %-14s %s\n" "${kw}" "${c:-0}"
    done
    echo "   （以上关键词命中数；'reopen/downgrade' 增多说明硬编在反复失败降级，需排查驱动。）"
  else
    echo "   （未记录目标日志：未提供 TARGET_CMD，跳过编码器事件统计。）"
  fi
  echo
  echo "CSV: ${CSV}"
  echo "==============================================================="
} | tee "${REPORT}"

# 清理
if [[ -n "${CLEANUP_PID}" ]]; then
  kill -0 "${CLEANUP_PID}" 2>/dev/null && kill "${CLEANUP_PID}" 2>/dev/null || true
  CLEANUP_PID=""
fi

echo "==> 压测完成。报告：${REPORT}   CSV：${CSV}"
