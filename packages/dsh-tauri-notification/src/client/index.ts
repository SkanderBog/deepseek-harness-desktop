import type { ClientContext } from 'dsh-tauri/client'
import { PLUGIN_ID } from '../shared/constants'
import { LOCALE_EFFECT, NOTIFY_EFFECT, SETTINGS_EFFECT } from './constants'
import { locale } from './locales'
import { notifyFeature } from './register/notify'
import { settingsFeature } from './register/settings'
import { notificationSettings } from './store/modules/settings'

export const name = PLUGIN_ID

/**
 * `uiSession` 不写进 inject：它在部分上下文里可能缺席，通知运行时用
 * `ctx.get('uiSession')` 探测并降级，而不是让整个插件卡在等待注入。
 */
export const inject = ['slots', 'locale', 'sessions']

export function apply(ctx: ClientContext): void {
  // 先把本地存储里的设置读回内存：通知运行时与设置面板都直接读 store，
  // 不 hydrate 的话每次启动都从默认值开始，用户改过的选项会被首次写回覆盖。
  void notificationSettings.hydrate()
  ctx.effect(locale.registerLocale, LOCALE_EFFECT)
  ctx.effect(settingsFeature, SETTINGS_EFFECT)
  ctx.effect(notifyFeature, NOTIFY_EFFECT)
}
