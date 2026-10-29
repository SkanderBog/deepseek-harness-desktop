const NAVIGATION_GLOBAL = '__DSH_NAVIGATION__'

const PRESENTATION_FULL_SIDEBAR = 'full-sidebar'

export function isFullSidebarWindow(): boolean {
  const payload = (globalThis as Record<string, unknown>)[NAVIGATION_GLOBAL]
  if (typeof payload !== 'object' || payload === null)
    return false
  return (payload as { presentation?: unknown }).presentation === PRESENTATION_FULL_SIDEBAR
}
