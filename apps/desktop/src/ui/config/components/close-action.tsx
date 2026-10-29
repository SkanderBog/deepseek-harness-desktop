import { ListBox, Select } from '@heroui/react'
import { useMutation } from '@tanstack/react-query'
import { useTranslation } from 'react-i18next'
import { useStore } from 'valtio-define'
import { store } from '@/store'
import { CLOSE_ACTION_OPTIONS, normalizeCloseAction } from '@/utils/close-action'
import { toast } from '@/utils/toast'

const CLOSE_ACTION_LABEL_KEYS = {
  tray: 'ui.close_action_tray',
  quit: 'ui.close_action_quit',
}

export function ConfigCloseAction() {
  const { t } = useTranslation()
  const { close_action: closeAction } = useStore(store.setting)
  const { mutate: setCloseAction, isPending } = useMutation({
    mutationFn: (next: string) =>
      store.setting.update({ closeAction: normalizeCloseAction(next) }),
    onError: (error: unknown) => {
      console.error('[ConfigCloseAction] update failed:', error)
      toast(t('messages.close_action_failed'), { variant: 'danger' })
    },
  })

  return (
    <div>
      <div className="flex items-center justify-between gap-2">
        <span className="text-xs font-medium text-ink">{t('ui.close_action')}</span>
        <Select
          variant="secondary"
          selectedKey={normalizeCloseAction(closeAction)}
          onSelectionChange={key => setCloseAction(String(key))}
          isDisabled={isPending}
          className="w-[140px]"
          aria-label={t('ui.close_action')}
        >
          <Select.Trigger className="min-h-8! h-8 py-0 items-center">
            <Select.Value />
            <Select.Indicator />
          </Select.Trigger>
          <Select.Popover>
            <ListBox>
              {CLOSE_ACTION_OPTIONS.map(action => (
                <ListBox.Item
                  className="min-h-8!"
                  id={action}
                  key={action}
                  textValue={t(CLOSE_ACTION_LABEL_KEYS[action])}
                >
                  {t(CLOSE_ACTION_LABEL_KEYS[action])}
                </ListBox.Item>
              ))}
            </ListBox>
          </Select.Popover>
        </Select>
      </div>
      <p className="text-[10px] text-muted/70">{t('ui.close_action_hint')}</p>
    </div>
  )
}
