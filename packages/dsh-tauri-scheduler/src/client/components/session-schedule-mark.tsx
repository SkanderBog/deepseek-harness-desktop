import type { ReactElement } from 'react'
import type { Translate } from '../locales/index.types'
import { Clock, Icon } from 'dsh-tauri-ui/client'
import { useStore } from 'dsh-tauri/client'
import { activeSessionTasks } from '../service/session-schedule'
import { store } from '../store'

export function SessionScheduleMark({ sessionId, t }: { sessionId: string, t: Translate }): ReactElement | null {
  const catalog = useStore(store.scheduler)
  const tasks = activeSessionTasks(catalog.tasks, sessionId)
  if (tasks.length === 0)
    return null
  return (
    <span
      data-session-schedule-mark=""
      className="inline-flex h-[16px] w-[16px] shrink-0 items-center justify-center text-tertiary"
      onClick={event => event.stopPropagation()}
    >
      <Icon as={Clock} size={14} aria-hidden="true" />
      <span className="sr-only">{t('ambient.mark.aria', { count: tasks.length })}</span>
    </span>
  )
}
