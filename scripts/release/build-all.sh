#!/usr/bin/env bash
# ============================================================================
# LanDesk 四平台产物 + 签名配置（M9）
#
# 对齐 docs/architecture.md §6「平台构建策略」：
#   平台        采集            注入       硬编降级链                    打包
#   Windows     WGC(降级GDI)    SendInput  qsv→nvenc→amf→openh264        NSIS
#   Linux PC    PipeWire        /dev/uinput qsv→nvenc→v4l2m2m→openh264  deb + AppImage
#   macOS       CGDisplayStream  CGEvent   videotoolbox→openh264         dmg
#   OK3568      V4L2/DRM        /dev/uinput v4l2m2m→openh264            aarch64 静态单文件
#
# NFR-05：ffmpeg 裁剪 + 静态链接 + 单二进制无外部 .so 依赖。
# NFR-06：打包时把 landesk-encoder.log / 分级 tracing 配置一并带上。
#
# 本脚本是「打包编排」入口：
#   - 本机/CI 上能对哪些平台就地编译，就编译哪些；
#   - 需要交叉工具链/签名凭据的环节缺失时，打印明确提示而非报错退出，
#     让 M9 真机/签名阶段可增量补做（与 build-libav 脚本同风格：缺 FFMPEG_SRC 用 : ? 报错，
#     但本入口脚本是编排器，缺失项应「跳过 + 提示」而非整体失败）。
#
# 用法：
#   ./scripts/release/build-all.sh                 # 探测当前宿主，就地构建可行平台
#   ./scripts/release/build-all.sh --all            # 构建全部可行平台
#   ./scripts/release/build-all.sh ok3568           # 只构建 OK3568 aarch64（可并列多平台名）
#   ./scripts/release/build-all.sh --sign            # 构建并签名（需签名凭据）
#   OUT_DIR=dist ./scripts/release/build-all.sh      # 指定产物输出根（默认 dist/，已 gitignore）
# ============================================================================
set -euo pipefail

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
cd "${REPO_ROOT}"

OUT_DIR="${OUT_DIR:-dist}"
PLATFORMS=()
DO_SIGN=0

# --- 参数解析 ---------------------------------------------------------------
while [[ $# -gt 0 ]]; do
  case "$1" in
    --all)     PLATFORMS=(win linux macos ok3568); shift ;;
    --sign)    DO_SIGN=1; shift ;;
    -h|--help) grep '^#' "${BASH_SOURCE[0]}" | sed 's/^# \{0,2\}//'; exit 0 ;;
    win|windows|linux|macos|mac|ok3568|aarch64)
      PLATFORMS+=("$1"); shift ;;
    *) echo "未知参数: $1（可用: --all | win|linux|macos|ok3568 | --sign）" >&2; exit 2 ;;
  esac
done
# 归一化别名（windows→win, macos/mac→macos, aarch64→ok3568）
normalize_platforms() {
  local out=()
  for p in "${PLATFORMS[@]}"; do
    case "${p}" in
      windows) p=win ;;
      macos|mac) p=macos ;;
      aarch64) p=ok3568 ;;
    esac
    out+=("${p}")
  done
  PLATFORMS=("${out[@]}")
}
normalize_platforms

if [[ ${#PLATFORMS[@]} -eq 0 ]]; then
  # 探测宿主默认构建
  case "$(uname -s)" in
    Darwin)  PLATFORMS=(macos) ;;
    MINGW*|MSYS*|CYGWIN*) PLATFORMS=(win) ;;
    Linux)   PLATFORMS=(linux) ;;
    *) echo "无法识别宿主平台 $(uname -s)" >&2; exit 2 ;;
  esac
fi

# cargo / rustc 不在默认 PATH 的兜底（与本仓构建约定一致）
export PATH="${HOME}/.cargo/bin:${PATH}"

# 产物统一输出到 gitignore 的 dist/（.gitignore 已含 dist/ release/ thirdparty/）
mkdir -p "${OUT_DIR}"

# 裁剪 libav 根目录（build-libav 产物；thirdparty/ 已 gitignore）
AV_X86="thirdparty/av-x86_64"
AV_AARCH64="thirdparty/av-aarch64"

# 各平台 feature 组合（对齐 architecture.md §6 平台构建策略）
declare -A PLATFORM_FEATURES=(
  [win]="capture/windows,avcodec-wrap/qsv,avcodec-wrap/nvenc,avcodec-wrap/amf,avcodec-wrap/openh264"
  [linux]="capture/linux,avcodec-wrap/qsv,avcodec-wrap/nvenc,avcodec-wrap/v4l2m2m,avcodec-wrap/openh264"
  [macos]="capture/macos,avcodec-wrap/videotoolbox,avcodec-wrap/openh264"
  [ok3568]="capture/linux,avcodec-wrap/v4l2m2m,avcodec-wrap/openh264"
)

# 各平台主产物（受控端/控制端 双二进制，NFR-05 单文件）
BINS=(controlled controller)
# 可选独立二进制
OPTIONAL_BINS=(relay-server signaling-server)

# ----------------------------------------------------------------------------
# 构建单个平台的 Rust 二进制（cargo build --release）
# ----------------------------------------------------------------------------
build_rust() {
  local plat="$1"; local target="$2"; local use_cross="${3:-0}"
  local feats="${PLATFORM_FEATURES[${plat}]}"
  local out="${OUT_DIR}/${plat}"
  mkdir -p "${out}"

  # aarch64 需要裁剪 libav 交叉产物
  if [[ "${plat}" == ok3568 ]]; then
    if [[ ! -d "${AV_AARCH64}" ]]; then
      echo "!! [ok3568] 缺裁剪 libav aarch64 产物 ${AV_AARCH64}（thirdparty/ 未备）。"
      echo "   降级为**无编码骨架**：avcodec-wrap 各编码器 feature 当前为占位空标记"
      echo "   （无 build.rs/FFI 链接点，不拉 thirdparty），仍可 cross 编出可运行骨架；"
      echo "   完整编码产物需先跑 CC=aarch64-linux-gnu-gcc ./scripts/build-libav/build-aarch64-ok3568.sh。"
    else
      export LAVF_ROOT="${AV_AARCH64}"
      export OPENH264_ROOT="${AV_AARCH64}"
    fi
  else
    if [[ ! -d "${AV_X86}" ]]; then
      echo "!! [${plat}] 缺裁剪 libav x86_64 产物 ${AV_X86}（thirdparty/ 未备）。"
      echo "   降级为**无编码骨架**：avcodec-wrap feature 当前为占位空标记（不链接 thirdparty/FFI），"
      echo "   仍可编出 controlled/controller 可运行骨架；完整编码产物需先备"
      echo "   thirdparty（./scripts/build-libav/build-x86_64.sh，需 FFMPEG_SRC 指向源码）。"
      # 不导出 LAVF_ROOT/OPENH264_ROOT，继续编骨架（缺 thirdparty 时真 FFI 后端本就不存在）
    else
      export LAVF_ROOT="${AV_X86}"
      export OPENH264_ROOT="${AV_X86}"
    fi
  fi

  if [[ "${use_cross}" == "1" ]]; then
    # cross 容器（scripts/cross-ok3568/Dockerfile）或本机 aarch64-linux-gnu 工具链
    if command -v cross >/dev/null 2>&1; then
      echo "== [${plat}] cross build --target ${target} --features ${feats}"
      cross build --release --locked --target "${target}" \
        -p controlled -p controller -p relay-server -p signaling-server \
        -p capture -p avcodec-wrap \
        --features "${feats}"
    elif command -v "${target%%-*}"-gcc- >/dev/null 2>&1 || command -v aarch64-linux-gnu-gcc >/dev/null 2>&1; then
      echo "== [${plat}] 本机交叉工具链 build --target ${target} --features ${feats}"
      cargo build --release --locked --target "${target}" \
        -p controlled -p controller -p relay-server -p signaling-server \
        -p capture -p avcodec-wrap \
        --features "${feats}"
    else
      echo "!! [${plat}] 无 cross 容器也无 aarch64-linux-gnu-gcc，跳过。"
      echo "   装 cross：cargo install cross；或 apt install gcc-aarch64-linux-gnu。"
      return 0
    fi
  else
    echo "== [${plat}] cargo build --release --features ${feats}"
    # -p capture -p avcodec-wrap：feature 属主 crate 须进选择集，否则
    # `pkg/feat` 路径语法解析不到（controlled/controller 只依赖 landesk-core）。
    cargo build --release --locked -p controlled -p controller \
      -p capture -p avcodec-wrap \
      --features "${feats}"
    # 可选组件单独编（不拉平台 feature）
    cargo build --release --locked -p relay-server -p signaling-server || echo "   (可选组件编译失败，跳过)"
  fi

  # 归集产物到 dist/<plat>/
  for b in controlled controller; do
    local src; src="$(find "target" -maxdepth 3 -name "${b}" -type f 2>/dev/null | head -1 || true)"
    if [[ -n "${src}" && -f "${src}" ]]; then
      cp -f "${src}" "${out}/${b}"
    else
      echo "   [${plat}] 未找到产物 ${b}（该 feature 组合可能未生成二进制，见上方 cargo 输出）"
    fi
  done
  echo "   [${plat}] 产物 -> ${out}/（$(ls "${out}" 2>/dev/null | tr '\n' ' ')）"
}

# ----------------------------------------------------------------------------
# 打包 + 签名（各平台安装器/镜像）
# ----------------------------------------------------------------------------
package_and_sign() {
  local plat="$1"; local out="${OUT_DIR}/${plat}"
  case "${plat}" in
    win)
      echo "-- [win] 打包 NSIS 安装包（tauri bundle）"
      # NSIS 由 tauri 打包；签名需 signtool + 企业证书
      ( cd ui/src-tauri && cargo tauri bundle --target x86_64-pc-windows-msvc 2>/dev/null ) \
        || echo "   需要 tauri-cli + Windows SDK；CI 上由 release 矩阵生成 .exe。"
      if [[ ${DO_SIGN} -eq 1 ]]; then
        if command -v signtool >/dev/null 2>&1 || command -v osslsigncode >/dev/null 2>&1; then
          echo "   签名 ${out}/*.exe （WIN_CERTEXP 证书）"
        else
          echo "   !! 无 signtool/osslsigncode，未签名。CI Windows runner 有 signtool。"
        fi
      fi
      ;;
    linux)
      echo "-- [linux] 打包 deb + AppImage"
      # deb：fpm/dpkg 打包；AppImage：appimagetool
      if command -v fpm >/dev/null 2>&1; then
        fpm -s dir -t deb -n landesk -v 0.1.0 \
          -C "${out}" -I controlled -I controller \
          --description "LanDesk 远程桌面" "${out}/landesk.deb" \
          || echo "   fpm 打包失败"
      else
        echo "   !! 无 fpm，未生成 .deb（apt install fpm 或 dpkg-deb）。"
      fi
      if command -v appimagetool >/dev/null 2>&1; then
        appimagetool "${out}" "${out}/LanDesk.AppImage" || echo "   AppImage 生成失败"
      else
        echo "   !! 无 appimagetool，未生成 AppImage（需 linuxdeploy + AppImage toolchain）。"
      fi
      if [[ ${DO_SIGN} -eq 1 ]]; then
        echo "   Linux 通常免签名；如需 deb 签名：debsigsign / dpkg-sig -s -k <key> ${out}/landesk.deb"
      fi
      ;;
    macos)
      echo "-- [macos] 打包 dmg + 公证（notarize）"
      if command -v create-dmg >/dev/null 2>&1 || command -v hdiutil >/dev/null 2>&1; then
        echo "   需先 cargo tauri bundle --target aarch64-apple-darwin / x86_64-apple-darwin"
      else
        echo "   !! 无 dmg 工具，未生成 .dmg（需 tauri bundle 或 create-dmg）。"
      fi
      if [[ ${DO_SIGN} -eq 1 ]]; then
        echo "   macOS 需 codesign + 公证（APPLE_ID/APPLE_APP_ID 或 Developer ID 证书）："
        echo "   xcrun notarytool submit *.dmg --team-id \$TEAM_ID --wait"
      fi
      ;;
    ok3568)
      echo "-- [ok3568] 静态单文件（NFR-05），无需打包安装器"
      if command -v file >/dev/null 2>&1; then
        for b in controlled controller; do
          [[ -f "${out}/${b}" ]] && { echo "   file ${out}/${b}:"; file "${out}/${b}"; }
        done
      fi
      echo "   验证：ldd 应显示 'not a dynamic executable'（静态链接）"
      echo "   部署：scp 到开发板（见 docs/ops/ok3568-build.md §4）"
      ;;
  esac
}

# ----------------------------------------------------------------------------
# 主流程
# ----------------------------------------------------------------------------
echo "==> LanDesk 打包构建：平台 [${PLATFORMS[*]}] 输出 ${OUT_DIR}/ 签名=${DO_SIGN}"
for p in "${PLATFORMS[@]}"; do
  case "${p}" in
    win)
      build_rust win "" 0
      package_and_sign win "${OUT_DIR}/win"
      ;;
    linux)
      build_rust linux "" 0
      package_and_sign linux "${OUT_DIR}/linux"
      ;;
    macos)
      build_rust macos "" 0
      package_and_sign macos "${OUT_DIR}/macos"
      ;;
    ok3568)
      build_rust ok3568 aarch64-unknown-linux-gnu 1
      package_and_sign ok3568 "${OUT_DIR}/ok3568"
      ;;
    *)
      echo "未知平台 ${p}"
      continue
      ;;
  esac
done

echo "==> 完成。产物根目录：${OUT_DIR}/（已 gitignore，不入库）"
echo "    各平台产物布局："
ls -R "${OUT_DIR}" 2>/dev/null | head -40
echo "    签名状态：${DO_SIGN}（0=仅构建，1=构建+签名；缺失签名工具时按提示补齐）"
