const WHEEL_LINE_HEIGHT = 28
const WHEEL_SPEED_FACTOR = 1.2

/** 将滚轮的像素、行和页增量统一成当前布局上的像素距离。 */
export function getWheelScrollAmount(event: WheelEvent, pageSize: number): number {
  const delta = Math.abs(event.deltaX) > Math.abs(event.deltaY) ? event.deltaX : event.deltaY
  const scale =
    event.deltaMode === WheelEvent.DOM_DELTA_LINE
      ? WHEEL_LINE_HEIGHT
      : event.deltaMode === WheelEvent.DOM_DELTA_PAGE
        ? pageSize
        : 1
  const amount = delta * scale * WHEEL_SPEED_FACTOR
  return Number.isFinite(amount) ? amount : 0
}
