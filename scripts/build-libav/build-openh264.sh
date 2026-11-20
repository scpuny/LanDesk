#!/usr/bin/env bash
# 独立编译 OpenH264（软编兜底，encoder kind=ENC_OPENH264=9）。
# x86_64 与 aarch64 各编一份，输出到 thirdparty/<arch>/。
# 对齐 docs/architecture.md scripts/build-libav/build-openh264.sh
# 与 docs/ops/ok3568-build.md §2/§3（OPENH264_ROOT 供 avcodec-wrap/build.rs 探测）。
#
# 用法：
#   ./scripts/build-libav/build-openh264.sh                       # x86_64
#   CC=aarch64-linux-gnu-gcc ARCH=aarch64 ./scripts/build-libav/build-openh264.sh
set -euo pipefail

CC="${CC:-gcc}"
ARCH="${ARCH:-x86_64}"
OUT="${OUT:-thirdparty/av-${ARCH}}"
JOBS="$(nproc)"

: "${OPENH264_SRC:?need OPENH264_SRC pointing to OpenH264 source (ciscocpi/openh264)}"

echo "==> building OpenH264 into ${OUT} (${ARCH}, static, CC=${CC})"
mkdir -p "${OUT}"

cd "${OPENH264_SRC}"
# OpenH264 提供 autotools 入口（./configure --enable-static）；
# 若源码树是 Android/NDK 分支，需自行切换 openh264-stable 或改用 make 目录。
CC="${CC}" ./configure \
  --prefix="${OUT}" \
  --enable-static \
  --disable-shared \
  > "${OUT}/openh264-configure.log" 2>&1
make -j"${JOBS}"
make install

echo "==> done. set OPENH264_ROOT=${PWD}/${OUT} when building avcodec-wrap"
