#!/usr/bin/env bash
# ============================================================================
# LanDesk 图标生成器 —— 从 SVG 单一真源渲染全套应用图标
#
# 依赖（本地、零成本）：
#   - rsvg-convert  (SVG -> PNG)
#   - iconutil      (macOS，生成标准 .icns)
#   - python3 + Pillow (生成多尺寸 .ico)
#
# 用法：  ./scripts/build-icons.sh
# 设计源：ui/src-tauri/icons/source.svg
# 输出：  ui/src-tauri/icons/  +  ui/frontend/public/favicon.*
# ============================================================================
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
ICON_DIR="${ROOT}/ui/src-tauri/icons"
SRC="${ICON_DIR}/source.svg"
WEB_DIR="${ROOT}/ui/frontend/public"

command -v rsvg-convert >/dev/null || { echo "缺少 rsvg-convert" >&2; exit 1; }
command -v iconutil >/dev/null || { echo "缺少 iconutil（需 macOS）" >&2; exit 1; }
python3 -c 'import PIL' 2>/dev/null || { echo "缺少 Pillow" >&2; exit 1; }
[[ -f "$SRC" ]] || { echo "找不到设计源: $SRC" >&2; exit 1; }

mkdir -p "$ICON_DIR" "$WEB_DIR"
echo "[icons] 设计源: $SRC"

# --- 1. 各尺寸 PNG（rsvg 矢量渲染，边缘清晰）---
png() {
  local size="$1" out="$2"
  rsvg-convert -w "$size" -h "$size" "$SRC" -o "$out"
  echo "  PNG $size -> $(basename "$out")"
}
png 32  "${ICON_DIR}/32x32.png"
png 128 "${ICON_DIR}/128x128.png"
png 256 "${ICON_DIR}/128x128@2x.png"
png 256 "${ICON_DIR}/icon.png"
png 512 "${ICON_DIR}/icon-512.png"
png 1024 "${ICON_DIR}/icon-1024.png"

# --- 2. 多尺寸 ICO（Windows 任务栏/资源管理器各尺寸）---
python3 - "${ICON_DIR}" <<'PY'
import sys, os
from PIL import Image
icon_dir = sys.argv[1]
sizes = [(16,16),(24,24),(32,32),(48,48),(64,64),(128,128),(256,256)]
base = Image.open(os.path.join(icon_dir, 'icon-1024.png')).convert('RGBA')
out = os.path.join(icon_dir, 'icon.ico')
base.save(out, format='ICO', sizes=sizes)
print(f'  ICO {sizes} -> icon.ico ({os.path.getsize(out)}B)')
PY

# --- 3. 标准 ICNS（iconutil 从 iconset 生成，含全部 10 个标准尺寸）---
# iconutil 强制要求目录名以 .iconset 结尾，否则报 "Invalid Iconset"
ICONSET="$(mktemp -d "${TMPDIR:-/tmp}/landesk.XXXXXX").iconset"
mv "${ICONSET%.iconset}" "${ICONSET}"
iset() { rsvg-convert -w "$1" -h "$1" "$SRC" -o "${ICONSET}/icon_$2.png"; }
iset 16   "16x16"
iset 32   "16x16@2x"
iset 32   "32x32"
iset 64   "32x32@2x"
iset 128  "128x128"
iset 256  "128x128@2x"
iset 256  "256x256"
iset 512  "256x256@2x"
iset 512  "512x512"
iset 1024 "512x512@2x"
iconutil -c icns "${ICONSET}" -o "${ICON_DIR}/icon.icns"
rm -rf "${ICONSET}"
echo "  ICNS -> icon.icns ($(wc -c < "${ICON_DIR}/icon.icns" | tr -d ' ')B)"

# --- 4. 前端 favicon ---
cp "${ICON_DIR}/32x32.png" "${WEB_DIR}/favicon.png"
cp "${ICON_DIR}/icon.ico"  "${WEB_DIR}/favicon.ico"
echo "  favicon -> ui/frontend/public/favicon.{png,ico}"

echo "[icons] 完成。"
