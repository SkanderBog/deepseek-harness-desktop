import type { ClientContext } from 'dsh-tauri/client'
import { defineRegister } from 'dsh-tauri/client'
import { PLUGIN_ID } from '../../shared/constants'
import { NotificationSettingsSection } from '../components/settings-section'
import { SETTINGS_SECTION_ID, SETTINGS_SECTION_ORDER, SETTINGS_SECTION_SLOT } from '../constants'
import { locale } from '../locales'

/**
 * 把通知设置注册进官方设置面板的 `settings.section` 槽位。
 *
 * 槽位名与组件都用 `as never` 收窄：槽位表由官方 UI 包做声明合并，
 * 本包不依赖它们的类型（与 `dsh-tauri-pet` 的做法一致）。
 */
export const sectionFeature = defineRegister<ClientContext>((controller, ctx) => {
  controller.add(ctx.slots.inject(SETTINGS_SECTION_SLOT as never, () => ctx.slots.register({
    name: SETTINGS_SECTION_SLOT,
    id: SETTINGS_SECTION_ID,
    order: SETTINGS_SECTION_ORDER,
    registrant: PLUGIN_ID,
    label: () => locale.text('nav'),
    locale: locale.NS,
    inject: () => ({}),
  } as never, NotificationSettingsSection as never)))
})
