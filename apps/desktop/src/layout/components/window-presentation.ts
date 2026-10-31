/** 壳层建窗时注入的全局名（与 `src-tauri/src/desktop/window.rs` 逐字一致）。 */
const NAVIGATION_GLOBAL = '__DSH_NAVIGATION__'

/** 全屏右栏窗口的载荷标记（与插件 `sidebar-fullwindow.ts` 逐字一致）。 */
const PRESENTATION_FULL_SIDEBAR = 'full-sidebar'

/**
 * 本窗口是否由「在新窗口全屏打开」建出。
 *
 * 注入脚本对全部 frame 生效：壳层与内嵌 dsh 各读自己 realm 的同一份载荷，
 * 所以这里读壳层全局，不需要插件经 iframe 转发。
 */
export function isFullSidebarWindow(): boolean {
  const payload = (globalThis as Record<string, unknown>)[NAVIGATION_GLOBAL]
  if (typeof payload !== 'object' || payload === null)
    return false
  return (payload as { presentation?: unknown }).presentation === PRESENTATION_FULL_SIDEBAR
}
