import { beforeEach, describe, expect, it, vi } from 'vitest'

// `settings.ts` 的模块级 `createStorage(...)` 会走 `dsh-tauri/client` 的运行时模块表，
// 在 node 下不可求值；这里按仓库既有做法（见 dsh-tauri-ssh 的客户端用例）打桩成内存存储。
// `vi.hoisted` 保证工厂在 import 之前就能拿到这份 Map。
const { memory } = vi.hoisted(() => ({ memory: new Map<string, unknown>() }))

vi.mock('dsh-tauri/client', () => ({
  createStorage: () => ({
    getItem: async (key: string) => memory.get(key) ?? null,
    setItem: async (key: string, value: unknown) => { memory.set(key, value) },
  }),
  localStorageDriver: () => ({}),
}))

const { DEFAULT_SETTINGS, loadSettings, parseSettings, saveSettings } = await import('./settings')

/** 一个合法的自定义提示音（data URL，长度在限制内）。 */
const SOUND_DATA_URL = `data:audio/wav;base64,${'A'.repeat(64)}`

beforeEach(() => {
  memory.clear()
})

describe('dEFAULT_SETTINGS', () => {
  it('与参考实现一致：完成提醒仅在未聚焦时、权限与提问开启、默认提示音', () => {
    expect(DEFAULT_SETTINGS).toEqual({
      turnComplete: 'background',
      approval: true,
      question: true,
      sound: 'default',
      customSound: null,
    })
  })

  it('被冻结，避免调用方就地改写共享默认值', () => {
    expect(Object.isFrozen(DEFAULT_SETTINGS)).toBe(true)
  })
})

describe('parseSettings', () => {
  it('完整合法值原样通过', () => {
    const raw = { turnComplete: 'always', approval: false, question: false, sound: 'classic', customSound: SOUND_DATA_URL }
    expect(parseSettings(raw)).toEqual(raw)
  })

  it('非对象输入整体退化为默认值', () => {
    expect(parseSettings(undefined)).toEqual(DEFAULT_SETTINGS)
    expect(parseSettings(null)).toEqual(DEFAULT_SETTINGS)
    expect(parseSettings('nope')).toEqual(DEFAULT_SETTINGS)
    expect(parseSettings(42)).toEqual(DEFAULT_SETTINGS)
  })

  it('未知枚举值退回默认，而不是透传脏数据', () => {
    expect(parseSettings({ turnComplete: 'sometimes', sound: 'loud' })).toEqual(DEFAULT_SETTINGS)
  })

  it('缺字段时逐项补默认值', () => {
    expect(parseSettings({ turnComplete: 'never' })).toEqual({ ...DEFAULT_SETTINGS, turnComplete: 'never' })
  })

  it('非布尔开关退回默认开启', () => {
    expect(parseSettings({ approval: 'yes', question: 0 })).toEqual(DEFAULT_SETTINGS)
    expect(parseSettings({ approval: false, question: false })).toEqual({ ...DEFAULT_SETTINGS, approval: false, question: false })
  })

  it('自定义提示音必须是 data URL 且在长度上限内', () => {
    expect(parseSettings({ sound: 'custom', customSound: SOUND_DATA_URL }).customSound).toBe(SOUND_DATA_URL)
    expect(parseSettings({ sound: 'custom', customSound: 'https://example.com/a.wav' }).customSound).toBeNull()
    expect(parseSettings({ sound: 'custom', customSound: '' }).customSound).toBeNull()
    expect(parseSettings({ sound: 'custom', customSound: `data:audio/wav;base64,${'A'.repeat(760_001)}` }).customSound).toBeNull()
  })

  it('选了自定义却没有可用音频时退回默认音，避免「选了却没声音」', () => {
    expect(parseSettings({ sound: 'custom' }).sound).toBe('default')
    expect(parseSettings({ sound: 'custom', customSound: 'not-a-data-url' }).sound).toBe('default')
    expect(parseSettings({ sound: 'custom', customSound: SOUND_DATA_URL }).sound).toBe('custom')
  })

  it('非自定义音时仍保留已存的自定义数据，便于切回自定义', () => {
    expect(parseSettings({ sound: 'none', customSound: SOUND_DATA_URL }).customSound).toBe(SOUND_DATA_URL)
    expect(parseSettings({ sound: 'none' }).sound).toBe('none')
  })
})

describe('loadSettings', () => {
  it('存储为空时退化为默认值', async () => {
    await expect(loadSettings()).resolves.toEqual(DEFAULT_SETTINGS)
  })

  it('读取已存设置时收敛成合法值', async () => {
    memory.set('settings', { turnComplete: 'always', approval: false, sound: 'classic' })
    await expect(loadSettings()).resolves.toEqual({
      turnComplete: 'always',
      approval: false,
      question: true,
      sound: 'classic',
      customSound: null,
    })
  })
})

describe('saveSettings', () => {
  it('写回不含运行时态（hydrated）的纯设置', async () => {
    await expect(saveSettings({ ...DEFAULT_SETTINGS, turnComplete: 'never' })).resolves.toEqual({ ok: true })
    expect(memory.get('settings')).toEqual({ ...DEFAULT_SETTINGS, turnComplete: 'never' })
  })
})
