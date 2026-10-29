import { CircleExclamation, ShieldCheck } from '@gravity-ui/icons'
import { Button, Chip, Description, Spinner } from '@heroui/react'
import { useWatch } from '@reause/core'
import { invoke } from '@tauri-apps/api/core'
import { useState } from 'react'
import { useTranslation } from 'react-i18next'
import { If } from 'react-if-lite'
import { useStore } from 'valtio-define'
import { store } from '@/store'
import { silence } from '@/utils/silence'

const REASON_KEYS: Record<string, { title: string, detail: string }> = {
  duplicate_route: { title: 'recovery.reason.duplicate_route.title', detail: 'recovery.reason.duplicate_route.detail' },
  duplicate_loader_entry: { title: 'recovery.reason.duplicate_loader_entry.title', detail: 'recovery.reason.duplicate_loader_entry.detail' },
  cannot_resolve_bundle: { title: 'recovery.reason.cannot_resolve_bundle.title', detail: 'recovery.reason.cannot_resolve_bundle.detail' },
  no_dsh_bundle: { title: 'recovery.reason.no_dsh_bundle.title', detail: 'recovery.reason.no_dsh_bundle.detail' },
  slot_conflict: { title: 'recovery.reason.slot_conflict.title', detail: 'recovery.reason.slot_conflict.detail' },
  load_failed: { title: 'recovery.reason.load_failed.title', detail: 'recovery.reason.load_failed.detail' },
  runtime: { title: 'recovery.reason.runtime.title', detail: 'recovery.reason.runtime.detail' },
  unknown: { title: 'recovery.reason.unknown.title', detail: 'recovery.reason.unknown.detail' },
}

export function Recovery({ fullScreen = false }: { fullScreen?: boolean }) {
  const { t } = useTranslation()
  const { recovery, exhausted } = useStore(store.recovery)

  // （issue #303）。无快照的插件还原会 SNAPSHOT_NOT_FOUND，故必须按 id 过滤，
  const [restorableIds, setRestorableIds] = useState<string[]>([])
  useWatch([recovery.required, recovery.info], () => {
    if (!recovery.required || !recovery.info) {
      return
    }
    Promise.all(recovery.info.plugins.map(async (id) => {
      try {
        const r = await invoke<{ exists: boolean }>('get_plugin_backup', { id })
        return r.exists ? id : null
      }
      catch (e) {
        silence(e, 'plugin recovery: backup check failed, will retry')
        return null
      }
    }))
      .then(results => setRestorableIds(results.filter((id): id is string => id !== null)))
      .catch((err) => {
        console.error('[Recovery] check snapshot failed:', err)
        setRestorableIds([])
      })
  }, { immediate: true })

  if (!recovery.required || !recovery.info) {
    return null
  }

  const info = recovery.info
  const multiple = info.plugins.length > 1
  const reasonKeys = REASON_KEYS[info.reason] ?? REASON_KEYS.unknown
  const heading = multiple
    ? t('recovery.heading_many', { count: info.plugins.length })
    : t('recovery.heading_one')
  const primaryLabel = multiple
    ? t('recovery.remove_many', { count: info.plugins.length })
    : t('recovery.remove_one')
  const restoreLabel = restorableIds.length > 1
    ? t('recovery.restore_many', { count: restorableIds.length })
    : t('recovery.restore_one')
  const reasonDetail = info.detail
    ? t(reasonKeys.detail, { detail: info.detail })
    : t(reasonKeys.detail)

  return (
    <div
      className={fullScreen
        ? 'flex h-full w-full items-center justify-center overflow-auto bg-startup p-6'
        : 'fixed inset-0 z-50 flex items-center justify-center bg-backdrop p-4'}
    >
      <div className={`w-full ${fullScreen ? 'max-w-[640px]' : 'max-w-[560px]'}`}>
        <div className={fullScreen
          ? 'rounded-xl border border-line bg-panel2/40 p-6'
          : 'rounded-xl border border-line bg-panel p-6 shadow-lg'}
        >
          <div className="mb-4 flex items-center gap-3">
            <div className="flex size-12 shrink-0 items-center justify-center rounded-full bg-danger/10 text-danger">
              <CircleExclamation className="size-6" />
            </div>
            <div className="min-w-0">
              <p className="m-0 text-base font-semibold text-ink">{heading}</p>
              <p className="m-0 text-xs text-muted">{t(reasonKeys.title)}</p>
            </div>
          </div>

          <Description className="mb-3 text-sm text-ink">
            {reasonDetail}
          </Description>

          <div className="mb-3 space-y-1.5">
            {info.plugins.map(pkg => (
              <div key={pkg} className="flex items-center justify-between gap-2 rounded-md bg-black/5 px-3 py-2">
                <code className="min-w-0 truncate font-mono text-sm text-ink">{pkg}</code>
                <Chip className="shrink-0 text-xs" variant="primary" color="danger" size="sm">
                  {t('recovery.offender')}
                </Chip>
              </div>
            ))}
          </div>

          <If cond={exhausted}>
            <p className="m-0 mb-3 text-xs text-warning">{t('recovery.summary_exhausted')}</p>
          </If>

          <p className="m-0 mb-4 text-xs text-load-muted">{t('recovery.safety_note')}</p>

          <div className="mb-4 max-h-48 overflow-auto rounded-md bg-black/5 p-3">
            <pre className="m-0 whitespace-pre-wrap break-all font-mono text-[11px] text-muted">
              {info.rawError || '—'}
            </pre>
          </div>

          <div className="flex flex-wrap items-center gap-2">
            <If cond={restorableIds.length > 0}>
              <Button
                variant="primary"
                onPress={() => store.recovery.restoreAndRedetect(restorableIds)}
              >
                <span className="flex items-center gap-1">
                  <If cond={recovery.busy} then={<Spinner size="sm" color="current" />} />
                  {recovery.busy ? t('recovery.restoring') : restoreLabel}
                </span>
              </Button>
            </If>
            <Button
              variant="danger"
              onPress={() => store.recovery.recoverAndRedetect(info.plugins)}
            >
              <span className="flex items-center gap-1">
                <If cond={recovery.busy} then={<Spinner size="sm" color="current" />} />
                {recovery.busy ? t('recovery.removing') : primaryLabel}
              </span>
            </Button>
            <Button variant="tertiary" onPress={() => store.harness.restart()}>
              {t('recovery.restart')}
            </Button>
            <Button variant="ghost" onPress={() => store.harness.enterSafeMode()}>
              <span className="flex items-center gap-1">
                <ShieldCheck className="size-4" />
                {t('buttons.safe_mode')}
              </span>
            </Button>
            <Button variant="ghost" onPress={() => store.recovery.dismissRecovery()}>
              {t('recovery.dismiss')}
            </Button>
          </div>
        </div>
      </div>
    </div>
  )
}
