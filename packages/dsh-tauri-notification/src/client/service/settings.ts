import type { NotificationSettings } from '../types'
import { createStorage, localStorageDriver } from 'dsh-tauri/client'
import { PLUGIN_ID } from '../../shared/constants'

/** 存储键：unstorage 的 base 已隔离到插件自身命名空间。 */
const SETTINGS_KEY = 'settings'

/** 自定义提示音的 data URL 上限，与 512 KB 的原始文件上限留出 base64 膨胀余量。 */
const MAX_CUSTOM_SOUND_CHARS = 760_000

const TURN_MODES = ['never', 'background', 'always'] as const
const SOUNDS = ['default', 'classic', 'none', 'custom'] as const

/** 默认设置：与参考实现（问题/权限开、完成提醒仅在未聚焦时、默认提示音）一致。 */
export const DEFAULT_SETTINGS: NotificationSettings = Object.freeze({
  turnComplete: 'background',
  approval: true,
  question: true,
  sound: 'default',
  customSound: null,
})

const storage = createStorage({ driver: localStorageDriver({ base: PLUGIN_ID }) })

function asBoolean(value: unknown, fallback: boolean): boolean {
  return typeof value === 'boolean' ? value : fallback
}

function asCustomSound(value: unknown): string | null {
  if (typeof value !== 'string' || !value.startsWith('data:') || value.length > MAX_CUSTOM_SOUND_CHARS)
    return null
  return value
}

/** 把任意来源（本地存储、旧版本写入的残缺值）收敛成合法设置。 */
export function parseSettings(raw: unknown): NotificationSettings {
  const source: Record<string, unknown> = raw && typeof raw === 'object' ? raw as Record<string, unknown> : {}
  const turnComplete = typeof source.turnComplete === 'string' && (TURN_MODES as readonly string[]).includes(source.turnComplete)
    ? source.turnComplete as NotificationSettings['turnComplete']
    : DEFAULT_SETTINGS.turnComplete
  const customSound = asCustomSound(source.customSound)
  const picked = typeof source.sound === 'string' && (SOUNDS as readonly string[]).includes(source.sound)
    ? source.sound as NotificationSettings['sound']
    : DEFAULT_SETTINGS.sound
  return {
    turnComplete,
    approval: asBoolean(source.approval, DEFAULT_SETTINGS.approval),
    question: asBoolean(source.question, DEFAULT_SETTINGS.question),
    // 选自定义但没有可用的音频数据时退回默认音，避免「选了却没声音」。
    sound: picked === 'custom' && !customSound ? DEFAULT_SETTINGS.sound : picked,
    customSound,
  }
}

export interface SaveSettingsResult {
  ok: boolean
  error?: string
}

/** 读取设置；任何异常（无 localStorage、配额、解析失败）都退化为默认值。 */
export async function loadSettings(): Promise<NotificationSettings> {
  try {
    return parseSettings(await storage.getItem(SETTINGS_KEY))
  }
  catch (error) {
    console.warn(`[${PLUGIN_ID}] failed to read settings`, error)
    return { ...DEFAULT_SETTINGS }
  }
}

/** 写入设置；失败只记录日志，UI 永远以内存态为准。 */
export async function saveSettings(settings: NotificationSettings): Promise<SaveSettingsResult> {
  try {
    await storage.setItem(SETTINGS_KEY, { ...settings })
    return { ok: true }
  }
  catch (error) {
    console.warn(`[${PLUGIN_ID}] failed to save settings`, error)
    return { ok: false, error: error instanceof Error ? error.message : String(error) }
  }
}
