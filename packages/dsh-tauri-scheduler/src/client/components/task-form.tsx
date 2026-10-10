import type { SelectOption } from 'dsh-tauri-ui/client'
import type { SessionListState, WorkspaceSnapshot } from 'dsh-tauri/client'
import type { ReactElement } from 'react'
import type { LocaleKey, Translate } from '../locales/index.types'
import type { ScheduleForm, ScheduleKind, SchedulerOptions, TaskFormState, TaskView, Weekday } from '../types'
import { Button, Checkbox, Input, Select, Text, Textarea } from 'dsh-tauri-ui/client'
import { cloneDeep, isEmpty, map, useMount, useUnmount } from 'dsh-tauri/client'
import { useRef, useState } from 'react'
import { SCHEDULE_KINDS } from '../../shared/constants'
import { createTask, updateTask } from '../service/scheduler'
import { ModelPicker } from './model-picker'
import { sessionLabel, sessionLinkState } from './session-link'
import { TaskField } from './task-field'
import { absoluteDateTime, defaultScheduleFor, emptyTaskForm, localDateTime, taskFormInput, taskToForm } from './task-form.utils'

interface TaskFormProps {
  t: Translate
  options: SchedulerOptions
  sessions: SessionListState
  workspaces: WorkspaceSnapshot
  onClose: () => void
  onSaved: (task: TaskView) => void
  task?: TaskView
  initial?: TaskFormState
  defaultSessionId: string
  disabled?: boolean
}

const WEEKDAYS: Weekday[] = ['MO', 'TU', 'WE', 'TH', 'FR', 'SA', 'SU']
const PERMISSION_KEYS: Record<string, LocaleKey> = {
  'read-only': 'permissionReadOnly',
  'workspace-write': 'permissionWrite',
  'danger-full-access': 'permissionFullAccess',
}
const WEEKDAY_KEYS: Record<Weekday, LocaleKey> = {
  MO: 'dayMon',
  TU: 'dayTue',
  WE: 'dayWed',
  TH: 'dayThu',
  FR: 'dayFri',
  SA: 'daySat',
  SU: 'daySun',
}
const SCHEDULE_KEYS: Record<ScheduleKind, LocaleKey> = {
  once: 'scheduleOnce',
  hourly: 'scheduleHourly',
  daily: 'scheduleDaily',
  interval: 'scheduleInterval',
  workdays: 'scheduleWorkdays',
  weekly: 'scheduleWeekly',
  monthly: 'scheduleMonthly',
  custom: 'scheduleCustom',
}
const MINUTE_OPTIONS: SelectOption[] = Array.from({ length: 60 }, (_, minute) => ({ value: String(minute), label: `: ${String(minute).padStart(2, '0')}` }))

export function TaskForm({ t, options, sessions, workspaces, onClose, onSaved, task, initial, defaultSessionId, disabled }: TaskFormProps): ReactElement {
  const [expected] = useState(() => task ? cloneDeep(task) : undefined)
  const [form, setForm] = useState<TaskFormState>(() => task ? taskToForm(task) : cloneDeep(initial ?? emptyTaskForm(options, defaultSessionId)))
  const [saving, setSaving] = useState(false)
  const [error, setError] = useState('')
  const pendingRef = useRef(false)
  const aliveRef = useRef(true)
  useMount(() => {
    aliveRef.current = true
  })
  useUnmount(() => {
    aliveRef.current = false
  })
  const link = sessionLinkState(form.sessionId, sessions, workspaces)
  const blocked = disabled || saving || (form.delivery === 'this-session' && (!form.sessionId || link !== 'available'))

  function setSchedule(patch: Partial<ScheduleForm>): void {
    setForm(state => ({ ...state, schedule: { ...state.schedule, ...patch } as ScheduleForm }))
  }

  async function onSave(): Promise<void> {
    if (pendingRef.current || blocked)
      return
    pendingRef.current = true
    setSaving(true)
    setError('')
    const input = taskFormInput(form, !!expected)
    const result = expected ? await updateTask(expected.id, input, expected) : await createTask(input)
    pendingRef.current = false
    if (!aliveRef.current)
      return
    setSaving(false)
    if (!result.ok || !result.task) {
      setError(result.error ?? t('createFailed'))
      return
    }
    onSaved(result.task)
  }

  const currentTime = 'time' in form.schedule ? form.schedule.time : '09:00'
  const workspaceOptions: SelectOption[] = [{ value: '', label: t('workspaceDefault') }, ...options.workspaces.map(ws => ({ value: ws.id, label: ws.title || ws.path }))]
  if (form.workspaceId && !workspaceOptions.some(item => item.value === form.workspaceId))
    workspaceOptions.push({ value: form.workspaceId, label: form.workspaceId })
  const permissionOptions: SelectOption[] = isEmpty(options.permissions)
    ? Object.entries(PERMISSION_KEYS).map(([value, key]) => ({ value, label: t(key) }))
    : map(options.permissions, option => ({ value: option.value, label: PERMISSION_KEYS[option.value] ? t(PERMISSION_KEYS[option.value]) : option.name }))
  if (form.permission && !permissionOptions.some(option => option.value === form.permission))
    permissionOptions.unshift({ value: form.permission, label: form.permission })
  const sessionOptions: SelectOption[] = [{ value: '', label: t('session.select') }, ...sessions.ids.map(id => ({ value: id, label: sessionLabel(id, sessions).text }))]
  if (form.sessionId && !sessionOptions.some(item => item.value === form.sessionId))
    sessionOptions.push({ value: form.sessionId, label: sessionLabel(form.sessionId, sessions).text })
  const modelKey = form.provider && form.model ? `${form.provider}::${form.model}` : 'default'

  return (
    <form
      className="flex flex-col gap-[12px] min-w-0"
      onSubmit={(event) => {
        event.preventDefault()
        void onSave()
      }}
    >
      <fieldset disabled={disabled || saving} className="border-0 p-0 m-0 flex flex-col gap-[8px] min-w-0">
        <TaskField as="label" label={t('taskName')}>
          <Input type="text" maxLength={120} required value={form.name} placeholder={t('taskNamePlaceholder')} onChange={event => setForm(state => ({ ...state, name: event.target.value }))} />
        </TaskField>
        <TaskField label={t('delivery')}>
          <Select
            label={t('delivery')}
            value={form.delivery}
            options={[
              { value: 'this-session', label: t('delivery.this-session') },
              { value: 'new-session', label: t('delivery.new-session') },
            ]}
            onChange={delivery => setForm(state => ({ ...state, delivery: delivery === 'this-session' ? delivery : 'new-session', sessionId: state.sessionId || defaultSessionId }))}
          />
        </TaskField>
        {form.delivery === 'this-session'
          ? (
              <TaskField label={t('session.link')}>
                <Select label={t('session.link')} value={form.sessionId} options={sessionOptions} onChange={sessionId => setForm(state => ({ ...state, sessionId }))} />
                <Text size="sm" tone={link === 'available' ? 'secondary' : 'error'} role={link === 'available' ? undefined : 'alert'}>{t(`session.${link}`)}</Text>
                <Text size="sm" tone="tertiary">{t('delivery.inherited')}</Text>
              </TaskField>
            )
          : <Text size="sm" tone="tertiary">{t('dialogHint')}</Text>}
        <Checkbox checked={form.enabled} onChange={enabled => setForm(state => ({ ...state, enabled }))}>{t('resume')}</Checkbox>
        <TaskField label={t('schedule')}>
          <div className="flex flex-wrap gap-[8px] items-center">
            <Select label={t('schedule')} value={form.schedule.kind} options={SCHEDULE_KINDS.map(kind => ({ value: kind, label: t(SCHEDULE_KEYS[kind]) }))} onChange={value => setForm(state => ({ ...state, schedule: defaultScheduleFor(value as ScheduleKind, state.schedule.timeZone) }))} />
            {form.schedule.kind === 'once'
              ? <Input className="flex-1 min-w-0" type="datetime-local" step={1} required aria-label={t('scheduleOnce')} value={localDateTime(form.schedule.at)} onChange={event => setSchedule({ kind: 'once', at: absoluteDateTime(event.target.value) })} />
              : form.schedule.kind === 'hourly'
                ? <Select label={t('scheduleHourly')} value={String(form.schedule.minute)} options={MINUTE_OPTIONS} onChange={value => setSchedule({ kind: 'hourly', minute: Number(value) })} />
                : form.schedule.kind === 'interval'
                  ? <Input type="number" min={Number.MIN_VALUE} step="any" max={525600} required value={form.schedule.everyMinutes} aria-label={t('scheduleEveryMinutes')} onChange={event => setSchedule({ kind: 'interval', everyMinutes: Number(event.target.value) })} />
                  : (
                      <>
                        {form.schedule.kind === 'monthly'
                          ? <Input type="number" min={1} max={31} required value={form.schedule.day} aria-label={t('scheduleMonthDay')} onChange={event => setSchedule({ kind: 'monthly', day: Number(event.target.value) })} />
                          : form.schedule.kind === 'custom'
                            ? <Input type="number" min={1} max={366} required value={form.schedule.everyDays} aria-label={t('scheduleEveryDays')} onChange={event => setSchedule({ kind: 'custom', everyDays: Number(event.target.value) })} />
                            : null}
                        <Input type="time" step={60} required value={currentTime} aria-label={t('scheduleTime')} onChange={event => setSchedule({ time: event.target.value })} />
                      </>
                    )}
          </div>
          {form.schedule.kind === 'weekly'
            ? (
                <div className="flex flex-wrap gap-[8px]" role="group" aria-label={t('scheduleWeekdays')}>
                  {WEEKDAYS.map(day => (
                    <Checkbox
                      key={day}
                      checked={form.schedule.kind === 'weekly' && form.schedule.weekdays.includes(day)}
                      onChange={(checked) => {
                        setForm(state => state.schedule.kind !== 'weekly' ? state : { ...state, schedule: { ...state.schedule, weekdays: checked ? [...state.schedule.weekdays, day] : state.schedule.weekdays.filter(value => value !== day) } })
                      }}
                    >
                      {t(WEEKDAY_KEYS[day])}
                    </Checkbox>
                  ))}
                </div>
              )
            : null}
        </TaskField>
        <TaskField as="label" label={t('timeZone')}>
          <Input required value={form.schedule.timeZone ?? new Intl.DateTimeFormat().resolvedOptions().timeZone} onChange={event => setSchedule({ timeZone: event.target.value })} />
        </TaskField>
        <TaskField label={t('schedulePrompt')}>
          <div className="relative flex flex-col gap-[8px]">
            <Textarea className="min-h-[240px] border-[0.5px] border-border-l4 [font-family:inherit] text-[14px] leading-[1.55]" required maxLength={64000} value={form.prompt} placeholder={t('schedulePromptPlaceholder')} onChange={event => setForm(state => ({ ...state, prompt: event.target.value }))} />
            {form.delivery === 'new-session'
              ? (
                  <div className="flex flex-wrap gap-[8px] items-center">
                    <Select label={t('workspace')} value={form.workspaceId} variant="composerTrigger" options={workspaceOptions} onChange={workspaceId => setForm(state => ({ ...state, workspaceId }))} />
                    <Select label={t('permission')} value={form.permission} variant="composerTrigger" options={permissionOptions} onChange={permission => setForm(state => ({ ...state, permission }))} />
                    <div className="flex-1" />
                    <ModelPicker
                      t={t}
                      models={options.models}
                      failures={options.failures}
                      modelKey={modelKey}
                      reasoningEffort={form.reasoningEffort || 'none'}
                      onSelection={(nextKey, effort) => {
                        const sep = nextKey.indexOf('::')
                        setForm(state => ({ ...state, provider: sep >= 0 ? nextKey.slice(0, sep) : '', model: sep >= 0 ? nextKey.slice(sep + 2) : '', reasoningEffort: effort === 'none' ? '' : effort }))
                      }}
                    />
                  </div>
                )
              : null}
          </div>
        </TaskField>
      </fieldset>
      {error
        ? (
            <Text tone="error" role="alert">
              {error}
              <br />
              {t('task.conflictHint')}
            </Text>
          )
        : null}
      <div className="flex justify-end gap-[8px]">
        <Button variant="outline" disabled={saving} onClick={onClose}>{t('cancel')}</Button>
        <Button variant="primary" type="submit" disabled={blocked}>{saving ? t('loading') : t('save')}</Button>
      </div>
    </form>
  )
}
