// 画面几何 + 输入编码的纯函数（无 DOM / Vue 依赖，可离线单测）。
//
// 为什么单独抽出来：会话页的「自适应 / 原始尺寸」走 CSS object-fit: contain/none，
// 画布盒子（元素 rect）和真实视频显示矩形不是一回事。鼠标坐标必须按**视频显示矩形**
// 归一化，否则原始尺寸模式（1:1 居中、可能溢出裁切）会把点击落到错误位置。
// 右键的 DOM button=2 与协议码 button=1（右）也不同，集中在这里一处转换。

export interface BoxRect {
  left: number
  top: number
  width: number
  height: number
}

export interface FrameSize {
  width: number
  height: number
}

export interface NormPoint {
  x: number
  y: number
}

export type ScaleMode = '自适应' | '原始尺寸'

/** 按 object-fit 语义算视频在画布盒子里的真实显示矩形。
 *  - 自适应 → contain：等比缩放并居中，黑边在盒子内；
 *  - 原始尺寸 → none：按帧内禀像素 1:1 居中，可能超出盒子（被裁切）。 */
export function contentRect(box: BoxRect, frame: FrameSize, mode: ScaleMode): BoxRect {
  if (box.width <= 0 || box.height <= 0 || frame.width <= 0 || frame.height <= 0) {
    return { left: box.left, top: box.top, width: 0, height: 0 }
  }
  if (mode === '原始尺寸') {
    return {
      left: box.left + (box.width - frame.width) / 2,
      top: box.top + (box.height - frame.height) / 2,
      width: frame.width,
      height: frame.height,
    }
  }
  const scale = Math.min(box.width / frame.width, box.height / frame.height)
  const width = frame.width * scale
  const height = frame.height * scale
  return {
    left: box.left + (box.width - width) / 2,
    top: box.top + (box.height - height) / 2,
    width,
    height,
  }
}

/** client 坐标 → 0..=65535 归一化坐标；落在黑边 / 盒子外返回 null（不转发）。
 *  与受控端 `input::denormalize` 可逆：denormalize(x, screen_w) 还原到屏幕像素。 */
export function normPoint(
  clientX: number,
  clientY: number,
  box: BoxRect,
  frame: FrameSize,
  mode: ScaleMode,
): NormPoint | null {
  const r = contentRect(box, frame, mode)
  if (r.width <= 0 || r.height <= 0) return null
  const rx = (clientX - r.left) / r.width
  const ry = (clientY - r.top) / r.height
  if (rx < 0 || rx > 1 || ry < 0 || ry > 1) return null
  return { x: Math.round(rx * 65535), y: Math.round(ry * 65535) }
}

/** DOM `MouseEvent.button`（0 左 / 1 中 / 2 右）→ 线层协议码（0 左 / 1 右 / 2 中）。
 *  历史 bug：直接把 DOM 值透传，右键 2 被受控端当成中键，表现为「右键没透传」。 */
export function protocolButton(jsButton: number): 0 | 1 | 2 | null {
  if (jsButton === 0) return 0
  if (jsButton === 2) return 1
  if (jsButton === 1) return 2
  return null
}

/** DOM `MouseEvent.button` → MouseMove.buttons 位掩码（1 左 / 2 右 / 4 中）。 */
export function buttonBit(jsButton: number): number {
  if (jsButton === 0) return 1
  if (jsButton === 2) return 2
  if (jsButton === 1) return 4
  return 0
}
