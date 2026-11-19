#!/usr/bin/env bash
# 裁剪编译 ffmpeg，aarch64 / OK3568（Allwinner VPU，仅 v4l2m2m 硬编 + 软编兜底）。
# 产物输出到 thirdparty/av-aarch64/（.gitignore 已忽略 thirdparty/）。
# 对齐 docs/ops/ok3568-build.md §2：aarch64 硬编通路只有 v4l2m2m。
#
# 用法（交叉工具链由 CC 指定，默认 aarch64-linux-gnu-gcc）：
#   CC=aarch64-linux-gnu-gcc ./scripts/build-libav/build-aarch64-ok3568.sh
set -euo pipefail

CC="${CC:-aarch64-linux-gnu-gcc}"
OUT="${OUT:-thirdparty/av-aarch64}"
JOBS="$(nproc)"

echo "==> building libav into ${OUT} (aarch64 / OK3568, static, CC=${CC})"
mkdir -p "${OUT}"

# 实际 ffmpeg 源码需自行准备（FFMPEG_SRC）
: "${FFMPEG_SRC:?need FFMPEG_SRC pointing to ffmpeg source}"

cd "${FFMPEG_SRC}"
CC="${CC}" ./configure \
  --host=aarch64-linux-gnu \
  --target-os=linux \
  --prefix="${OUT}" \
  --cross-prefix="${CC%-gcc}gcc-" \
  --cc="${CC}" \
  --disable-shared \
  --enable-static \
  --enable-gpl \
  --disable-doc \
  --disable-network \
  --disable-filters \
  --disable-avfilter \
  --enable-avcodec \
  --enable-avutil \
  --enable-swscale \
  --disable-avdevice \
  --disable-avformat \
  --disable-postprocess \
  --disable-ffmpeg --disable-ffplay --disable-ffprobe \
  # OK3568 硬编通路：仅 VPU 的 v4l2m2m（Allwinner）；x86 专属 qsv/nvenc/amf 不适用
  --enable-encoder=h264_v4l2m2m,openh264 \
  --enable-decoder=h264 \
  > "${OUT}/configure.log" 2>&1
make -j"${JOBS}"
make install

echo "==> done."
echo "    使用：LAVF_ROOT=${PWD}/${OUT} OPENH264_ROOT=${PWD}/${OUT}"
echo "          cargo build --release --target aarch64-unknown-linux-gnu \\"
echo "            -p controlled -p controller \\"
echo '            --features "capture/linux,avcodec-wrap/v4l2m2m,avcodec-wrap/openh264"'
