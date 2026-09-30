import type { NotificationSettings, NotificationSettingsState, NotificationSound, TurnCompleteMode } from '../../types'
import { defineStore } from 'dsh-tauri/client'
import { DEFAULT_SETTINGS, loadSettings, saveSettings } from '../../service/settings'

/** 抽出要持久化的字段（`hydrated` 是内存态，不入库）。 */
function persisted(state: NotificationSettings): NotificationSettings {
  return {
    turnComplete: state.turnComplete,
    approval: state.approval,
    question: state.question,
    sound: state.sound,
    customSound: state.customSound,
  }
}

function persist(state: NotificationSettingsState): void {
  void saveSettings(persisted(state))
}

/**
 * 通知设置的唯一真源。
 *
 * `hydrate` 幂等：React 严格模式的双调用、设置面板反复挂载都只会读一次本地存储。
 * 所有写操作先落内存态（UI 立即响应）再异步持久化。
 */
export const notificationSettings = defineStore({
  state: (): NotificationSettingsState => ({ ...DEFAULT_SETTINGS, hydrated: false }),
  actions: {
    async hydrate() {
      if (this.hydrated)
        return
      this.hydrated = true
      const loaded = await loadSettings()
      Object.assign(this, loaded)
    },
    setTurnComplete(mode: TurnCompleteMode) {
      this.turnComplete = mode
      persist(this)
    },
    setApproval(enabled: boolean) {
      this.approval = enabled
      persist(this)
    },
    setQuestion(enabled: boolean) {
      this.question = enabled
      persist(this)
    },
    setSound(sound: NotificationSound) {
      this.sound = sound
      persist(this)
    },
    /** 选中自定义提示音文件；传 `null` 表示移除并（必要时）退回默认音。 */
    setCustomSound(dataUrl: string | null) {
      this.customSound = dataUrl
      if (dataUrl)
        this.sound = 'custom'
      else if (this.sound === 'custom')
        this.sound = DEFAULT_SETTINGS.sound
      persist(this)
    },
  },
})
