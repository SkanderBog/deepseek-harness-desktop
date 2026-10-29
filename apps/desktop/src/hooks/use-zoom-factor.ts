import { useMount, useWatch } from '@reause/core'
import { getCurrentWebview } from '@tauri-apps/api/webview'
import { getCurrentWindow } from '@tauri-apps/api/window'
import { useRef, useState } from 'react'

export type ZoomFactorSetter = (value: number) => void

const ZOOM_FACTOR_ERROR = 'the factor must be greater than 0.0.'

function assertZoomFactor(value: number): void {
  if (value === 0)
    throw new Error(ZOOM_FACTOR_ERROR)
}

/**
 * 响应式 WebView 缩放比例（Tauri）。
 *
 * 平台缺口：Tauri 没有 `Webview.getZoom`，唯一可读的比例是 `Window.scaleFactor()`
 * （显示器 DPI 缩放），因此**初值**取它；外部直接调 `setZoom` 的改动无法反映到本 hook。
 *
 * @param factor 显式比例：挂载时应用一次，之后随值变化重应用
 * @returns `[factor, setFactor]`：当前比例与写入函数
 */
export function useZoomFactor(factor?: number): [number, ZoomFactorSetter] {
  const target = getCurrentWebview()

  if (factor !== undefined)
    assertZoomFactor(factor)

  const [value, setValue] = useState<number>(() => factor ?? 1)

  useMount(() => {
    if (factor !== undefined)
      return
    getCurrentWindow()
      .scaleFactor()
      .then(setValue)
      .catch(error => console.error('[useZoomFactor] scaleFactor failed:', error))
  })

  // 最后一次真正写入 `setZoom` 的比例（`null` = 尚未写过，保证显式比例在挂载时也会被应用）
  const lastAppliedRef = useRef<number | null>(null)

  // 显式比例：首次运行即 Electron 版的 `immediate: true`（挂载应用一次），
  // 之后仅在比例变化到不同值时重应用。
  useWatch(factor, (next) => {
    if (next === undefined || next === lastAppliedRef.current)
      return
    assertZoomFactor(next)
    lastAppliedRef.current = next
    setValue(next)
    void target.setZoom(next).catch(error => console.error('[useZoomFactor] setZoom failed:', error))
  }, { immediate: true })

  function setFactor(nextFactor: number) {
    assertZoomFactor(nextFactor)
    // Tauri 没有回读接口：本地状态即「最后一次写入的比例」
    lastAppliedRef.current = nextFactor
    setValue(nextFactor)
    void target.setZoom(nextFactor).catch(error => console.error('[useZoomFactor] setZoom failed:', error))
  }

  return [value, setFactor]
}
