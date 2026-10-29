import type { PropsWithOverlays } from '@overlastic/react'
import type { InstallProgress } from '@/store/modules/harness'
import type { HarnessCore } from '@/types'
import { AlertDialog, Button, Spinner } from '@heroui/react'
import { useDisclosure } from '@overlastic/react'
import { useEffect, useState } from 'react'
import { useTranslation } from 'react-i18next'
import { If } from 'react-if-lite'
import { Panel } from '@/components/panel'
import { useListen } from '@/hooks/use-listen'

export interface DownloadCoreDialogProps extends PropsWithOverlays {
  tag: string
  version: string
  runDownload: (tag: string) => Promise<HarnessCore>
}

export function DownloadCoreDialog(props: DownloadCoreDialogProps) {
  const disclosure = useDisclosure({ props, delay: 300 })

  const { t } = useTranslation()
  const [percentage, setPercentage] = useState(0)
  const [logs, setLogs] = useState<string[]>([])
  const [errorMsg, setErrorMsg] = useState<string | null>(null)
  const error = errorMsg != null

  useListen<InstallProgress>('install-progress', (event) => {
    const payload = event.payload
    setPercentage(prev => Math.max(prev, payload.percentage))
    if (payload.log) {
      setLogs(prev => [...prev, payload.log].slice(-5))
    }
  })

  useEffect(() => {
    if (!disclosure.visible) {
      return
    }
    let cancelled = false

    props.runDownload(props.tag)
      .then(() => {
        if (!cancelled) {
          disclosure.confirm()
        }
      })
      .catch((err) => {
        if (!cancelled) {
          setErrorMsg(String(err))
        }
      })

    return () => {
      cancelled = true
    }
    // eslint-disable-next-line react/exhaustive-deps -- 仅打开时执行一次
  }, [disclosure.visible])

  const status = error ? 'danger' : 'default'

  return (
    <AlertDialog onOpenChange={disclosure.cancel} isOpen={disclosure.visible}>
      <AlertDialog.Backdrop>
        <AlertDialog.Container>
          <AlertDialog.Dialog className="sm:max-w-[420px]">
            <If cond={error}>
              <AlertDialog.CloseTrigger />
            </If>
            <AlertDialog.Header>
              <AlertDialog.Icon status={status} />
              <AlertDialog.Heading>
                {error ? t('core.download_failed') : t('core.downloading', { version: props.version })}
              </AlertDialog.Heading>
            </AlertDialog.Header>
            <AlertDialog.Body>
              <If
                cond={!error}
                else={(
                  <div className="flex flex-col gap-3">
                    <p className="break-all font-mono text-xs leading-[1.7] text-danger">{errorMsg}</p>
                    <Panel.Progress logs={logs} />
                  </div>
                )}
              >
                <div className="flex flex-col items-start gap-3">
                  <div className="flex items-center gap-2">
                    <Spinner size="sm" color="current" />
                    <span className="text-xs text-muted">{t('core.downloading_hint', { version: props.version })}</span>
                  </div>
                  <Panel.Progress percentage={percentage} logs={logs} />
                </div>
              </If>
            </AlertDialog.Body>
            <AlertDialog.Footer>
              <If cond={error}>
                <Button variant="tertiary" onPress={disclosure.cancel}>
                  {t('core.download_close')}
                </Button>
              </If>
            </AlertDialog.Footer>
          </AlertDialog.Dialog>
        </AlertDialog.Container>
      </AlertDialog.Backdrop>
    </AlertDialog>
  )
}
