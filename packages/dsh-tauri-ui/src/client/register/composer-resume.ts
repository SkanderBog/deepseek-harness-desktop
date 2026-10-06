import type { ClientContext } from 'dsh-tauri/client'
import type { ComposerIconState, ComposerSessionBinding, ComposerSessionSnapshot, ComposerSessionsRuntime } from './composer-resume.types'
import { defineRegister } from 'dsh-tauri/client'
import { PLUGIN_ID } from '../../shared/constants'
import { isContentRiskFailure } from '../../shared/content-risk'
import { locale } from '../locales'
import { resumeComposer } from '../service/composer-resume'
import {
  contentRiskRecoveryBoundary,
  isComposerEmpty,
  lastTurnEndReason,
  paintResumeIcon,
  primaryButtonOf,
  readIconPath,
  restoreDisabled,
  restorePrimaryIcon,
  shouldOfferResume,
} from './composer-resume.utils'

const COMPOSER_CARD_SELECTOR = '[data-composer-card]'

export const composerResumeFeature = defineRegister<ClientContext>((controller, ctx, adapter) => {
  if (typeof document === 'undefined')
    return

  const sessions = adapter.sessions as ComposerSessionsRuntime
  const sessionsList = sessions.list
  const sessionsBinding = sessions.binding
  if (sessionsList === undefined || sessionsBinding === undefined)
    return

  let binding: ComposerSessionBinding | undefined
  let watchedSessionId: string | undefined
  let unwatchEvents: (() => void) | undefined
  let unwatchSession: (() => void) | undefined
  let patch: { button: HTMLButtonElement, icon: ComposerIconState } | undefined
  let pending = false

  const resumeLabel = (): string => locale.text('resumeTask')

  const snapshotNow = (): ComposerSessionSnapshot | undefined => binding?.session?.getSnapshot?.()

  const sessionIdNow = (): string | undefined => sessionsList.getSnapshot().current

  const bindingOf = (sessionId: string): ComposerSessionBinding | undefined => {
    const resolved = sessionsBinding(sessionId)
    return typeof resolved === 'object' && resolved !== null ? resolved as ComposerSessionBinding : undefined
  }

  function restore(card?: Element | null): void {
    const current = patch
    if (current === undefined)
      return
    patch = undefined
    const anchor = card ?? document.querySelector(COMPOSER_CARD_SELECTOR)
    const snapshot = snapshotNow()
    const disabled = anchor !== null && restoreDisabled(
      isComposerEmpty(anchor),
      snapshot?.running === true,
      snapshot?.subagent !== null && snapshot?.subagent !== undefined,
    )
    restorePrimaryIcon(current.button, current.icon, { label: resumeLabel(), disabled })
  }

  function paint(button: HTMLButtonElement): void {
    if (patch?.button !== button) {
      const path = readIconPath(button)
      if (path === null) {
        button.disabled = false
        return
      }
      restore()
      patch = { button, icon: { path, ariaLabel: button.getAttribute('aria-label') } }
    }
    paintResumeIcon(button, resumeLabel())
  }

  function reconcile(): void {
    if (controller.isDisposed())
      return
    const card = document.querySelector(COMPOSER_CARD_SELECTOR)
    const entries = binding?.eventSource?.getSnapshot?.().entries
    if (card !== null && isComposerEmpty(card) && shouldOfferResume({ session: snapshotNow(), entries })) {
      const button = primaryButtonOf(card)
      if (button !== null) {
        paint(button)
        return
      }
    }
    restore(card)
  }

  function unwatch(): void {
    unwatchEvents?.()
    unwatchEvents = undefined
    unwatchSession?.()
    unwatchSession = undefined
  }

  function refresh(): void {
    const sessionId = sessionIdNow()
    if (sessionId !== watchedSessionId) {
      watchedSessionId = sessionId
      unwatch()
      const next = sessionId === undefined ? undefined : bindingOf(sessionId)
      if (typeof next?.eventSource?.subscribe === 'function')
        unwatchEvents = next.eventSource.subscribe(reconcile)
      if (typeof next?.session?.subscribe === 'function')
        unwatchSession = next.session.subscribe(reconcile)
    }
    binding = sessionId === undefined ? undefined : bindingOf(sessionId)
    reconcile()
  }

  async function resume(): Promise<void> {
    const sessionId = sessionIdNow()
    if (sessionId === undefined || pending)
      return
    pending = true
    try {
      const source = bindingOf(sessionId)
      const eventSource = source?.eventSource
      let recoveryBoundary: number | undefined
      let entries = eventSource?.getSnapshot?.().entries
      if (isContentRiskFailure(lastTurnEndReason(entries)?.error)) {
        if (eventSource === undefined) {
          console.warn(`[${PLUGIN_ID}] 无法读取内容审核错误的会话事件`)
          return
        }
        let eventSnapshot = eventSource.getSnapshot()
        recoveryBoundary = contentRiskRecoveryBoundary(entries, { historyComplete: eventSnapshot.hasMore !== true })
        let firstSeq = entries?.[0]?.event?.seq
        const sourceSession = source?.session
        const loadOlder = sourceSession?.loadOlder?.bind(sourceSession)
        if (loadOlder !== undefined) {
          while (recoveryBoundary === undefined && eventSource.getSnapshot().hasMore === true) {
            await loadOlder()
            eventSnapshot = eventSource.getSnapshot()
            entries = eventSnapshot.entries
            const nextFirstSeq = entries?.[0]?.event?.seq
            recoveryBoundary = contentRiskRecoveryBoundary(entries, { historyComplete: eventSnapshot.hasMore !== true })
            if (nextFirstSeq === firstSeq)
              break
            firstSeq = nextFirstSeq
          }
        }
        if (recoveryBoundary === undefined) {
          console.warn(`[${PLUGIN_ID}] 无法定位内容审核错误之前的安全恢复边界`)
          return
        }
      }
      if (sessionIdNow() !== sessionId)
        return
      let outcome: Awaited<ReturnType<typeof resumeComposer>>
      if (recoveryBoundary === undefined) {
        outcome = await resumeComposer({ sessionId })
      }
      else {
        const fork = sessions.fork
        if (typeof fork !== 'function') {
          console.warn(`[${PLUGIN_ID}] 当前内核不支持安全分叉，已拒绝在原会话重放内容审核错误`)
          return
        }
        outcome = await resumeComposer({
          sessionId,
          recovery: {
            atSeq: recoveryBoundary,
            sessions: { fork: options => fork.call(sessions, options) },
            navigation: { open: childId => adapter.openSession(childId) },
          },
        })
      }
      if (!outcome.ok)
        console.warn(`[${PLUGIN_ID}] 会话继续失败: ${outcome.error ?? 'unknown'}`)
    }
    catch (error) {
      console.warn(`[${PLUGIN_ID}] 会话继续失败: ${error instanceof Error ? error.message : String(error)}`)
    }
    finally {
      pending = false
      reconcile()
    }
  }

  controller.add(sessionsList.subscribe(refresh))
  controller.add(ctx.locale.subscribe(() => {
    if (patch !== undefined)
      paintResumeIcon(patch.button, resumeLabel())
  }))
  controller.add(unwatch)
  controller.listen('click', (event) => {
    if (patch === undefined)
      return
    const button = event.target instanceof Element ? event.target.closest('button') : null
    if (button !== patch.button)
      return
    event.preventDefault()
    event.stopPropagation()
    void resume()
  }, { capture: true })
  controller.observe(document.body, () => reconcile(), {
    attributeFilter: ['d', 'disabled'],
    attributes: true,
    childList: true,
    subtree: true,
  })

  refresh()
})
