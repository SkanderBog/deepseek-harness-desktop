import type { ClientContext } from 'dsh-tauri/client'
import type { NativeNotificationAction, NotificationKind, PendingInteractionFace, SessionsFace, UiSessionFace } from '../types'
import { defineRegister } from 'dsh-tauri/client'
import { PLUGIN_ID } from '../../shared/constants'
import { APPROVE_ACTION_ID, REJECT_ACTION_ID, REPLY_ACTION_ID } from '../constants'
import { locale } from '../locales'
import { answerQuestion, approveInteraction, pendingDetail, pendingIdentity, pendingNotificationKind, rejectInteraction } from '../service/approval'
import { allowPendingNotification, allowTurnNotification, boundText, notificationTag } from '../service/decision'
import { showNativeNotification } from '../service/native'
import { createSoundPlayer } from '../service/sound'
import { turnEndedByUser } from '../service/turn-end'
import { notificationSettings } from '../store'

/** 状态抖动合并窗口：running 可能连续翻转几次，等它稳定后再判断是否提醒。 */
const SETTLE_MS = 250
/** 通知正文上限。 */
const MAX_BODY_CHARS = 400
/** 会话尚未出现在列表里时，聚焦重试次数与间隔。 */
const FOCUS_RETRY_LIMIT = 6
const FOCUS_RETRY_MS = 200
/** 等待 `uiSession` 服务注册的上限（20 × 250ms = 5s）。 */
const UI_SESSION_WAIT_LIMIT = 20
const UI_SESSION_WAIT_MS = 250

/** 单次观察到的会话状态（`uiSession` 缺席时退化为列表里的 running）。 */
interface SessionObservation {
  readonly running: boolean | undefined
  readonly pending: PendingInteractionFace | undefined
}

/**
 * 通知运行时：把客户端状态变化翻译成原生通知。
 *
 * 数据全部来自客户端服务——`sessions.list`（会话标题、当前会话、running 兜底）与
 * `uiSession.sessionStatus`（运行态与待处理交互）。两者都用最小结构面读取，
 * 服务缺席时降级而不是报错。
 */
export const notifyFeature = defineRegister<ClientContext>((controller, ctx, adapter) => {
  const sound = createSoundPlayer()
  controller.add(sound.dispose)

  const seenRunning = new Map<string, boolean | undefined>()
  const seenPending = new Map<string, string | undefined>()
  const timers = new Map<string, () => void>()
  let sequence = 0

  const readSessions = (): SessionsFace | undefined => ctx.get('sessions') as SessionsFace | undefined
  const readUiSession = (): UiSessionFace | undefined => ctx.get('uiSession') as UiSessionFace | undefined

  const sessionTitle = (sessionId: string): string => {
    const summary = readSessions()?.list.getSnapshot().byId[sessionId]
    return summary?.displayTitle?.trim() || summary?.title?.trim() || locale.text('sessionFallback')
  }

  const focusSession = (sessionId: string, attempt = 0): void => {
    const outcome = adapter.openSession(sessionId)
    if (outcome.status === 'opened')
      return
    if (attempt >= FOCUS_RETRY_LIMIT) {
      console.warn(`[${PLUGIN_ID}] cannot focus session ${sessionId}: ${outcome.reason}`)
      return
    }
    controller.timeout(() => {
      focusSession(sessionId, attempt + 1)
    }, FOCUS_RETRY_MS)
  }

  const approveFromNotification = async (sessionId: string): Promise<void> => {
    const result = await approveInteraction(readUiSession(), sessionId)
    if (!result.ok)
      console.warn(`[${PLUGIN_ID}] approve from notification failed: ${result.error ?? 'unknown'}`)
  }

  const rejectFromNotification = async (sessionId: string): Promise<void> => {
    const result = await rejectInteraction(readUiSession(), sessionId)
    if (!result.ok)
      console.warn(`[${PLUGIN_ID}] reject from notification failed: ${result.error ?? 'unknown'}`)
  }

  const replyFromNotification = async (sessionId: string, text: string): Promise<void> => {
    const result = await answerQuestion(readUiSession(), sessionId, text)
    if (!result.ok)
      console.warn(`[${PLUGIN_ID}] reply from notification failed: ${result.error ?? 'unknown'}`)
  }

  const notify = (sessionId: string, kind: NotificationKind, title: string, body: string, requireInteraction: boolean, actions?: NativeNotificationAction[], onAction?: (action: string, inputValue: string | null) => void): void => {
    const settings = notificationSettings.$state
    sound.play(settings.sound, settings.customSound)
    sequence += 1
    showNativeNotification({
      title,
      body: boundText(body, MAX_BODY_CHARS),
      tag: notificationTag(sessionId, kind, sequence),
      sessionId,
      requireInteraction,
      actions,
      onAction,
      onClick: () => {
        focusSession(sessionId)
      },
    })
  }

  /** 合并窗口结束后再确认状态仍然成立，避免「刚弹通知就被新状态推翻」。 */
  const settle = (key: string, run: () => void): void => {
    timers.get(key)?.()
    const cancel = controller.timeout(() => {
      timers.delete(key)
      run()
    }, SETTLE_MS)
    timers.set(key, cancel)
  }

  const reconcilePending = (sessionId: string, pending: PendingInteractionFace, changed: boolean): void => {
    if (!changed)
      return
    settle(`${sessionId}:pending`, () => {
      const current = readUiSession()?.sessionStatus.getSnapshot().get(sessionId)?.pendingInteraction
      if (!current || pendingIdentity(current) !== pendingIdentity(pending))
        return
      const kind = pendingNotificationKind(current)
      if (!kind)
        return
      if (!allowPendingNotification(notificationSettings.$state, kind))
        return
      const detail = pendingDetail(current)
      const fallback = kind === 'approval' ? locale.text('bodyApproval') : locale.text('bodyQuestion')
      const label = kind === 'approval' ? locale.text('labelApproval') : locale.text('labelQuestion')
      const answerable = current.answerable !== false && typeof current.answer === 'function'
      // 通知按钮一律要求交互仍可回答：答应不了的时候给按钮只会让用户白点。
      // 授权给「批准 / 拒绝」，提问给一个带输入框的「回复」——系统通知渲染不了选项列表，
      // 所以提问只能走自由文本，宿主原样回传输入内容。
      let actions: NativeNotificationAction[] | undefined
      if (answerable) {
        actions = kind === 'approval'
          ? [
              { id: APPROVE_ACTION_ID, title: locale.text('approve') },
              { id: REJECT_ACTION_ID, title: locale.text('reject') },
            ]
          : [{
              id: REPLY_ACTION_ID,
              title: locale.text('reply'),
              input: true,
              inputPlaceholder: locale.text('replyPlaceholder'),
              inputButtonTitle: locale.text('reply'),
            }]
      }
      const onAction = answerable
        ? (action: string, inputValue: string | null): void => {
            if (action === APPROVE_ACTION_ID)
              void approveFromNotification(sessionId)
            else if (action === REJECT_ACTION_ID)
              void rejectFromNotification(sessionId)
            else if (action === REPLY_ACTION_ID && inputValue)
              void replyFromNotification(sessionId, inputValue)
          }
        : undefined
      notify(
        sessionId,
        kind,
        // 标题放会话 title、正文放「动作 · 摘要」：与会话列表、桌宠气泡一致，用户一眼能认出
        // 是哪个会话、要做什么（系统通知的宽度只够一行摘要）。
        sessionTitle(sessionId),
        `${label} · ${detail || fallback}`,
        true,
        actions,
        onAction,
      )
    })
  }

  const reconcileTurn = (sessionId: string, finished: boolean): void => {
    if (!finished)
      return
    settle(`${sessionId}:turn`, () => {
      const status = readUiSession()?.sessionStatus.getSnapshot().get(sessionId)
      const summary = readSessions()?.list.getSnapshot().byId[sessionId]
      if (status ? status.running === true : summary?.running === true)
        return
      const mode = notificationSettings.$state.turnComplete
      const hostHidden = typeof document !== 'undefined' && document.visibilityState === 'hidden'
      const currentSessionId = adapter.sessionList()?.current
      if (!allowTurnNotification({ mode, hostHidden, sessionId, currentSessionId }))
        return
      // 「用户手动中断」和「跑完了」在客户端看到的是同一个 `running: false`：结束原因只在宿主
      // 的 `turn/end` 里，所以弹之前问一次宿主；是中断就什么都不做（用户自己掐断的，不需要提醒）。
      void (async () => {
        if (await turnEndedByUser(sessionId))
          return
        notify(sessionId, 'turn', sessionTitle(sessionId), locale.text('labelTurn'), false)
      })()
    })
  }

  const reconcile = (entries: ReadonlyMap<string, SessionObservation>): void => {
    for (const [sessionId, observed] of entries) {
      const pendingKey = observed.pending ? pendingIdentity(observed.pending) : undefined
      const known = seenRunning.has(sessionId) || seenPending.has(sessionId)
      const previousRunning = seenRunning.get(sessionId)
      const previousPending = seenPending.get(sessionId)
      seenRunning.set(sessionId, observed.running)
      seenPending.set(sessionId, pendingKey)
      // 首次观察只建立基线：插件加载前就存在的完成态与待处理交互不该补弹通知。
      if (!known)
        continue
      if (observed.pending && pendingKey !== previousPending)
        reconcilePending(sessionId, observed.pending, true)
      reconcileTurn(sessionId, previousRunning === true && observed.running !== true)
    }
    for (const sessionId of [...seenRunning.keys()]) {
      if (entries.has(sessionId))
        continue
      seenRunning.delete(sessionId)
      seenPending.delete(sessionId)
      timers.get(`${sessionId}:pending`)?.()
      timers.get(`${sessionId}:turn`)?.()
      timers.delete(`${sessionId}:pending`)
      timers.delete(`${sessionId}:turn`)
    }
  }

  const observe = (): void => {
    const list = readSessions()?.list.getSnapshot()
    if (!list)
      return
    const statuses = readUiSession()?.sessionStatus.getSnapshot()
    const entries = new Map<string, SessionObservation>()
    for (const sessionId of list.ids) {
      const summary = list.byId[sessionId]
      const status = statuses?.get(sessionId)
      entries.set(sessionId, {
        running: status ? status.running : summary?.running,
        pending: status?.pendingInteraction,
      })
    }
    reconcile(entries)
  }

  // 会话列表是常驻服务：用它驱动观察，顺带拿到标题与当前会话。
  const listSource = readSessions()?.list
  if (listSource) {
    controller.add(listSource.subscribe(observe))
    observe()
  }

  // `uiSession` 的注册时机不由本插件决定，最多等 5 秒；等到了就订阅，等不到则维持列表兜底。
  let statusAttached = false
  let statusAttempts = 0
  const attachStatus = (): void => {
    const source = readUiSession()?.sessionStatus
    if (!source) {
      if (statusAttempts < UI_SESSION_WAIT_LIMIT && !controller.isDisposed()) {
        statusAttempts += 1
        controller.timeout(attachStatus, UI_SESSION_WAIT_MS)
      }
      return
    }
    statusAttached = true
    controller.add(source.subscribe(observe))
    observe()
  }
  attachStatus()
  void statusAttached
})
