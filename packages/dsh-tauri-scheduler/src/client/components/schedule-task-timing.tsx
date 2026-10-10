import type { ReactElement } from 'react'
import type { Translate } from '../locales/index.types'
import type { TaskView } from '../types'
import { isTaskOverdue } from '../service/session-schedule'
import { describeSchedule, formatLocalTime, formatRelative } from './schedule.utils'

export function ScheduleTaskTiming({ task, now, t }: { task: TaskView, now: number, t: Translate }): ReactElement {
  const overdue = isTaskOverdue(task, now)
  const absolute = formatLocalTime(task.nextRunAt)
  return (
    <span className="block text-[11px] leading-[16px] text-tertiary">
      {describeSchedule(task.schedule, t)}
      {' · '}
      {absolute === undefined
        ? t('waiting')
        : (
            <>
              <time dateTime={task.nextRunAt}>{absolute}</time>
              {' '}
              <span className={overdue ? 'text-error' : undefined}>
                {overdue ? t('ambient.overdue') : formatRelative(task.nextRunAt, now, t)}
              </span>
            </>
          )}
    </span>
  )
}
