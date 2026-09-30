/** 手机端判据只看设备能力：触屏（无悬停）+ 主指针粗糙 + 没有任何可悬停指针，窗口缩放与旋转都不会改判。 */
const MOBILE_MEDIA_QUERIES = ['(hover: none)', '(any-pointer: coarse)', '(any-hover: none)'] as const

/** 三条必须同时成立：触屏设备一旦接上鼠标（`any-hover: hover`）仍按桌面端处理。 */
export function isMobileDevice(match: (query: string) => boolean): boolean {
  return MOBILE_MEDIA_QUERIES.every(query => match(query))
}

/** 没有 `window`（Node 单测）或内核缺 `matchMedia` 时一律按桌面端处理。 */
export function detectMobileDevice(): boolean {
  if (typeof window === 'undefined' || typeof window.matchMedia !== 'function')
    return false
  return isMobileDevice(query => window.matchMedia(query).matches)
}
