/*
 * LanDesk openh264 C ABI 桥接层
 *
 * 把 openh264 的 C++ ISVCEncoder 接口封装成极简 C ABI，供 Rust FFI 调用，
 * 避免 Rust 侧映射复杂 C++ vtable/结构布局。
 *
 * 输入 I420（YUV420P），输出 Annex-B H.264（NALU 自带 00 00 00 01 起始码，
 * 由 openh264 输出，桥接层仅顺序拼接，不额外添加起始码）。
 *
 * openh264: BSD-2-Clause, Copyright (c) 2013 Cisco Systems, Inc.
 * 本文件: MIT（LanDesk）。
 */

#include "codec_api.h"
#include "codec_app_def.h"
#include "codec_def.h"

#include <cstdint>
#include <cstring>

extern "C" {

/*
 * 创建并初始化编码器。成功返回不透明指针，失败返回 nullptr。
 * w/h 必须为正偶数（I420 要求）；fps > 0；bitrate_bps 为目标码率。
 */
void *ld_oh264_create(int w, int h, int fps, int bitrate_bps) {
  if (w <= 0 || h <= 0 || fps <= 0) return nullptr;
  ISVCEncoder *enc = nullptr;
  if (WelsCreateSVCEncoder(&enc) != 0) return nullptr;

  SEncParamBase param;
  std::memset(&param, 0, sizeof(param));
  param.iUsageType = CAMERA_VIDEO_REAL_TIME;
  param.iPicWidth = w;
  param.iPicHeight = h;
  param.iTargetBitrate = bitrate_bps;
  param.iRCMode = RC_BITRATE_MODE;
  param.fMaxFrameRate = static_cast<float>(fps);

  if (enc->Initialize(&param) != 0) {
    WelsDestroySVCEncoder(enc);
    return nullptr;
  }
  int fmt = videoFormatI420;
  enc->SetOption(ENCODER_OPTION_DATAFORMAT, &fmt);
  return enc;
}

/*
 * 编码一帧 I420，输出 Annex-B H.264 到 out（调用方提供缓冲，cap 为容量字节）。
 * 返回输出字节数（>0 成功）；-1 = 编码失败；-2 = 输出缓冲不足。
 * frame_type 输出：1 = IDR（关键帧），3 = P 帧（见 EVideoFrameType）。
 */
int ld_oh264_encode(void *ptr, const uint8_t *i420, int w, int h, int64_t ts,
                    uint8_t *out, int cap, int *frame_type) {
  ISVCEncoder *enc = static_cast<ISVCEncoder *>(ptr);
  if (!enc || !i420 || !out || !frame_type || w <= 0 || h <= 0) return -1;

  SSourcePicture pic;
  std::memset(&pic, 0, sizeof(pic));
  pic.iColorFormat = videoFormatI420;
  pic.iPicWidth = w;
  pic.iPicHeight = h;
  pic.iStride[0] = w;
  pic.iStride[1] = w / 2;
  pic.iStride[2] = w / 2;
  pic.pData[0] = const_cast<uint8_t *>(i420);
  pic.pData[1] = const_cast<uint8_t *>(i420 + (size_t)w * h);
  pic.pData[2] = const_cast<uint8_t *>(i420 + (size_t)w * h + (size_t)w * h / 4);
  pic.uiTimeStamp = ts;

  SFrameBSInfo info;
  std::memset(&info, 0, sizeof(info));
  if (enc->EncodeFrame(&pic, &info) != 0) return -1;
  *frame_type = static_cast<int>(info.eFrameType);

  int total = 0;
  for (int li = 0; li < info.iLayerNum; ++li) {
    SLayerBSInfo &layer = info.sLayerInfo[li];
    for (int n = 0; n < layer.iNalCount; ++n) {
      int len = layer.pNalLengthInByte[n];
      if (total + len > cap) return -2;
      std::memcpy(out + total, layer.pBsBuf, len);
      total += len;
      layer.pBsBuf += len;
    }
  }
  return total;
}

/* 销毁编码器。 */
void ld_oh264_destroy(void *ptr) {
  ISVCEncoder *enc = static_cast<ISVCEncoder *>(ptr);
  if (!enc) return;
  enc->Uninitialize();
  WelsDestroySVCEncoder(enc);
}

}  // extern "C"
