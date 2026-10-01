import type { SchedulerTask } from '../types'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import { storage } from '../storage'
import { task } from './task'

vi.mock('../storage', () => ({ storage: { getItem: vi.fn(), setItem: vi.fn() } }))

const current: SchedulerTask = {
  id: 'task-1',
  name: 'existing',
  prompt: 'run it',
  schedule: { kind: 'weekly', weekdays: ['FR'], time: '16:00', timeZone: 'UTC' },
  enabled: false,
  permission: 'read-only',
  createdAt: '2026-01-01T00:00:00.000Z',
  updatedAt: '2026-01-01T00:00:00.000Z',
  nextRunAt: '2026-02-06T16:00:00.000Z',
  module: 'legacy-module',
  agentPreset: 'custom-preset',
}

beforeEach(() => {
  vi.clearAllMocks()
  vi.useFakeTimers()
  vi.setSystemTime(new Date('2026-01-02T00:00:00.000Z'))
  vi.mocked(storage.getItem).mockResolvedValue({ tasks: [structuredClone(current)] })
})

afterEach(() => vi.useRealTimers())

describe('task storage and update', () => {
  it('reads persisted protocol fields without pruning them', async () => {
    expect(await task.get(current.id)).toEqual(current)
  })

  it('defaults undefined patches without mutating the caller and keeps deep-equal schedules', async () => {
    const patch = { name: undefined, permission: undefined, schedule: structuredClone(current.schedule) }
    const result = await task.update(current.id, patch)
    expect(result).toMatchObject({ ok: true, task: { name: current.name, permission: current.permission, nextRunAt: current.nextRunAt } })
    expect(patch).toEqual({ name: undefined, permission: undefined, schedule: current.schedule })
    expect(storage.setItem).toHaveBeenCalledWith('tasks', expect.stringContaining(current.nextRunAt!))
  })

  it('recomputes nextRunAt for a changed weekday array', async () => {
    const result = await task.update(current.id, { schedule: { ...current.schedule, kind: 'weekly', weekdays: ['MO'], time: '16:00' } })
    expect(result).toMatchObject({ ok: true, task: { nextRunAt: new Date(2026, 0, 5, 16).toISOString() } })
  })

  it.each([null, false, 0, '', [], { name: 'ok', prompt: 'ok', schedule: { kind: 'hourly', minute: Number.NaN } }])('keeps validation failures for %j', async (input) => {
    await expect(task.create(input as never)).resolves.toMatchObject({ ok: false })
    expect(storage.setItem).not.toHaveBeenCalled()
  })

  it('does not replace an explicitly null required field with a default', async () => {
    await expect(task.update(current.id, { name: null } as never)).resolves.toEqual({ ok: false, error: '任务名称不能为空' })
    expect(storage.setItem).not.toHaveBeenCalled()
  })
})
