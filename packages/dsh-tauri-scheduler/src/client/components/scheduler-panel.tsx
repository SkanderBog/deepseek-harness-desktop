import type { ReactElement } from 'react'
import type { Translate } from '../locales/index.types'
import { Button, Card, CommentPlus, Icon, Input, Magnifier, Plus, SegmentedControl, Text } from 'dsh-tauri-ui/client'
import { filter, includes, isEmpty, lowerCase, useStore } from 'dsh-tauri/client'
import { useState } from 'react'
import { store } from '../store'
import { Recommendations } from './recommendations'
import { describeSchedule, formatRelative } from './schedule.utils'
import { TaskCard } from './task-card'

interface SchedulerPanelProps {
  t: Translate
  onViaChat: () => void
}

export function SchedulerPanel({ t, onViaChat }: SchedulerPanelProps): ReactElement {
  const state = useStore(store.scheduler)
  const navigation = useStore(store.navigation)
  const [search, setSearch] = useState('')
  const filtered = filter(state.tasks, task =>
    isEmpty(search) || includes(lowerCase(`${task.name} ${task.prompt}`), lowerCase(search)))

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
          <Button style={{ flexShrink: 0 }} variant="add" icon={<Icon as={Plus} size={13} />} onClick={() => store.navigation.request({ draft: true })}>
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
      </div>

      <div className="flex mt-[4px] mb-[14px]">
        <SegmentedControl id="dshp-scheduler-tabs" label={t('scheduler')} value="tasks" options={[{ value: 'tasks', label: t('tasksTab') }]} onChange={() => {}} />
      </div>

      {state.error ? <Text tone="error" role="alert">{state.error}</Text> : null}
      {navigation.error ? <Text tone="error" role="alert">{navigation.error}</Text> : null}
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
                  nextRun={task.enabled && task.status === 'active' ? formatRelative(task.nextRunAt, state.refreshedAt, t) : undefined}
                  paused={!task.enabled}
                  onEdit={task => store.navigation.request({ id: task.id, sessionId: task.sessionId })}
                />
              ))}
            </Card.List>
          )}
      <Recommendations t={t} tasks={state.tasks} />
    </div>
  )
}
