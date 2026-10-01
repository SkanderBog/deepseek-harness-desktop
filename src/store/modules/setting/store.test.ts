import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'

const mocks = vi.hoisted(() => ({
  saved: {} as Record<string, unknown>,
  listeners: new Set<(event: { payload: Record<string, unknown> }) => Promise<void>>(),
}))

vi.mock('@tauri-apps/api/event', () => ({
  listen: vi.fn(async (_event, callback) => {
    mocks.listeners.add(callback)
    return () => mocks.listeners.delete(callback)
  }),
}))
vi.mock('@/config/storage', () => ({
  storage: {
    getItem: async () => JSON.stringify(mocks.saved),
    setItem: async (_key: string, value: string) => { mocks.saved = JSON.parse(value) },
  },
}))

let setting: (typeof import('./store'))['setting']

beforeEach(async () => {
  vi.resetModules()
  mocks.saved = { appearance: { palette: 'default', terminal: false, opacity: 100 }, zoom_factor: 1 }
  ;({ setting } = await import('./store'))
})

afterEach(() => {
  setting.$persist.dehydrate()
  mocks.listeners.clear()
  vi.clearAllMocks()
})

async function nativeUpdate(value: Record<string, unknown>) {
  mocks.saved = value
  await Promise.all([...mocks.listeners].map(callback => callback({ payload: value })))
}

describe('settings synchronization across windows', () => {
  it('applies native settings updates after initial hydration', async () => {
    await nativeUpdate(mocks.saved)
    await nativeUpdate({ ...mocks.saved, appearance: { palette: 'amber', terminal: true, opacity: 80 } })
    expect(setting.appearance).toEqual({ palette: 'amber', terminal: true, opacity: 80 })
  })

  it('keeps another window’s appearance when a local zoom change is saved', async () => {
    await nativeUpdate(mocks.saved)
    await nativeUpdate({ ...mocks.saved, appearance: { palette: 'nord', terminal: false, opacity: 78 } })
    setting.zoom('increase')
    await vi.waitFor(() => expect(mocks.saved.zoom_factor).toBe(1.1))
    expect(mocks.saved.appearance).toEqual({ palette: 'nord', terminal: false, opacity: 78 })
  })
})
