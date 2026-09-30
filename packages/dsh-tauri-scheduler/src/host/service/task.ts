import type { OperationResult, SchedulerSchedule, SchedulerTask, TaskInput } from '../types'
import { randomUUID } from 'node:crypto'
import { defineService } from 'dsh-tauri'
import { conformsTo, defaults, filter, find, findIndex, isArray, isBoolean, isEmpty, isEqual, isNil, isObject, isString, omitBy, pick, reject } from 'lodash-es'
import { withWriteQueue } from '../config/runtime'
import { storage } from '../storage'
import { localTimeZone, nextOccurrence, validateSchedule } from '../utils/schedule'

const SCHEDULER_TASKS_KEY = 'tasks'

const SCHEDULER_PROMPT_MAX_LENGTH = 64_000

const SCHEDULER_NAME_MAX_LENGTH = 120

export const task = defineService({
  async list(search?: string): Promise<SchedulerTask[]> {
    const all = await readAll()
    const needle = search?.trim().toLowerCase() ?? ''
    return needle === '' ? all : filter(all, item => item.name.toLowerCase().includes(needle))
  },

  async get(id: string): Promise<SchedulerTask | null> {
    return find(await readAll(), { id }) ?? null
  },

  async create(input: TaskInput): Promise<OperationResult<{ task: SchedulerTask }>> {
    const invalid = validateInput(input)
    if (invalid !== null)
      return { ok: false, error: invalid }
    const created = build(input)
    await saveTask(created)
    return { ok: true, task: created }
  },

  async update(id: string, patch: Partial<TaskInput>): Promise<OperationResult<{ task: SchedulerTask }>> {
    const current = await findTask(id)
    if (current === null)
      return { ok: false, error: '任务不存在' }
    const merged: TaskInput = defaults({}, patch, current)
    const invalid = validateInput(merged)
    if (invalid !== null)
      return { ok: false, error: invalid }
    const rebuilt = build(merged)
    const updated: SchedulerTask = {
      ...rebuilt,
      id: current.id,
      createdAt: current.createdAt,
      updatedAt: new Date().toISOString(),
      lastRunAt: current.lastRunAt,
      nextRunAt: !isEqual(rebuilt.schedule, current.schedule) || (rebuilt.enabled && isNil(current.nextRunAt))
        ? rebuilt.nextRunAt
        : current.nextRunAt,
    }
    await saveTask(updated)
    return { ok: true, task: updated }
  },

  async remove(id: string): Promise<OperationResult> {
    return await removeTask(id) ? { ok: true } : { ok: false, error: '任务不存在' }
  },

  async toggle(id: string, enabled: boolean): Promise<OperationResult<{ task: SchedulerTask }>> {
    const current = await findTask(id)
    if (current === null)
      return { ok: false, error: '任务不存在' }
    const next: SchedulerTask = { ...current, enabled, updatedAt: new Date().toISOString() }
    if (enabled && isNil(next.nextRunAt)) {
      const occurrence = nextOccurrence(next.schedule, Date.now())
      if (occurrence !== undefined)
        next.nextRunAt = new Date(occurrence).toISOString()
    }
    await saveTask(next)
    return { ok: true, task: next }
  },

  async advance(id: string, lastRunAt?: string, nextRunAt?: string): Promise<void> {
    const current = await findTask(id)
    if (current === null)
      return
    await saveTask({
      ...current,
      lastRunAt,
      nextRunAt,
      updatedAt: new Date().toISOString(),
    })
  },
})

// --- internal ---

async function readAll(): Promise<SchedulerTask[]> {
  const raw = await storage.getItem<{ tasks?: unknown[] }>(SCHEDULER_TASKS_KEY)
  return filter(isArray(raw?.tasks) ? raw.tasks : [], isTask)
}

async function findTask(id: string): Promise<SchedulerTask | null> {
  return find(await readAll(), { id }) ?? null
}

async function saveTask(next: SchedulerTask): Promise<void> {
  await withWriteQueue(async () => {
    const all = await readAll()
    const at = findIndex(all, { id: next.id })
    if (at === -1)
      all.push(next)
    else
      all[at] = next
    await writeTasks(all)
  })
}

async function removeTask(id: string): Promise<boolean> {
  return withWriteQueue(async () => {
    const all = await readAll()
    const remaining = reject(all, { id })
    if (remaining.length === all.length)
      return false
    await writeTasks(remaining)
    return true
  })
}

function isTask(value: unknown): value is SchedulerTask {
  return conformsTo(value, {
    id: isString,
    name: isString,
    prompt: isString,
    enabled: isBoolean,
    schedule: validateSchedule,
  })
}

async function writeTasks(all: SchedulerTask[]): Promise<void> {
  await storage.setItem(SCHEDULER_TASKS_KEY, `${JSON.stringify({ version: 1, tasks: all }, null, 2)}\n`)
}

const OPTIONAL_FIELDS = ['recommendationId', 'workspaceId', 'permission', 'provider', 'model', 'reasoningEffort'] as const

function build(input: TaskInput): SchedulerTask {
  const now = new Date()
  const schedule = input.schedule
  const anchored = (schedule.kind === 'interval' || schedule.kind === 'custom') && isEmpty(schedule.anchor)
    ? { ...schedule, anchor: now.toISOString() }
    : schedule
  const timeZone = isEmpty(anchored.timeZone) ? localTimeZone() : anchored.timeZone
  const normalized = { ...anchored, timeZone } as SchedulerSchedule
  const next = nextOccurrence(normalized, now.getTime())
  return {
    id: `task-${randomUUID()}`,
    name: input.name.trim(),
    schedule: normalized,
    prompt: input.prompt,
    ...omitBy(pick(input, ...OPTIONAL_FIELDS), isEmpty),
    enabled: input.enabled ?? true,
    createdAt: now.toISOString(),
    updatedAt: now.toISOString(),
    nextRunAt: next === undefined ? undefined : new Date(next).toISOString(),
  }
}

function validateInput(input: unknown): string | null {
  if (!isObject(input))
    return '请求体必须是对象'
  const value = input as Partial<TaskInput>
  if (typeof value.name !== 'string' || value.name.trim() === '')
    return '任务名称不能为空'
  if (value.name.trim().length > SCHEDULER_NAME_MAX_LENGTH)
    return `任务名称不能超过 ${SCHEDULER_NAME_MAX_LENGTH} 个字符`
  if (typeof value.prompt !== 'string' || value.prompt.trim() === '')
    return '任务指令不能为空'
  if (value.prompt.length > SCHEDULER_PROMPT_MAX_LENGTH)
    return `任务指令不能超过 ${SCHEDULER_PROMPT_MAX_LENGTH} 个字符`
  if (!validateSchedule(value.schedule))
    return '计划配置无效'
  return null
}
