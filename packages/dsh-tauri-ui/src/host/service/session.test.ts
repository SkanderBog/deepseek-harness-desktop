import type { HostContext } from '../types'
import type { PlanSession } from './session.types'
import { Session, SessionId } from '@deepseek-ai/dsh-session'
import { afterEach, describe, expect, it } from 'vitest'
import { apply } from '../apply'
import { setCurrentHostInstance } from '../config/runtime'
import { session } from './session'

interface SetupOptions {
  status?: string
  events?: unknown
  session?: unknown
  loader?: unknown
}

function turnEnd(kind: string) {
  return { type: 'turn/end', seq: 1, time: 0, data: { turn: 1, reason: { kind } } }
}

function setup(options: SetupOptions = {}): { followed: Array<{ content: unknown, source: unknown }> } {
  const followed: Array<{ content: unknown, source: unknown }> = []
  const agent = {
    status: options.status ?? 'idle',
    session: options.session ?? { snapshotEvents: () => options.events ?? [turnEnd('completed')] },
    followup: (message: { content: unknown, source: unknown }) => void followed.push(message),
  }
  setCurrentHostInstance({
    agents: { get: (id: string) => (id === 'unknown' ? undefined : agent) },
    loader: options.loader ?? {
      import: async () => ({ createUserMessage: (input: unknown) => input }),
      unwrapExports: (value: unknown) => value,
    },
    logger: { warn: () => {} },
  } as HostContext)
  return { followed }
}

afterEach(() => setCurrentHostInstance(undefined))

const interruptedPlan = [
  { content: '调查根因', status: 'completed' },
  { content: '修复继续操作', status: 'in_progress' },
  { content: '验证修复', status: 'pending' },
]

function setupTurn(kind: 'aborted' | 'interrupted' | 'error' = 'aborted') {
  const log = Session.create(SessionId('s1'))
  log.append('turn/start', { turn: 1 })
  const planLog = log as unknown as PlanSession
  planLog.append('todo/write', { todos: interruptedPlan })
  const reason = kind === 'aborted'
    ? { kind, reason: { kind: 'user' as const } }
    : kind === 'error' ? { kind, error: { message: 'test failure', code: 'UNKNOWN' } } : { kind }
  log.append('turn/end', { turn: 1, reason })
  const followed: Array<{ content: unknown, source: unknown }> = []
  const agent = { status: 'idle', session: log, followup: (message: { content: unknown, source: unknown }) => void followed.push(message) }
  const hooks = new Map<string, (payload: unknown, next: () => Promise<unknown>) => unknown>()
  apply({
    agents: { get: () => agent },
    loader: { import: async () => ({ createUserMessage: (input: unknown) => input }), unwrapExports: (value: unknown) => value },
    on: (name: string, callback: (payload: unknown, next: () => Promise<unknown>) => unknown) => hooks.set(name, callback),
    effect: () => {},
  } as HostContext)
  async function preStep(messages = followed, step = 1, decision: unknown = { kind: 'enter', messages }, signal = new AbortController().signal) {
    const handler = hooks.get('agent/pre-step')
    expect(handler).toBeTypeOf('function')
    const outcome = await handler!({ agent, messages, turn: log.snapshotEvents().filter(event => event.type === 'turn/start').length, step, signal }, async () => decision)
    expect(outcome).toBe(decision)
  }
  return {
    log,
    planLog,
    followed,
    preStep,
    async start(messages = followed, decision?: unknown) {
      const turn = log.snapshotEvents().filter(event => event.type === 'turn/start').length + 1
      log.append('turn/start', { turn })
      await preStep(messages, 1, decision)
    },
  }
}

function projectedTodos(log: Session): unknown {
  return log.snapshotEvents().reduce<unknown>((state, event) => {
    if (event.type === 'turn/start')
      return null
    if ((event.type as string) === 'todo/write')
      return (event.data as { todos: unknown }).todos
    return state
  }, null)
}

describe('continued turn plan', () => {
  it.each(['aborted', 'interrupted', 'error'] as const)('restores every todo and status when continuing an %s turn', async (kind) => {
    const { log, start, preStep } = setupTurn(kind)
    expect(await session.resume('s1')).toEqual({ ok: true })
    await start()
    expect(projectedTodos(log)).toEqual(interruptedPlan)
    expect(log.snapshotEvents().at(-1)).toMatchObject({ type: 'todo/write', data: { todos: interruptedPlan } })
    expect(projectedTodos(Session.create(SessionId('replayed'), log.snapshotEvents()))).toEqual(interruptedPlan)
    await preStep()
    await preStep(undefined, 2)
    expect(log.snapshotEvents().filter(event => (event.type as string) === 'todo/write')).toHaveLength(2)
  })

  it('keeps the normal new-task reset instead of restoring an unrelated plan', async () => {
    const { log, start } = setupTurn()
    await start([{ content: [], source: { kind: 'user' } }])
    expect(projectedTodos(log)).toBeNull()
  })

  it('restores the latest plan instead of an earlier checklist', async () => {
    const { log, planLog, start } = setupTurn()
    log.append('turn/start', { turn: 2 })
    const updated = [{ content: '后续任务', status: 'in_progress' }]
    planLog.append('todo/write', { todos: updated })
    log.append('turn/end', { turn: 2, reason: { kind: 'interrupted' } })
    expect(await session.resume('s1')).toEqual({ ok: true })
    await start()
    expect(projectedTodos(log)).toEqual(updated)
  })

  it('preserves an explicitly emptied checklist', async () => {
    const { log, planLog, start } = setupTurn()
    log.append('turn/start', { turn: 2 })
    planLog.append('todo/write', { todos: [] })
    log.append('turn/end', { turn: 2, reason: { kind: 'aborted', reason: { kind: 'user' } } })
    expect(await session.resume('s1')).toEqual({ ok: true })
    await start()
    expect(projectedTodos(log)).toEqual([])
  })

  it('does not restore for rejected or replaced continuation admission', async () => {
    const { log, start, preStep } = setupTurn()
    expect(await session.resume('s1')).toEqual({ ok: true })
    await start(undefined, { kind: 'reject' })
    expect(projectedTodos(log)).toBeNull()
    await preStep(undefined, 1, { kind: 'enter', messages: [{ source: { kind: 'user' } }] })
    expect(projectedTodos(log)).toBeNull()
  })

  it('restores the plan when continuation is admitted alongside injected context', async () => {
    const { log, followed, start } = setupTurn()
    expect(await session.resume('s1')).toEqual({ ok: true })
    await start([...followed, { content: [], source: { kind: 'model-change', form: 'notice' } }])
    expect(projectedTodos(log)).toEqual(interruptedPlan)
  })

  it('does not overwrite a newer plan or restore on a later step', async () => {
    const { log, planLog, preStep } = setupTurn()
    expect(await session.resume('s1')).toEqual({ ok: true })
    log.append('turn/start', { turn: 2 })
    await preStep(undefined, 2)
    expect(projectedTodos(log)).toBeNull()
    const updated = [{ content: '新计划', status: 'pending' }]
    planLog.append('todo/write', { todos: updated })
    await preStep()
    expect(projectedTodos(log)).toEqual(updated)
    expect(log.snapshotEvents().filter(event => (event.type as string) === 'todo/write')).toHaveLength(2)
  })

  it('does not append after admission was cancelled or the turn already ended', async () => {
    const { log, preStep } = setupTurn()
    expect(await session.resume('s1')).toEqual({ ok: true })
    log.append('turn/start', { turn: 2 })
    const controller = new AbortController()
    controller.abort()
    await preStep(undefined, 1, undefined, controller.signal)
    expect(projectedTodos(log)).toBeNull()
    log.append('turn/end', { turn: 2, reason: { kind: 'aborted', reason: { kind: 'user' } } })
    await preStep()
    expect(projectedTodos(log)).toBeNull()
  })

  it('does not restore a continuation batched with a new user task', async () => {
    const { log, followed, start } = setupTurn()
    expect(await session.resume('s1')).toEqual({ ok: true })
    await start([...followed, { content: [], source: { kind: 'user' } }])
    expect(projectedTodos(log)).toBeNull()
  })

  it('does not resurrect a plan from an earlier task with no plan in the interrupted turn', async () => {
    const { log, start } = setupTurn()
    log.append('turn/start', { turn: 2 })
    log.append('turn/end', { turn: 2, reason: { kind: 'aborted', reason: { kind: 'user' } } })
    expect(await session.resume('s1')).toEqual({ ok: true })
    await start()
    expect(projectedTodos(log)).toBeNull()
    expect(log.snapshotEvents().filter(event => (event.type as string) === 'todo/write')).toHaveLength(1)
  })
})

describe('session.resume', () => {
  it('continues a session whose turn was aborted by the user', async () => {
    const { followed } = setup({ events: [{ type: 'turn/start' }, turnEnd('aborted')] })
    expect(await session.resume('s1')).toEqual({ ok: true })
    expect(followed).toHaveLength(1)
    expect(followed[0]?.source).toEqual({ kind: 'continue' })
  })

  it('continues interrupted and errored turns', async () => {
    setup({ events: [turnEnd('interrupted')] })
    expect(await session.resume('s1')).toEqual({ ok: true })

    const { followed } = setup({ events: [turnEnd('error')] })
    expect(await session.resume('s1')).toEqual({ ok: true })
    expect(followed).toHaveLength(1)
  })

  it('refuses a turn that settled normally, naming the reason', async () => {
    for (const kind of ['completed', 'blocked', 'max-tokens']) {
      const { followed } = setup({ events: [turnEnd(kind)] })
      const outcome = await session.resume('s1')
      expect(outcome).toEqual({ ok: false, code: 409, error: expect.stringContaining(kind) })
      expect(followed).toHaveLength(0)
    }
  })

  it('refuses a running session and an unknown one', async () => {
    setup({ status: 'running' })
    expect(await session.resume('s1')).toEqual({ ok: false, code: 409, error: '会话仍在运行，无需继续' })
    expect(await session.resume('unknown')).toEqual({ ok: false, code: 404, error: '会话不存在或尚未运行' })
  })

  it('reads the log through the legacy fallbacks', async () => {
    const log = [turnEnd('aborted')]
    setup({ session: { log } })
    expect(await session.resume('s1')).toEqual({ ok: true })
    setup({ session: { events: log } })
    expect(await session.resume('s1')).toEqual({ ok: true })
  })

  it('treats an unreadable log as continuable rather than blocking the user', async () => {
    setup({ session: {} })
    expect(await session.resume('s1')).toEqual({ ok: true })
  })

  it('reports a missing runtime module instead of throwing', async () => {
    setup({ events: [turnEnd('aborted')], loader: { import: async () => ({}), unwrapExports: () => ({}) } })
    const outcome = await session.resume('s1')
    expect(outcome.ok).toBe(false)
    expect(outcome.ok === false && outcome.code).toBe(500)
  })

  it('reports 500 with the DSH_LOADER_MISSING literal when the host exposes no loader', async () => {
    const followed: unknown[] = []
    setCurrentHostInstance({
      agents: {
        get: () => ({
          status: 'idle',
          session: { snapshotEvents: () => [turnEnd('aborted')] },
          followup: (message: unknown) => void followed.push(message),
        }),
      },
      logger: { warn: () => {} },
    } as HostContext)
    expect(await session.resume('s1')).toEqual({
      ok: false,
      code: 500,
      error: 'TypeError: DSH_LOADER_MISSING: ctx.loader',
    })
    expect(followed).toHaveLength(0)
  })

  it('reports 500 with the DSH_LOADER_MISSING literal when the loader lacks import()', async () => {
    setup({ events: [turnEnd('aborted')], loader: { unwrapExports: (value: unknown) => value } })
    expect(await session.resume('s1')).toEqual({
      ok: false,
      code: 500,
      error: 'TypeError: DSH_LOADER_MISSING: ctx.loader',
    })
  })

  it('reports 500 with the DSH_LLM_EXPORT_MISSING literal when import() carries no createUserMessage', async () => {
    const { followed } = setup({
      events: [turnEnd('aborted')],
      loader: { import: async () => ({ createUserMessage: 'not-a-function' }), unwrapExports: () => undefined },
    })
    expect(await session.resume('s1')).toEqual({
      ok: false,
      code: 500,
      error: 'TypeError: DSH_LLM_EXPORT_MISSING: createUserMessage',
    })
    expect(followed).toHaveLength(0)
  })

  it('turns a throwing loader into a 500 naming the thrown error', async () => {
    setup({
      events: [turnEnd('aborted')],
      loader: { import: async () => { throw new Error('boom') }, unwrapExports: (value: unknown) => value },
    })
    expect(await session.resume('s1')).toEqual({ ok: false, code: 500, error: 'Error: boom' })
  })
})
