import { PLUGIN_ID } from '../../shared/constants'

export { PLUGIN_ID }

/** 设置面板分区 id（`settings.section` 槽位内唯一）。 */
export const SETTINGS_SECTION_ID = 'notification'

/** 设置面板分区顺序：紧随官方会话类分区，与参考实现的 60 对齐。 */
export const SETTINGS_SECTION_ORDER = 60

/** 设置面板分区槽位名。 */
export const SETTINGS_SECTION_SLOT = 'settings.section'

/** 系统通知「批准」按钮的 action id，宿主窗口补丁原样回传该字符串。 */
export const APPROVE_ACTION_ID = 'approve'

/** 系统通知「拒绝」按钮的 action id。 */
export const REJECT_ACTION_ID = 'reject'

/** 系统通知「回复」按钮的 action id：按钮带输入框，用户输入即提问的答案。 */
export const REPLY_ACTION_ID = 'reply'

/** `ctx.effect` 的标签，便于在插件日志里定位。 */
export const LOCALE_EFFECT = `${PLUGIN_ID}: locale`
export const SECTION_EFFECT = `${PLUGIN_ID}: settings section`
export const NOTIFY_EFFECT = `${PLUGIN_ID}: notify`
