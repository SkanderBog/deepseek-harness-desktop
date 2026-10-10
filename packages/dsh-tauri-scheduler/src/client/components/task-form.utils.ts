import type { ScheduleForm, SchedulerOptions, TaskFormState, TaskInput, TaskView } from '../types'
import { cloneDeep } from 'dsh-tauri/client'

export function defaultScheduleFor(kind: ScheduleForm['kind'], timeZone = new Intl.DateTimeFormat().resolvedOptions().timeZone): ScheduleForm {
  switch (kind) {
    case 'once': return { kind, at: new Date(Date.now() + 3_600_000).toISOString(), timeZone }
    case 'hourly': return { kind, minute: 0, timeZone }
    case 'interval': return { kind, everyMinutes: 30, timeZone }
    case 'monthly': return { kind, day: 1, time: '09:00', timeZone }
    case 'custom': return { kind, everyDays: 2, time: '09:00', timeZone }
    case 'weekly': return { kind, weekdays: ['MO'], time: '09:00', timeZone }
    case 'workdays': return { kind, time: '09:00', timeZone }
    default: return { kind: 'daily', time: '09:00', timeZone }
  }
}

export function emptyTaskForm(options: SchedulerOptions, sessionId = ''): TaskFormState {
  return {
    delivery: 'new-session',
    sessionId,
    enabled: true,
    name: '',
    prompt: '',
    schedule: defaultScheduleFor('daily'),
    workspaceId: '',
    permission: options.defaultPermission || 'read-only',
    provider: options.defaultModel?.provider ?? '',
    model: options.defaultModel?.model ?? '',
    reasoningEffort: options.defaultModel?.reasoning?.defaultEffort ?? '',
  }
}

export function taskToForm(task: TaskView): TaskFormState {
  return {
    delivery: task.delivery,
    sessionId: task.sessionId ?? '',
    enabled: task.enabled,
    recommendationId: task.recommendationId,
    name: task.name,
    schedule: cloneDeep(task.schedule),
    prompt: task.prompt,
    workspaceId: task.workspaceId ?? '',
    permission: task.permission ?? 'read-only',
    provider: task.provider ?? '',
    model: task.model ?? '',
    reasoningEffort: task.reasoningEffort ?? '',
  }
}

export function taskFormInput(form: TaskFormState, editing: boolean): TaskInput {
  const input: TaskInput = {
    delivery: form.delivery,
    name: form.name,
    prompt: form.prompt,
    schedule: cloneDeep(form.schedule),
    enabled: form.enabled,
    recommendationId: form.recommendationId,
  }
  if (form.delivery === 'this-session') {
    input.sessionId = form.sessionId
    if (editing)
      Object.assign(input, { workspaceId: '', permission: '', provider: '', model: '', reasoningEffort: '' })
  }
  else {
    Object.assign(input, {
      sessionId: editing ? '' : undefined,
      workspaceId: form.workspaceId,
      permission: form.permission,
      provider: form.provider,
      model: form.model,
      reasoningEffort: form.reasoningEffort,
    })
  }
  return input
}

export function localDateTime(at: string): string {
  const date = new Date(at)
  if (!Number.isFinite(date.getTime()))
    return ''
  const part = (value: number) => String(value).padStart(2, '0')
  return `${date.getFullYear()}-${part(date.getMonth() + 1)}-${part(date.getDate())}T${part(date.getHours())}:${part(date.getMinutes())}:${part(date.getSeconds())}`
}

export function absoluteDateTime(value: string): string {
  const date = new Date(value)
  return Number.isFinite(date.getTime()) ? date.toISOString() : ''
}
