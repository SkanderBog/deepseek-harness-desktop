import type { Remote } from '@/hooks/use-remote'
import { Button, Description, Modal, Spinner } from '@heroui/react'
import { useTranslation } from 'react-i18next'
import { If } from 'react-if-lite'
import { cn } from 'tailwind-variants'
import { Logs } from '@/components/logs'

export function ConnectDialog({ remote }: { remote: Remote }) {
  const { t } = useTranslation()
  const { machines, pendingId, connectFailed, connectTrail, connectLog, connectDismissed } = remote

  const targetId = pendingId ?? connectFailed?.id ?? null
  const machine = targetId === null ? undefined : machines.find(item => item.id === targetId)
  const isOpen = connectFailed !== null || (pendingId !== null && !connectDismissed)
  const failed = connectFailed !== null
  const currentPhase = machine?.progress?.phase ?? (failed ? undefined : connectTrail[connectTrail.length - 1])

  function handleRetry() {
    if (connectFailed === null)
      return
    remote.dismissConnect()
    remote.switchTo(connectFailed.id)
  }

  return (
    <Modal
      isOpen={isOpen}
      onOpenChange={(open: boolean) => {
        if (!open)
          remote.dismissConnect()
      }}
    >
      <Modal.Backdrop>
        <Modal.Container size="sm">
          <Modal.Dialog>
            <Modal.CloseTrigger />
            <Modal.Header className="mb-2">
              <Modal.Heading>
                {failed ? t('remote.connect.failed_title') : t('remote.connect.title', { name: machine?.name ?? '' })}
              </Modal.Heading>
            </Modal.Header>
            <Modal.Body className="space-y-3">
              <If cond={connectTrail.length > 0 || currentPhase !== undefined}>
                <div className="flex items-center gap-1.5" data-testid="connect-steps">
                  {(connectTrail.length > 0 ? connectTrail : currentPhase !== undefined ? [currentPhase] : []).map((phase, index, all) => (
                    <span
                      // eslint-disable-next-line react/no-array-index-key
                      key={`${phase}-${index}`}
                      className={cn(
                        'rounded-md px-2 py-0.5 text-xs',
                        index === all.length - 1 && !failed ? 'bg-info/10 text-info' : 'bg-panel2 text-muted',
                      )}
                    >
                      {t(`remote.step.${phase}`)}
                    </span>
                  ))}
                </div>
              </If>
              <If cond={failed}>
                <Description className="text-danger break-all">{connectFailed?.error}</Description>
              </If>
              <Logs logs={connectLog} bodyClassName="max-h-[160px]" />
              <If cond={!failed}>
                <Description className="flex items-center gap-1.5">
                  <Spinner className="size-3" />
                  {t('remote.connect.working')}
                </Description>
              </If>
            </Modal.Body>
            <Modal.Footer>
              <If cond={failed}>
                <Button variant="tertiary" onPress={() => remote.dismissConnect()}>
                  {t('buttons.close')}
                </Button>
                <Button variant="primary" onPress={handleRetry}>
                  {t('remote.connect.retry')}
                </Button>
              </If>
              <If cond={!failed}>
                <Button variant="tertiary" onPress={() => remote.dismissConnect()}>
                  {t('buttons.close')}
                </Button>
                <Button
                  variant="secondary"
                  data-testid="connect-cancel"
                  onPress={() => pendingId !== null && remote.disconnect(pendingId)}
                >
                  {t('remote.connect.cancel')}
                </Button>
              </If>
            </Modal.Footer>
          </Modal.Dialog>
        </Modal.Container>
      </Modal.Backdrop>
    </Modal>
  )
}
