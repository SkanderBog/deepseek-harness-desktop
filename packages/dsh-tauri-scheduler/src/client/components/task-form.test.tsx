import type { SessionId } from 'dsh-tauri/client'
// @vitest-environment jsdom
import type { ComponentProps } from 'react'
import { act, cleanup, fireEvent, render, screen } from '@testing-library/react'
import { StrictMode } from 'react'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import { createTask, updateTask } from '../service/scheduler'
import { deferred, formFixture, optionsFixture, sessionsFixture, taskFixture, translate, workspacesFixture } from './scheduler-client.test.harness'
import { TaskForm } from './task-form'

vi.mock('../service/scheduler', () => ({ createTask: vi.fn(), updateTask: vi.fn() }))
vi.mock('@deepseek-ai/dsh-client-ui-primitives', async () => {
  const ui = await import('./scheduler-client.test.ui')
  return { Button: ui.PrimitiveButton, Input: ui.PrimitiveInput, Menu: ui.PrimitiveMenu }
})
vi.mock('dsh-tauri-ui/client', async () => (await import('./scheduler-client.test.ui')).schedulerClientUi())

beforeEach(() => {
  vi.resetAllMocks()
  vi.mocked(createTask).mockResolvedValue({ ok: true, task: taskFixture() })
  vi.mocked(updateTask).mockResolvedValue({ ok: true, task: taskFixture() })
})

afterEach(() => {
  cleanup()
  vi.restoreAllMocks()
})

function renderForm(overrides: Partial<ComponentProps<typeof TaskForm>> = {}, strict = false) {
  const props: ComponentProps<typeof TaskForm> = {
    t: translate,
    options: optionsFixture(),
    sessions: sessionsFixture(),
    workspaces: workspacesFixture(),
    onClose: vi.fn(),
    onSaved: vi.fn(),
    defaultSessionId: 'session-a',
    initial: formFixture(),
    ...overrides,
  }
  const view = render(strict ? <StrictMode><TaskForm {...props} /></StrictMode> : <TaskForm {...props} />)
  return { ...view, props }
}

async function submit(view: ReturnType<typeof renderForm>): Promise<void> {
  const form = view.container.querySelector('form')
  expect(form).not.toBeNull()
  await act(async () => {
    fireEvent.submit(form!)
  })
}

async function select(label: string, option: string): Promise<void> {
  await act(async () => {
    fireEvent.click(screen.getByRole('button', { name: label }))
  })
  await act(async () => {
    fireEvent.click(screen.getByText(option, { exact: true }))
  })
}

describe('taskForm schedule editing', () => {
  it('preserves a custom schedule anchor when editing its days and wall time', async () => {
    const view = renderForm({ task: taskFixture({ schedule: { kind: 'custom', everyDays: 4, time: '16:45', anchor: '2030-05-01T05:06:07.000Z', timeZone: 'Pacific/Auckland' } }) })
    fireEvent.change(view.getByRole('spinbutton', { name: 'scheduleEveryDays' }), { target: { value: '6' } })
    fireEvent.change(view.getByLabelText('scheduleTime'), { target: { value: '18:19' } })
    await submit(view)
    expect(updateTask).toHaveBeenCalledTimes(1)
    expect(vi.mocked(updateTask).mock.calls[0]![1].schedule).toEqual({ kind: 'custom', everyDays: 6, time: '18:19', anchor: '2030-05-01T05:06:07.000Z', timeZone: 'Pacific/Auckland' })
  })

  it('preserves an interval anchor when editing its minutes and timezone', async () => {
    const view = renderForm({ task: taskFixture({ schedule: { kind: 'interval', everyMinutes: 17, anchor: '2030-05-01T05:06:07.000Z', timeZone: 'America/New_York' } }) })
    fireEvent.change(view.getByRole('spinbutton', { name: 'scheduleEveryMinutes' }), { target: { value: '23' } })
    fireEvent.change(view.getByLabelText('timeZone'), { target: { value: 'Europe/Berlin' } })
    await submit(view)
    expect(vi.mocked(updateTask).mock.calls[0]![1].schedule).toEqual({ kind: 'interval', everyMinutes: 23, anchor: '2030-05-01T05:06:07.000Z', timeZone: 'Europe/Berlin' })
  })

  it('retains all selected weekdays during unrelated edits and changes only the toggled days', async () => {
    const view = renderForm({ task: taskFixture() })
    expect((view.getByRole('checkbox', { name: 'dayMon' }) as HTMLInputElement).checked).toBe(true)
    expect((view.getByRole('checkbox', { name: 'dayWed' }) as HTMLInputElement).checked).toBe(true)
    expect((view.getByRole('checkbox', { name: 'dayFri' }) as HTMLInputElement).checked).toBe(true)
    fireEvent.change(view.getByLabelText('taskName'), { target: { value: 'Renamed weekly task' } })
    fireEvent.click(view.getByRole('checkbox', { name: 'dayTue' }))
    fireEvent.click(view.getByRole('checkbox', { name: 'dayWed' }))
    fireEvent.change(view.getByLabelText('scheduleTime'), { target: { value: '11:12' } })
    await submit(view)
    expect(vi.mocked(updateTask).mock.calls[0]![1]).toMatchObject({ name: 'Renamed weekly task', schedule: { kind: 'weekly', weekdays: ['MO', 'FR', 'TU'], time: '11:12', timeZone: 'Asia/Shanghai' } })
  })

  it.each([
    { kind: 'daily', time: '09:30', timeZone: 'UTC' },
    { kind: 'workdays', time: '09:30', timeZone: 'UTC' },
    { kind: 'weekly', weekdays: ['MO', 'WE'], time: '09:30', timeZone: 'UTC' },
    { kind: 'monthly', day: 7, time: '09:30', timeZone: 'UTC' },
    { kind: 'custom', everyDays: 4, anchor: '2030-05-01T05:06:07.000Z', time: '09:30', timeZone: 'UTC' },
  ] as const)('accepts minute-only wall time with step 60 for $kind schedules', async (schedule) => {
    const view = renderForm({ task: taskFixture({ schedule }) })
    const time = view.getByLabelText('scheduleTime') as HTMLInputElement
    expect(time.type).toBe('time')
    expect(time.step).toBe('60')
    expect(time.value).toBe('09:30')
    fireEvent.change(time, { target: { value: '18:19' } })
    expect(time.checkValidity()).toBe(true)
    await submit(view)
    expect(updateTask).toHaveBeenCalledTimes(1)
    expect(vi.mocked(updateTask).mock.calls[0]![1].schedule).toEqual({ ...schedule, time: '18:19' })
  })

  it('allows a fractional interval of 1.5 minutes to pass native validation and submit', async () => {
    const view = renderForm({ initial: formFixture({ schedule: { kind: 'interval', everyMinutes: 30, timeZone: 'UTC' } }) })
    const interval = view.getByRole('spinbutton', { name: 'scheduleEveryMinutes' }) as HTMLInputElement
    expect(interval.step).toBe('any')
    expect(Number(interval.min)).toBeGreaterThan(0)
    expect(Number(interval.min)).toBeLessThan(1.5)
    fireEvent.change(interval, { target: { value: '1.5' } })
    expect(interval.value).toBe('1.5')
    expect(interval.checkValidity()).toBe(true)
    await submit(view)
    expect(createTask).toHaveBeenCalledTimes(1)
    expect(vi.mocked(createTask).mock.calls[0]![0].schedule).toEqual({ kind: 'interval', everyMinutes: 1.5, timeZone: 'UTC' })
  })

  it('keeps the explicitly chosen timezone when changing schedule kind', async () => {
    const view = renderForm({ initial: formFixture({ schedule: { kind: 'daily', time: '13:00', timeZone: 'Pacific/Auckland' } }) })
    await select('schedule', 'scheduleInterval')
    await submit(view)
    expect(vi.mocked(createTask).mock.calls[0]![0].schedule).toEqual({ kind: 'interval', everyMinutes: 30, timeZone: 'Pacific/Auckland' })
  })
})

describe('taskForm delivery conversion', () => {
  it('hides and omits independent resources for a new this-session reminder', async () => {
    const view = renderForm({ initial: formFixture({ delivery: 'this-session' }) })
    expect(view.queryByRole('button', { name: 'workspace' })).toBeNull()
    expect(view.queryByRole('button', { name: 'permission' })).toBeNull()
    expect(view.queryByRole('button', { name: 'trigger.selectAria' })).toBeNull()
    expect(view.getByText('delivery.inherited').textContent).toBe('delivery.inherited')
    await submit(view)
    expect(createTask).toHaveBeenCalledExactlyOnceWith({
      delivery: 'this-session',
      sessionId: 'session-a',
      name: 'Draft task',
      prompt: 'Draft instruction',
      enabled: true,
      recommendationId: 'recommendation-a',
      schedule: { kind: 'daily', time: '09:30', timeZone: 'Asia/Shanghai' },
    })
  })

  it('clears stale resources when an independent task is converted to this-session', async () => {
    const view = renderForm({ task: taskFixture() })
    await select('delivery', 'delivery.this-session')
    await submit(view)
    expect(updateTask).toHaveBeenCalledTimes(1)
    expect(vi.mocked(updateTask).mock.calls[0]![1]).toEqual({
      delivery: 'this-session',
      sessionId: 'session-a',
      name: 'Task A',
      prompt: 'Original instruction',
      enabled: true,
      recommendationId: 'recommendation-a',
      schedule: { kind: 'weekly', weekdays: ['MO', 'WE', 'FR'], time: '09:30', timeZone: 'Asia/Shanghai' },
      workspaceId: '',
      permission: '',
      provider: '',
      model: '',
      reasoningEffort: '',
    })
  })

  it('clears an old linked session when a reminder is converted to an independent task', async () => {
    const view = renderForm({ task: taskFixture({ delivery: 'this-session', sessionId: 'session-a' }) })
    await select('delivery', 'delivery.new-session')
    await submit(view)
    expect(vi.mocked(updateTask).mock.calls[0]![1]).toEqual({
      delivery: 'new-session',
      sessionId: '',
      name: 'Task A',
      prompt: 'Original instruction',
      enabled: true,
      recommendationId: 'recommendation-a',
      schedule: { kind: 'weekly', weekdays: ['MO', 'WE', 'FR'], time: '09:30', timeZone: 'Asia/Shanghai' },
      workspaceId: 'workspace-a',
      permission: 'workspace-write',
      provider: 'provider-a',
      model: 'model-a',
      reasoningEffort: 'high',
    })
  })

  it.each([
    ['loading', sessionsFixture({ phase: 'pending' }), workspacesFixture()],
    ['loading', sessionsFixture(), workspacesFixture({ phase: 'pending' })],
    ['archived', sessionsFixture(), workspacesFixture({ archivedSessionIds: ['session-a' as SessionId] })],
    ['unavailable', sessionsFixture({ ids: [], byId: {} }), workspacesFixture()],
    ['unavailable', sessionsFixture(), workspacesFixture({ state: 'error' })],
  ] as const)('blocks submission when the linked session is %s', async (state, sessions, workspaces) => {
    const view = renderForm({ initial: formFixture({ delivery: 'this-session' }), sessions, workspaces })
    expect((view.getByRole('button', { name: 'save' }) as HTMLButtonElement).disabled).toBe(true)
    expect(view.getByRole('alert').textContent).toBe(`session.${state}`)
    await submit(view)
    expect(createTask).not.toHaveBeenCalled()
  })

  it('allows submission after an unopened linked session arrives in the list', async () => {
    const view = renderForm({ initial: formFixture({ delivery: 'this-session' }), sessions: sessionsFixture({ phase: 'pending', ids: [], byId: {} }) })
    expect((view.getByRole('button', { name: 'save' }) as HTMLButtonElement).disabled).toBe(true)
    view.rerender(<TaskForm {...view.props} sessions={sessionsFixture()} />)
    expect((view.getByRole('button', { name: 'save' }) as HTMLButtonElement).disabled).toBe(false)
    await submit(view)
    expect(createTask).toHaveBeenCalledTimes(1)
  })
})

describe('taskForm submission lifetime', () => {
  it('accepts a completed save after StrictMode effect replay', async () => {
    const response = deferred<Awaited<ReturnType<typeof createTask>>>()
    vi.mocked(createTask).mockImplementationOnce(() => response.promise)
    const view = renderForm({}, true)
    await submit(view)
    expect(createTask).toHaveBeenCalledTimes(1)
    await act(async () => {
      response.resolve({ ok: true, task: taskFixture() })
      await response.promise
    })
    expect(view.props.onSaved).toHaveBeenCalledExactlyOnceWith(taskFixture())
    expect((view.getByRole('button', { name: 'save' }) as HTMLButtonElement).disabled).toBe(false)
  })

  it('sends the original expected snapshot even after the external task is mutated and rerendered', async () => {
    const original = taskFixture()
    const view = renderForm({ task: original })
    original.prompt = 'External replacement'
    original.updatedAt = '2030-01-03T00:00:00.000Z'
    original.schedule.timeZone = 'UTC'
    if (original.schedule.kind !== 'weekly')
      throw new Error('Expected weekly task fixture')
    original.schedule.time = '20:00'
    original.schedule.weekdays = ['TU']
    view.rerender(<TaskForm {...view.props} task={original} />)
    fireEvent.change(view.getByLabelText('taskName'), { target: { value: 'My retained draft' } })
    await submit(view)
    expect(updateTask).toHaveBeenCalledTimes(1)
    const [id, input, expected] = vi.mocked(updateTask).mock.calls[0]!
    expect(id).toBe('task-a')
    expect(input).toMatchObject({ name: 'My retained draft', prompt: 'Original instruction', schedule: { kind: 'weekly', weekdays: ['MO', 'WE', 'FR'], time: '09:30', timeZone: 'Asia/Shanghai' } })
    expect(expected).toEqual(taskFixture())
    expect(expected).not.toBe(original)
    expect(expected.schedule).not.toBe(original.schedule)
  })

  it('retains the complete draft after an asynchronous save failure and permits a retry', async () => {
    const first = deferred<Awaited<ReturnType<typeof createTask>>>()
    vi.mocked(createTask).mockImplementationOnce(() => first.promise)
    const view = renderForm({ initial: formFixture({ schedule: { kind: 'custom', everyDays: 4, time: '16:45', anchor: '2030-05-01T05:06:07.000Z', timeZone: 'Pacific/Auckland' } }) })
    fireEvent.change(view.getByLabelText('taskName'), { target: { value: 'Unsaved task' } })
    fireEvent.change(view.getByPlaceholderText('schedulePromptPlaceholder'), { target: { value: 'Unsaved multiline\ninstruction' } })
    fireEvent.change(view.getByRole('spinbutton', { name: 'scheduleEveryDays' }), { target: { value: '7' } })
    await submit(view)
    await act(async () => {
      first.resolve({ ok: false, error: 'Storage unavailable' })
      await first.promise
    })
    expect(view.getByRole('alert').textContent).toBe('Storage unavailabletask.conflictHint')
    expect((view.getByLabelText('taskName') as HTMLInputElement).value).toBe('Unsaved task')
    expect((view.getByPlaceholderText('schedulePromptPlaceholder') as HTMLTextAreaElement).value).toBe('Unsaved multiline\ninstruction')
    expect((view.getByRole('spinbutton', { name: 'scheduleEveryDays' }) as HTMLInputElement).value).toBe('7')
    expect((view.getByLabelText('timeZone') as HTMLInputElement).value).toBe('Pacific/Auckland')
    expect(view.props.onSaved).not.toHaveBeenCalled()
    await submit(view)
    expect(createTask).toHaveBeenCalledTimes(2)
    expect(vi.mocked(createTask).mock.calls[1]![0]).toEqual({
      delivery: 'new-session',
      sessionId: undefined,
      enabled: true,
      recommendationId: 'recommendation-a',
      name: 'Unsaved task',
      prompt: 'Unsaved multiline\ninstruction',
      schedule: { kind: 'custom', everyDays: 7, time: '16:45', anchor: '2030-05-01T05:06:07.000Z', timeZone: 'Pacific/Auckland' },
      workspaceId: 'workspace-a',
      permission: 'workspace-write',
      provider: 'provider-a',
      model: 'model-a',
      reasoningEffort: 'high',
    })
    expect(view.props.onSaved).toHaveBeenCalledExactlyOnceWith(taskFixture())
    expect(view.queryByRole('alert')).toBeNull()
  })

  it('guards duplicate submit events before React can render the saving state', async () => {
    const response = deferred<Awaited<ReturnType<typeof createTask>>>()
    vi.mocked(createTask).mockImplementationOnce(() => response.promise)
    const view = renderForm()
    const form = view.container.querySelector('form')!
    await act(async () => {
      fireEvent.submit(form)
      fireEvent.submit(form)
      fireEvent.submit(form)
    })
    expect(createTask).toHaveBeenCalledTimes(1)
    expect((view.getByRole('button', { name: 'loading' }) as HTMLButtonElement).disabled).toBe(true)
    expect((view.getByRole('button', { name: 'cancel' }) as HTMLButtonElement).disabled).toBe(true)
    await act(async () => {
      response.resolve({ ok: true, task: taskFixture() })
      await response.promise
    })
    expect(view.props.onSaved).toHaveBeenCalledExactlyOnceWith(taskFixture())
  })

  it.each([false, true])('does not call onSaved after a pending save outlives an unmounted form with StrictMode=%s', async (strict) => {
    const response = deferred<Awaited<ReturnType<typeof createTask>>>()
    vi.mocked(createTask).mockImplementationOnce(() => response.promise)
    const view = renderForm({}, strict)
    await submit(view)
    view.unmount()
    await act(async () => {
      response.resolve({ ok: true, task: taskFixture() })
      await response.promise
    })
    expect(view.props.onSaved).not.toHaveBeenCalled()
    expect(createTask).toHaveBeenCalledTimes(1)
    expect(view.container.textContent).toBe('')
  })

  it('never submits a form disabled by an inactive task owner', async () => {
    const view = renderForm({ task: taskFixture({ status: 'inactive', enabled: false }), disabled: true })
    expect((view.getByRole('button', { name: 'save' }) as HTMLButtonElement).disabled).toBe(true)
    await submit(view)
    expect(updateTask).not.toHaveBeenCalled()
    expect(view.props.onSaved).not.toHaveBeenCalled()
  })
})
