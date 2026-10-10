import type { ReactElement } from 'react'
import type { Translate } from '../locales/index.types'
import type { ScheduleTurnCardInjected } from './schedule-turn-card'
import { Action, Button, Clock, Icon, Menu, TrashBin } from 'dsh-tauri-ui/client'
import { useStore, useWatchImmediate } from 'dsh-tauri/client'
import { useState } from 'react'
import { requestTaskDeletion } from '../service/deletion'
import { activeSessionTasks, isTaskOverdue, orderSessionTasks } from '../service/session-schedule'
import { store } from '../store'
import { ScheduleTaskTiming } from './schedule-task-timing'

export interface ScheduleCatalogActionInjected extends ScheduleTurnCardInjected {
  readonly sessionId: string
}

export function ScheduleCatalogAction({ sessionId, openTaskDetail, t }: ScheduleCatalogActionInjected & { t: Translate }): ReactElement | null {
  const catalog = useStore(store.scheduler)
  const [open, setOpen] = useState(false)
  const [now, setNow] = useState(() => Date.now())
  const tasks = orderSessionTasks(activeSessionTasks(catalog.tasks, sessionId), now)
  useWatchImmediate([sessionId, tasks.length], () => setOpen(false))
  if (tasks.length === 0)
    return null
  const multiple = tasks.length > 1
  const label = t(multiple ? 'ambient.trigger.other' : 'ambient.trigger.one', { count: tasks.length })
  return (
    <Menu
      open={multiple && open}
      onClose={() => setOpen(false)}
      align="end"
      portal
      autoFocus
      listClassName="min-w-[280px] max-w-[420px]"
      anchor={(
        <Action
          variant="toolbar"
          className="relative"
          icon={<Icon as={Clock} size={18} aria-hidden="true" />}
          aria-label={label}
          title={label}
          aria-haspopup={multiple ? 'menu' : undefined}
          aria-expanded={multiple ? open : undefined}
          onClick={() => {
            if (!multiple) {
              openTaskDetail(tasks[0]!.id)
              return
            }
            setNow(Date.now())
            setOpen(value => !value)
          }}
        >
          {multiple && <span aria-hidden="true" className="absolute right-[-3px] top-[-3px] text-[10px] leading-[12px]">{tasks.length}</span>}
        </Action>
      )}
    >
      <div aria-label={t('ambient.list.aria')}>
        {tasks.map(task => (
          <div key={task.id} data-overdue={isTaskOverdue(task, now) || undefined} className="flex min-w-0 items-center gap-[4px] rounded-sm px-[4px] hover:bg-hover">
            <Button
              variant="link"
              role="menuitem"
              className="flex-1 no-underline hover:no-underline"
              aria-label={t('ambient.open', { name: task.name })}
              onClick={() => {
                setOpen(false)
                openTaskDetail(task.id)
              }}
            >
              <span className="block min-w-0 py-[6px]">
                <span className="block truncate text-[13px] leading-[18px]">{task.name}</span>
                <ScheduleTaskTiming task={task} now={now} t={t} />
              </span>
            </Button>
            <Action
              variant="round"
              role="menuitem"
              aria-label={`${t('delete')} ${task.name}`}
              title={t('delete')}
              icon={<Icon as={TrashBin} size={14} aria-hidden="true" />}
              onClick={() => {
                setOpen(false)
                requestTaskDeletion(task)
              }}
            />
          </div>
        ))}
      </div>
    </Menu>
  )
}
