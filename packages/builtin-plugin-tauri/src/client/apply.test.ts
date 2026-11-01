import type { ClientContext } from './types'
import { afterEach, expect, it, vi } from 'vitest'
import { apply } from './apply'

const tauri = vi.hoisted(() => ({ available: false }))

vi.mock('./service/tauri', () => ({ isTauri: () => tauri.available }))
vi.mock('./register/account', () => ({ accountSignInFeature: () => {} }))
vi.mock('./register/appearance', () => ({ registerAppearance: () => {} }))
vi.mock('./register/key-repeat', () => ({ keyRepeatFeature: () => {} }))
vi.mock('./register/navigation', () => ({ navigationFeature: () => {} }))
vi.mock('./register/shortcuts', () => ({ shortcutsFeature: () => {} }))
vi.mock('./register/sidebar', () => ({ sidebarFeature: () => {} }))
vi.mock('./register/sidebar-fullwindow', () => ({ sidebarFullwindowFeature: () => {} }))
vi.mock('./register/sidebar-tweaks', () => ({ sidebarTweaksFeature: () => {} }))
vi.mock('./register/style', () => ({ registerStyle: () => {} }))
vi.mock('./register/zoom-shortcut', () => ({ zoomShortcutFeature: () => {} }))

afterEach(() => {
  tauri.available = false
  Reflect.deleteProperty(globalThis, 'NEARTOKEN_AFF')
  vi.unstubAllGlobals()
})

function fakeCtx(labels: string[]): ClientContext {
  const ctx = {
    effect(_callback: unknown, label?: string) {
      if (label !== undefined)
        labels.push(label)
    },
    inject(_names: string[], callback: (scoped: ClientContext) => void) {
      callback(ctx as unknown as ClientContext)
    },
  }
  return ctx as unknown as ClientContext
}

function affiliateCode(): unknown {
  return Reflect.get(globalThis, 'NEARTOKEN_AFF')
}

it('独立浏览器不写邀请码全局', () => {
  vi.stubGlobal('window', {})
  vi.stubGlobal('document', {})
  const labels: string[] = []
  apply(fakeCtx(labels))
  expect(affiliateCode()).toBeUndefined()
  expect(labels).toEqual([])
})

it('内嵌页面注入 NearToken 邀请码后再登记桌面 effect', () => {
  vi.stubGlobal('window', {})
  vi.stubGlobal('document', {})
  tauri.available = true
  const labels: string[] = []
  apply(fakeCtx(labels))
  expect(affiliateCode()).toBe('yKfg')
  expect(labels).toHaveLength(10)
})
