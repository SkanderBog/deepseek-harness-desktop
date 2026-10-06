import { Button, Modal } from '@deepseek-ai/dsh-client-ui-primitives'
import { createRoot } from 'react-dom/client'
import { locale } from '../locales'

export interface ContentRiskRecoveryDialog {
  result: Promise<boolean>
  close: () => void
}

interface DialogOptions {
  title: string
  description: string
  confirmLabel?: string
}

export function openContentRiskRecoveryConfirmation(options: {
  safeSeq: number
  excludedEventCount: number
}): ContentRiskRecoveryDialog {
  return openDialog({
    title: locale.text('contentRiskRecoveryTitle'),
    description: locale.text('contentRiskRecoveryDescription', options),
    confirmLabel: locale.text('contentRiskRecoveryConfirm'),
  })
}

export function openContentRiskRecoveryUnavailable(): ContentRiskRecoveryDialog {
  return openDialog({
    title: locale.text('contentRiskRecoveryUnavailableTitle'),
    description: locale.text('contentRiskRecoveryUnavailableDescription'),
  })
}

function openDialog(options: DialogOptions): ContentRiskRecoveryDialog {
  if (typeof document === 'undefined')
    return { result: Promise.resolve(false), close: () => {} }
  const host = document.createElement('div')
  document.body.append(host)
  const root = createRoot(host)
  let settled = false
  let settle: (result: boolean) => void = () => {}
  const result = new Promise<boolean>((resolve) => {
    settle = resolve
  })
  const close = (accepted = false): void => {
    if (settled)
      return
    settled = true
    root.unmount()
    host.remove()
    settle(accepted)
  }
  root.render(
    <Modal
      open
      onClose={() => close(false)}
      closeLabel={locale.text('close')}
      title={options.title}
      description={options.description}
      footer={options.confirmLabel === undefined
        ? (
            <Button variant="primary" onClick={() => close(false)}>
              {locale.text('close')}
            </Button>
          )
        : (
            <>
              <Button variant="ghost" onClick={() => close(false)}>
                {locale.text('cancel')}
              </Button>
              <Button variant="primary" onClick={() => close(true)}>
                {options.confirmLabel}
              </Button>
            </>
          )}
    />,
  )
  return { result, close }
}
