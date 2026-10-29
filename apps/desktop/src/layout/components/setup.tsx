import type { IconComponent } from './loadable'
import type { SetupStatus } from '@/store/modules/harness'
import { ArrowRightFromSquare, CircleCheck, CircleExclamation, CircleInfo, Copy, Magnifier, Rocket, ShieldCheck } from '@gravity-ui/icons'
import { invoke } from '@tauri-apps/api/core'
import { useTranslation } from 'react-i18next'
import { If, Then } from 'react-if-lite'
import { useStore } from 'valtio-define'
import { button } from '@/components/primitives'
import { store } from '@/store'
import { containsPatchEntryUnresolved } from '@/store/modules/harness'
import { writeClipboardText } from '@/utils/clipboard'
import { toast } from '@/utils/toast'
import { Loadable } from './loadable'

const STATUS_ICONS: Record<SetupStatus, IconComponent> = {
  checking: Magnifier,
  installing: ArrowRightFromSquare,
  starting: Rocket,
  preinstall: CircleInfo,
  ready: CircleCheck,
  error: CircleExclamation,
}

async function copyLogsHandler(t: (key: string) => string) {
  let logs: string
  try {
    logs = await invoke<string>('read_run_logs')
  }
  catch (err) {
    console.error('[Setup] failed to read logs:', err)
    toast(t('messages.logs_read_failed'), { variant: 'danger' })
    return
  }
  await writeClipboardText(logs, t('messages.logs_copied')).catch((err) => {
    console.error('[Setup] failed to copy logs:', err)
  })
}

export function Setup() {
  const { t } = useTranslation()
  const {
    status,
    installer,
    errorMsg,
    errorLogs,
    pluginConflictHint,
    inotifyLimitHint,
    patchLayerHint,
    heapOomHint,
  } = useStore(store.harness)
  const error = status === 'error'
  const installing = status === 'installing'
  const heading = error ? t('status.error') : (installer.title || t('status.installing'))
  const StatusIcon = STATUS_ICONS[status]
  const logs = installing ? installer.logs : (error && errorLogs.length > 0 ? errorLogs : undefined)
  const hint = error ? (patchLayerHint || pluginConflictHint || inotifyLimitHint || heapOomHint) : undefined
  const patchEntriesUnresolved = error && containsPatchEntryUnresolved(errorMsg)
  const patchLayerBroken = error && patchLayerHint !== '' && !patchEntriesUnresolved

  return (
    <Loadable
      icon={StatusIcon}
      title={heading}
      subtitle={error ? undefined : installer.detail || t('status.installing')}
      percentage={installing ? installer.percentage : undefined}
      logs={logs}
      errorMsg={error ? errorMsg : undefined}
      testId={error ? 'dsh-setup-error' : undefined}
    >
      {hint && (
        <p className="m-0 text-xs leading-[18px] break-all text-load-muted">{hint}</p>
      )}
      <If cond={error}>
        <Then>
          <div className="flex flex-wrap items-center justify-center gap-2">
            <button
              className={button({ tone: 'primary', size: 'sm' })}
              onClick={() => {
                void store.harness.boot()
              }}
            >
              {t('app.retry')}
            </button>
            <If cond={patchLayerBroken}>
              <button
                className={button({ tone: 'primary', size: 'sm' })}
                onClick={() => {
                  void store.harness.quarantineBrokenPatchLayers()
                }}
              >
                {t('buttons.quarantine_patch')}
              </button>
            </If>
            <If cond={patchEntriesUnresolved}>
              <button
                className={button({ tone: 'primary', size: 'sm' })}
                onClick={() => {
                  void store.harness.stripUnresolvedPatchEntries()
                }}
              >
                {t('buttons.strip_patch_entries')}
              </button>
            </If>
            <button
              className={button({ tone: 'ghost', size: 'sm' })}
              onClick={() => copyLogsHandler(t)}
            >
              <Copy className="size-4" />
              {t('buttons.copy_logs')}
            </button>
            <button
              className={button({ tone: 'primary', size: 'sm' })}
              onClick={() => {
                void store.harness.enterSafeMode()
              }}
            >
              <ShieldCheck className="size-4" />
              {t('buttons.safe_mode')}
            </button>
          </div>
          <p className="m-0 text-xs leading-[18px] break-all text-load-muted">
            {t('hints.safe_mode')}
          </p>
        </Then>
      </If>
    </Loadable>
  )
}
