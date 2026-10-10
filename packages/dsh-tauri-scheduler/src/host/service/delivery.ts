import type { UserMessage } from '@deepseek-ai/dsh-llm'
import type { HostContext, OperationResult, RunTrigger, SchedulerTask } from '../types'
import type { PlatformModuleLoader } from '../utils/agent-runtime.types'
import { getServerContext } from 'dsh-h3/utils'
import { defineService } from 'dsh-tauri'
import { runtime } from '../config/runtime'
import { server } from '../server'
import { loadSchedulerMessageFactory } from '../utils/agent-runtime'
import { nextFutureOccurrence } from '../utils/occurrence'
import { sameTaskRecord } from '../utils/task-record'
import { history } from './history'
import { task } from './task'

export const delivery = defineService({
  async run(target: SchedulerTask, trigger: RunTrigger, scheduledAt: string): Promise<OperationResult> {
    const ctx = getServerContext<HostContext>(server)
    if (typeof ctx.sessionController?.resolveAgent !== 'function' || typeof ctx.sessionController?.inspect !== 'function' || typeof ctx.sessions?.flush !== 'function')
      return { ok: false, error: '宿主无法恢复或持久化目标会话', code: 'session_unavailable' }
    let pending = runtime.pending.get(target.id) ?? (await history.pending(target.id))[0]
    let enqueued = runtime.pending.has(target.id)
    if (pending && !enqueued) {
      const inspected = await ctx.sessionController.inspect(pending.task.sessionId!)
      enqueued = containsMessage(inspected.events, pending.message.id)
    }
    const snapshot = pending?.task ?? target
    const occurrence = pending?.scheduledAt ?? scheduledAt
    const runTrigger = pending?.trigger ?? trigger
    if (!enqueued) {
      const allowed = await canEnqueue(snapshot, runTrigger, occurrence)
      if (allowed === 'changed') {
        if (pending)
          await history.discardPending(target.id)
        return { ok: true }
      }
      if (allowed === 'future')
        return { ok: false, error: '计划尚未到期', code: 'delivery_not_due' }
    }
    const bound = await task.validateTarget(snapshot)
    if (!bound.ok)
      return bound
    const resolved = await ctx.sessionController.resolveAgent(snapshot.sessionId!)
    if (!resolved.agent)
      return { ok: false, error: resolved.error?.message ?? '无法恢复目标会话', code: resolved.error?.code ?? 'session_unavailable' }
    const agent = resolved.agent
    const rechecked = await task.validateTarget(snapshot)
    if (!rechecked.ok)
      return rechecked
    if (agent.session?.id !== snapshot.sessionId)
      return { ok: false, error: '恢复的会话与任务绑定不匹配', code: 'session_mismatch' }
    if (!enqueued) {
      const allowed = await canEnqueue(snapshot, runTrigger, occurrence)
      if (allowed === 'changed') {
        if (pending)
          await history.discardPending(target.id)
        return { ok: true }
      }
      if (allowed === 'future')
        return { ok: false, error: '计划尚未到期', code: 'delivery_not_due' }
    }
    if (!pending) {
      const createMessage = await loadSchedulerMessageFactory(ctx.loader as PlatformModuleLoader)
      const message = createMessage({
        content: [{ type: 'text', text: framing(target, scheduledAt) }],
        source: { kind: 'schedule', form: 'notice', summary: `Scheduled: ${target.name}`.slice(0, 120) },
      })
      if (typeof message.id !== 'string')
        throw new TypeError('SCHEDULER_MESSAGE_ID_MISSING')
      pending = { task: target, trigger, scheduledAt, deliveredAt: new Date().toISOString(), message }
      await history.prepare(pending)
    }
    if (!enqueued) {
      const allowed = await canEnqueue(snapshot, runTrigger, occurrence)
      if (allowed === 'changed') {
        await history.discardPending(target.id)
        return { ok: true }
      }
      if (allowed === 'future')
        return { ok: false, error: '计划尚未到期', code: 'delivery_not_due' }
      if (ctx.workspaceRegistry.archivedSessionIds.includes(snapshot.sessionId))
        return { ok: false, error: '目标会话已归档', code: 'session_archived' }
      agent.followup(pending.message)
    }
    runtime.pending.set(target.id, pending)
    const flushed = await ctx.sessions.flush(agent.session)
    if (flushed !== true)
      return { ok: false, error: 'Session persistence did not acknowledge the reminder', code: 'delivery_pending' }
    pending.deliveredAt = new Date().toISOString()
    const next = pending.trigger === 'schedule' ? nextFutureOccurrence(pending.task.schedule, Date.parse(pending.scheduledAt), Date.now()) : undefined
    pending.nextRunAt = next === undefined ? undefined : new Date(next).toISOString()
    await history.commit(pending)
    await task.complete(pending.task, pending.deliveredAt, pending.nextRunAt, pending.trigger === 'schedule')
    await history.acknowledge(pending.message.id)
    runtime.pending.delete(target.id)
    return { ok: true }
  },
})

async function canEnqueue(snapshot: SchedulerTask, trigger: RunTrigger, scheduledAt: string): Promise<'ready' | 'changed' | 'future'> {
  const current = await task.get(snapshot.id)
  if (!sameTaskRecord(current, snapshot) || current?.status !== 'active' || (trigger === 'schedule' && !current.enabled))
    return 'changed'
  return trigger === 'schedule' && Date.parse(scheduledAt) > Date.now() ? 'future' : 'ready'
}

function framing(target: SchedulerTask, scheduledAt: string): string {
  return [
    '[SCHEDULE REMINDER]',
    'This is a scheduled message from the user.',
    'This reminder may have been queued while the session was busy. Check the current conversation before acting; do not override a newer request or repeat completed work. Ask for clarification if its context is stale.',
    `schedule_id_json: ${JSON.stringify(target.id)}`,
    `occurrence_at: ${scheduledAt}`,
    `reminder_prompt_json: ${JSON.stringify(target.prompt)}`,
  ].join('\n')
}

function containsMessage(events: readonly { type: string, data?: { inserted?: readonly UserMessage[], message?: { id?: string } } }[], id: string): boolean {
  return events.some(event => event.data?.message?.id === id || (event.type === 'agent/inbox/spliced' && event.data?.inserted?.some(message => message.id === id)))
}
