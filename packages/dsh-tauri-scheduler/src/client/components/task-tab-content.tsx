import type { ReactElement } from 'react'
import type { TaskView } from '../types'
import type { TaskTabInfo, TaskTabNavigation } from '../types/task-tab'
import type { TaskTabProps } from './task-tab'
import { Button, Text } from 'dsh-tauri-ui/client'
import { cloneDeep, useMount, useStore, useWatchImmediate } from 'dsh-tauri/client'
import { useState } from 'react'
import { SCHEDULE_TASK_KIND } from '../constants'
import { useTaskTabTarget } from '../hooks/use-task-tab-target'
import { requestTaskDeletion } from '../service/deletion'
import { loadOptions, loadScheduler } from '../service/scheduler'
import { store } from '../store'
import { DeliveryHistory } from './delivery-history'
import { Recommendations } from './recommendations'
import { sessionLabel, sessionLinkState } from './session-link'
import { TaskForm } from './task-form'

export function TaskTabContent(props: TaskTabProps & { tab: TaskTabInfo['tab'] }): ReactElement {
  const { sessionId, tab, taskBindings, t, openHistorySession } = props
  const { navigated, navigation, recovered, params } = useTaskTabTarget(sessionId, tab, taskBindings)
  const state = useStore(store.scheduler)
  const deletion = useStore(store.deletion)
  const sessions = props.useSessions(value => value)
  const workspaces = props.useWorkspaces(value => value)
  const [baseline] = useState(() => store.scheduler.loadToken)
  const [feedbackBaseline] = useState(() => store.deletion.feedbackSeq)
  const [optionsReady, setOptionsReady] = useState(false)
  const [openError, setOpenError] = useState('')
  const record = params && !params.draft ? state.tasks.find(task => task.id === params.id) : undefined
  const [retained, setRetained] = useState<TaskView | undefined>(() => record ? cloneDeep(record) : undefined)
  const answered = state.successfulReadToken > baseline
  const task = retained ?? record
  const inactive = record?.status === 'inactive' || task?.status === 'inactive'

  useMount(() => {
    void loadScheduler()
    void loadOptions().then(() => setOptionsReady(true)).catch(error => setOpenError(error instanceof Error ? error.message : String(error)))
  })
  useWatchImmediate(record, () => {
    if (!retained && record)
      setRetained(cloneDeep(record))
  })
  useWatchImmediate([navigation?.draft, navigation && !navigation.draft ? navigation.id : undefined, navigation?.sessionId], () => {
    if (navigation?.draft)
      taskBindings.forget(sessionId, tab)
    else if (navigation)
      taskBindings.write(sessionId, tab, navigation)
  })
  useWatchImmediate([answered, record !== undefined], () => {
    if (!navigated && recovered && answered && !record)
      taskBindings.forget(sessionId, tab)
  })
  useWatchImmediate(deletion.feedbackSeq, () => {
    if (task && deletion.feedback?.seq && deletion.feedback.seq > feedbackBaseline && deletion.feedback.kind === 'deleted' && deletion.feedback.taskId === task.id)
      tab.actions.close()
  })

  function navigate(target: TaskTabNavigation): void {
    tab.actions.openTab(SCHEDULE_TASK_KIND, { params: target })
  }
  async function openSession(id: string): Promise<void> {
    const result = await openHistorySession(id)
    setOpenError(result.ok ? '' : result.error ?? t('openRunFailed'))
  }

  if (!params || (!params.draft && !task)) {
    return (
      <div className="p-[16px] flex flex-col gap-[8px]">
        <Text tone={state.error ? 'error' : 'secondary'} role="status">{answered ? t(params ? 'task.missing' : 'task.unbound') : state.error || t('loading')}</Text>
        <Button variant="outline" onClick={() => void loadScheduler()}>{t('refresh')}</Button>
      </div>
    )
  }

  const linkedId = task?.delivery === 'this-session' ? task.sessionId : undefined
  const link = linkedId ? sessionLinkState(linkedId, sessions, workspaces) : undefined
  return (
    <div className="p-[16px] flex flex-col gap-[16px] min-w-0 overflow-auto h-full">
      <header className="flex items-start justify-between gap-[8px]">
        <h2 className="m-0 text-[16px] font-medium">{params.draft ? t('createDialogTitle') : task?.name}</h2>
        {task ? <Button size="sm" variant="ghost" onClick={() => requestTaskDeletion(task)}>{t('delete')}</Button> : null}
      </header>
      {state.error ? <Text tone="error" role="alert">{state.error}</Text> : null}
      {openError ? <Text tone="error" role="alert">{openError}</Text> : null}
      {inactive ? <Text tone="secondary">{t('task.inactive')}</Text> : null}
      {task && answered && !record ? <Text tone="error" role="status">{t('task.missing')}</Text> : null}
      {linkedId && link
        ? (
            <Button size="sm" variant="ghost" disabled={link !== 'available'} title={t(`session.${link}`)} onClick={() => void openSession(linkedId)}>
              {t('session.link')}
              :
              {' '}
              {sessionLabel(linkedId, sessions).text}
            </Button>
          )
        : null}
      {optionsReady
        ? <TaskForm t={t} task={task} initial={params.draft ? params.initial : undefined} options={state.options} sessions={sessions} workspaces={workspaces} defaultSessionId={params.sessionId ?? sessionId} disabled={!!task && (inactive || (answered && !record))} onClose={tab.actions.close} onSaved={saved => navigate({ id: saved.id, sessionId: saved.sessionId })} />
        : <Text tone="secondary">{t('loading')}</Text>}
      {params.draft
        ? <Recommendations t={t} tasks={state.tasks} onSelect={initial => navigate({ draft: true, sessionId, initial })} />
        : task ? <DeliveryHistory taskId={task.id} refreshKey={record?.lastRunAt} timeZone={task.schedule.timeZone} t={t} sessions={sessions} workspaces={workspaces} onOpenSession={id => void openSession(id)} /> : null}
    </div>
  )
}
