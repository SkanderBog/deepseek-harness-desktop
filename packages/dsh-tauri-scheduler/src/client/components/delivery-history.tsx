import type { SessionListState, WorkspaceSnapshot } from 'dsh-tauri/client'
import type { ReactElement } from 'react'
import type { Translate } from '../locales/index.types'
import type { HistoryPage } from '../types'
import { Button, Card, Text } from 'dsh-tauri-ui/client'
import { useMount, useUnmount, useWatchImmediate } from 'dsh-tauri/client'
import { useRef, useState } from 'react'
import { loadTaskHistory } from '../service/scheduler'
import { sessionLabel, sessionLinkState } from './session-link'

interface DeliveryHistoryProps {
  taskId: string
  refreshKey?: string
  timeZone?: string
  t: Translate
  sessions: SessionListState
  workspaces: WorkspaceSnapshot
  onOpenSession: (id: string) => void
}

export function DeliveryHistory({ taskId, refreshKey, timeZone, t, sessions, workspaces, onOpenSession }: DeliveryHistoryProps): ReactElement {
  const [page, setPage] = useState<HistoryPage>()
  const [loading, setLoading] = useState(false)
  const [error, setError] = useState('')
  const requestRef = useRef({ epoch: 0, pending: false, alive: true })
  useMount(() => {
    requestRef.current.alive = true
  })
  useUnmount(() => {
    requestRef.current.alive = false
    requestRef.current.epoch++
  })

  async function load(before?: string): Promise<void> {
    if (requestRef.current.pending || !requestRef.current.alive)
      return
    requestRef.current.pending = true
    const epoch = requestRef.current.epoch
    setLoading(true)
    setError('')
    try {
      const next = await loadTaskHistory(taskId, 20, before)
      if (!requestRef.current.alive || epoch !== requestRef.current.epoch)
        return
      setPage((previous) => {
        if (!before || !previous)
          return next
        const seen = new Set(previous.records.map(record => record.id))
        return { ...next, records: [...previous.records, ...next.records.filter(record => !seen.has(record.id))] }
      })
    }
    catch (failure) {
      if (!requestRef.current.alive || epoch !== requestRef.current.epoch)
        return
      setError(failure instanceof Error ? failure.message : String(failure))
      if (failure && typeof failure === 'object' && 'code' in failure && failure.code === 'delivery_cursor_not_found') {
        requestRef.current.pending = false
        refresh()
      }
    }
    finally {
      if (requestRef.current.alive && epoch === requestRef.current.epoch) {
        requestRef.current.pending = false
        setLoading(false)
      }
    }
  }

  function refresh(): void {
    requestRef.current.epoch++
    requestRef.current.pending = false
    setPage(undefined)
    void load()
  }
  useWatchImmediate([taskId, refreshKey], refresh)

  function format(at: string): string {
    try {
      return new Intl.DateTimeFormat(undefined, { dateStyle: 'medium', timeStyle: 'medium', timeZone }).format(new Date(at))
    }
    catch {
      return at
    }
  }

  return (
    <section className="flex flex-col gap-[8px] min-w-0" aria-label={t('history.title')}>
      <div className="flex justify-between items-center">
        <h3 className="text-[14px] font-medium m-0">{t('history.title')}</h3>
        <Button size="sm" variant="ghost" onClick={refresh}>{t('refresh')}</Button>
      </div>
      {error ? <Text tone="error" role="alert">{error}</Text> : null}
      {page?.records.length === 0 ? <Text tone="tertiary" size="sm">{t('history.empty')}</Text> : null}
      <Card.List className="gap-[8px]">
        {page?.records.map((record) => {
          const state = record.sessionId ? sessionLinkState(record.sessionId, sessions, workspaces) : 'unavailable'
          return (
            <Card key={record.id} className="p-[12px] flex flex-col gap-[6px] min-w-0">
              <Text size="sm">
                {t(record.delivery === 'this-session' ? 'delivery.this-session' : 'delivery.new-session')}
                {' · '}
                {t(record.trigger === 'manual' ? 'triggerManual' : 'triggerSchedule')}
              </Text>
              <Text size="sm" tone="secondary">
                {t('history.scheduled')}
                :
                {' '}
                {format(record.delivery === 'this-session' ? record.scheduledAt : record.scheduledFor)}
              </Text>
              {record.delivery === 'this-session'
                ? (
                    <Text size="sm" tone="secondary">
                      {t('history.delivered')}
                      :
                      {' '}
                      {format(record.deliveredAt)}
                    </Text>
                  )
                : (
                    <>
                      <Text size="sm">{t(record.status)}</Text>
                      <Text size="sm" tone="secondary">
                        {t('startedAt')}
                        :
                        {' '}
                        {format(record.startedAt)}
                      </Text>
                      {record.finishedAt
                        ? (
                            <Text size="sm" tone="secondary">
                              {t('history.finished')}
                              :
                              {' '}
                              {format(record.finishedAt)}
                            </Text>
                          )
                        : null}
                      {record.error ? <Text tone="error">{record.error}</Text> : null}
                    </>
                  )}
              {record.prompt ? <Text size="sm" className="whitespace-pre-wrap wrap-anywhere">{record.prompt}</Text> : null}
              {record.sessionId
                ? (
                    <Button size="sm" variant="ghost" disabled={state !== 'available'} title={t(`session.${state}`)} onClick={() => record.sessionId && onOpenSession(record.sessionId)}>
                      {t('history.open')}
                      :
                      {' '}
                      {sessionLabel(record.sessionId, sessions).text}
                    </Button>
                  )
                : null}
            </Card>
          )
        })}
      </Card.List>
      {loading ? <Text size="sm" tone="secondary">{t('loading')}</Text> : null}
      {page?.nextBefore ? <Button variant="outline" disabled={loading} onClick={() => void load(page.nextBefore)}>{t('history.more')}</Button> : null}
      {page?.earlierRecordsUnavailable ? <Text size="sm" tone="tertiary">{t('history.unavailable')}</Text> : null}
      {page?.earlierRecordsPruned ? <Text size="sm" tone="tertiary">{t('history.pruned', page.retention)}</Text> : null}
    </section>
  )
}
