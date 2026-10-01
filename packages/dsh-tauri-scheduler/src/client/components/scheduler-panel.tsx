import type { ReactElement } from 'react'
import type { Translate } from '../locales/index.types'
import type { RunView, TaskFormState, TaskView } from '../types'
import { Button, Card, Check, CommentPlus, Icon, Input, Magnifier, Plus, SegmentedControl, Text } from 'dsh-tauri-ui/client'
import { filter, includes, isEmpty, lowerCase, omit, useEventListener, useStore } from 'dsh-tauri/client'
import { useCallback, useEffect, useRef, useState } from 'react'
import { REFRESH_INTERVAL_MS } from '../constants'
import { deleteRun, loadScheduler } from '../service/scheduler'
import { store } from '../store'
import { Recommendations } from './recommendations'
import { RunsTab } from './runs-tab'
import { countUnreadRuns, describeSchedule, formatRelative } from './schedule.utils'
import { TaskCard } from './task-card'
import { TaskCreateDialog } from './task-create-dialog'

interface SchedulerPanelProps {
  t: Translate
  onViaChat: () => void
  onOpenSession: (sessionId: string) => 'opened' | 'archived' | 'unavailable'
}

type DialogState = { taskId?: string, initial?: TaskFormState } | null

/** 由任务视图构造编辑表单（去掉 timeZone 等宿主字段）。 */
function taskToForm(task: TaskView): TaskFormState {
  return {
    name: task.name,
    schedule: omit(task.schedule, 'timeZone') as TaskFormState['schedule'],
    prompt: task.prompt,
    workspaceId: task.workspaceId ?? '',
    permission: task.permission || 'read-only',
    provider: task.provider ?? '',
    model: task.model ?? '',
    // 编辑旧任务（无显式推理等级）时不预填。
    reasoningEffort: task.reasoningEffort || '',
  }
}

export function SchedulerPanel({ t, onViaChat, onOpenSession }: SchedulerPanelProps): ReactElement {
  const state = useStore(store.scheduler)
  const [tab, setTab] = useState<'tasks' | 'runs'>('tasks')
  const [search, setSearch] = useState('')
  const [dialog, setDialog] = useState<DialogState>(null)
  const [openError, setOpenError] = useState('')
  // 相对「下次运行」以刷新时刻为基准，避免每次渲染抖动。
  const [now, setNow] = useState(() => Date.now())

  // 首帧载入：面板打开时拉一次全量（含对话框选项）。轮询只做增量刷新。
  useEffect(() => {
    void loadScheduler(true)
  }, [])

  // 拉取由注册层轮询（面板关闭时也要更新未读角标），这里只推进相对时间基准。
  useEffect(() => {
    const timer = setInterval(() => setNow(Date.now()), REFRESH_INTERVAL_MS)
    return () => clearInterval(timer)
  }, [])

  // 回到前台 / 重新聚焦时立刻补一次刷新（离开期间轮询可能被浏览器节流）
  const refreshOnResume = useCallback((): void => {
    if (document.visibilityState === 'visible')
      void loadScheduler(false)
  }, [])
  const documentRef = useRef<Document | null | undefined>(typeof document === 'undefined' ? undefined : document)
  const windowRef = useRef<Window | null | undefined>(typeof window === 'undefined' ? undefined : window)
  useEventListener(documentRef, 'visibilitychange', refreshOnResume)
  useEventListener(windowRef, 'focus', refreshOnResume)

  const filtered = filter(state.tasks, task =>
    isEmpty(search) || includes(lowerCase(`${task.name} ${task.prompt}`), lowerCase(search)))
  const filteredRuns = filter(state.runs, run =>
    isEmpty(search) || includes(lowerCase(run.taskName), lowerCase(search)))
  const unread = countUnreadRuns(state.runs, state.readAt, state.readIds)

  function onOpenRun(run: RunView): void {
    // 点开即视为看过：归档或不可用也不留下永远消不掉的未读。
    store.scheduler.markRunRead(run.id)
    if (run.sessionId === undefined) {
      setOpenError(t('openRunFailed'))
      return
    }
    const outcome = onOpenSession(run.sessionId)
    if (outcome === 'archived') {
      setOpenError(t('runSessionArchived'))
      return
    }
    if (outcome === 'unavailable') {
      setOpenError(t('openRunFailed'))
      return
    }
    setOpenError('')
  }

  return (
    <div className="box-border font-sans text-primary text-[13px] leading-[1.5]">
      <header className="flex justify-between items-start gap-[16px] mb-[12px]">
        <div className="min-w-0">
          <h1 className="m-0 text-[20px] leading-[28px] font-medium">{t('scheduler')}</h1>
          <p className="mt-[4px] mx-0 mb-0 text-secondary text-[13px] leading-[20px]">{t('subtitle')}</p>
        </div>
        <div className="flex justify-end items-center gap-[16px]">
          <Button style={{ flexShrink: 0 }} variant="addGhost" icon={<Icon as={CommentPlus} />} onClick={onViaChat}>
            {t('viaChat')}
          </Button>
          <Button style={{ flexShrink: 0 }} variant="add" icon={<Icon as={Plus} size={13} />} onClick={() => setDialog({})}>
            {t('createManual')}
          </Button>
        </div>
      </header>

      <div className="flex justify-between items-center mb-[12px]">
        <Input
          className="flex-[0_1_280px] min-w-0 max-w-[280px] max-[680px]:max-w-[160px]"
          type="search"
          icon={<Icon as={Magnifier} />}
          aria-label={t('searchPlaceholder')}
          placeholder={t('searchPlaceholder')}
          value={search}
          onChange={event => setSearch(event.target.value)}
        />
        {tab === 'runs' && unread > 0
          ? (
              <Button variant="ghost" size="sm" icon={<Icon as={Check} />} onClick={() => store.scheduler.markAllRunsRead()}>
                {t('markAllRead')}
              </Button>
            )
          : null}
      </div>

      <div className="flex mt-[4px] mb-[14px]">
        <SegmentedControl
          id="dshp-scheduler-tabs"
          label={t('scheduler')}
          value={tab}
          options={[
            { value: 'tasks', label: t('tasksTab') },
            { value: 'runs', label: t('runsTab') },
          ]}
          onChange={next => setTab(next === 'runs' ? 'runs' : 'tasks')}
        />
      </div>

      {state.error ? <Text tone="error" role="alert">{state.error}</Text> : null}
      {openError ? <Text tone="error" role="alert">{openError}</Text> : null}

      {tab === 'tasks'
        ? (
            <>
              {filtered.length === 0
                ? <Text size="sm" tone="tertiary" className="py-[48px] text-center">{search ? t('noMatch') : t('emptyTasks')}</Text>
                : (
                    <Card.List className="gap-[8px]">
                      {filtered.map(task => (
                        <TaskCard
                          key={task.id}
                          task={task}
                          t={t}
                          describe={describeSchedule(task.schedule, t)}
                          nextRun={task.enabled ? formatRelative(task.nextRunAt, now, t) : undefined}
                          paused={!task.enabled}
                          onEdit={task => setDialog({ taskId: task.id, initial: taskToForm(task) })}
                        />
                      ))}
                    </Card.List>
                  )}
              <Recommendations t={t} tasks={state.tasks} />
            </>
          )
        : (
            <RunsTab
              t={t}
              runs={filteredRuns}
              readAt={state.readAt}
              readIds={state.readIds}
              emptyLabel={search ? t('noMatchRuns') : t('emptyRuns')}
              onOpen={onOpenRun}
              onDelete={id => void deleteRun(id)}
            />
          )}

      {dialog
        ? <TaskCreateDialog t={t} options={state.options} initial={dialog.initial} taskId={dialog.taskId} onClose={() => setDialog(null)} />
        : null}
    </div>
  )
}
