#!/usr/bin/env bash
# 裁剪编译 ffmpeg（仅保留 LanDesk 需要的组件），x86_64。
# 产物输出到 thirdparty/av-x86_64/（.gitignore 已忽略 thirdparty/）。
# 对齐 docs/ops/ok3568-build.md §2：只保留 avcodec/avutil/swscale + qsv/nvenc/amf/v4l2m2m。
set -euo pipefail

OUT="${OUT:-thirdparty/av-x86_64}"
JOBS="$(nproc)"

echo "==> building libav into ${OUT} (x86_64, static)"
mkdir -p "${OUT}"

# 裁剪编译（仅核心组件 + 目标编码器，禁用无关 codec/demux/mux）
# 实际 ffmpeg 源码需自行准备（FFMPEG_SRC），这里给出配置基线。
: "${FFMPEG_SRC:?need FFMPEG_SRC pointing to ffmpeg source}"

cd "${FFMPEG_SRC}"
./configure \
  --prefix="${OUT}" \
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
  --enable-encoder=libx264,h264_qsv,h264_nvenc,h264_amf,h264_v4l2m2m,openh264 \
  --enable-decoder=h264 \
  > "${OUT}/configure.log" 2>&1
make -j"${JOBS}"
make install

echo "==> done. set LAVF_ROOT=${PWD}/${OUT} when building avcodec-wrap"
