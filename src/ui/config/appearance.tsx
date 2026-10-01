import { Button, Label, ListBox, Select, Switch } from '@heroui/react'
import { useMutation } from '@tanstack/react-query'
import { useTranslation } from 'react-i18next'
import { If } from 'react-if-lite'
import { useStore } from 'valtio-define'
import { Panel } from '@/components/panel'
import { store } from '@/store'
import { toast } from '@/utils/toast'
import { APPEARANCE_DEFAULTS, APPEARANCE_PALETTES, normalizeAppearance } from '../../../packages/dsh-tauri/src/shared/appearance'

const OPACITY_OPTIONS = [100, 90, 80, 70, 60, 50, 40, 30, 20]

export function ConfigAppearance() {
  const { t } = useTranslation()
  const { appearance: saved } = useStore(store.setting)
  const appearance = normalizeAppearance(saved)
  const opacityOptions = OPACITY_OPTIONS.includes(appearance.opacity)
    ? OPACITY_OPTIONS
    : [...OPACITY_OPTIONS, appearance.opacity].sort((a, b) => b - a)
  const transparent = (window as Window & { __DSH_TRANSPARENT__?: boolean }).__DSH_TRANSPARENT__ === true
  const { mutate: save, isPending } = useMutation({
    mutationFn: (value: typeof appearance) => store.setting.update({ appearance: normalizeAppearance(value) }),
    onError: () => toast(t('appearance.save_failed'), { variant: 'danger' }),
  })

  return (
    <div className="space-y-5" data-testid="dsh-appearance-settings">
      <Panel.Header title={t('config.appearance')} description={t('appearance.description')} testId="dsh-config-panel-title" />
      <Select
        aria-label={t('appearance.palette')}
        selectedKey={appearance.palette}
        onSelectionChange={key => save(normalizeAppearance({ ...appearance, palette: key }))}
        isDisabled={isPending}
      >
        <Label>{t('appearance.palette')}</Label>
        <Select.Trigger data-testid="dsh-appearance-palette">
          <Select.Value />
          <Select.Indicator />
        </Select.Trigger>
        <Select.Popover>
          <ListBox>
            {APPEARANCE_PALETTES.map(palette => (
              <ListBox.Item key={palette} id={palette} textValue={t(`appearance.palette.${palette}`)} data-testid={`dsh-appearance-palette-${palette}`}>
                {t(`appearance.palette.${palette}`)}
              </ListBox.Item>
            ))}
          </ListBox>
        </Select.Popover>
      </Select>
      <div className="flex items-center justify-between gap-4">
        <div>
          <p className="text-sm">{t('appearance.terminal')}</p>
          <p className="text-xs text-muted">{t('appearance.terminal_description')}</p>
        </div>
        <Switch
          aria-label={t('appearance.terminal')}
          isSelected={appearance.terminal}
          isDisabled={isPending}
          onChange={terminal => save({ ...appearance, terminal })}
        >
          <Switch.Content data-testid="dsh-appearance-terminal">
            <Switch.Control><Switch.Thumb /></Switch.Control>
          </Switch.Content>
        </Switch>
      </div>
      <Select
        aria-label={t('appearance.opacity')}
        selectedKey={String(appearance.opacity)}
        onSelectionChange={key => save(normalizeAppearance({ ...appearance, opacity: Number(key) }))}
        isDisabled={isPending}
      >
        <Label>{t('appearance.opacity')}</Label>
        <Select.Trigger data-testid="dsh-appearance-opacity">
          <Select.Value />
          <Select.Indicator />
        </Select.Trigger>
        <Select.Popover>
          <ListBox>
            {opacityOptions.map(opacity => (
              <ListBox.Item key={opacity} id={String(opacity)} textValue={`${opacity}%`} data-testid={`dsh-appearance-opacity-${opacity}`}>{`${opacity}%`}</ListBox.Item>
            ))}
          </ListBox>
        </Select.Popover>
      </Select>
      <p className="text-xs text-muted">{t('appearance.opacity_description')}</p>
      <If cond={(appearance.opacity < 100) !== transparent}>
        <p role="status" data-testid="dsh-appearance-restart" className="text-sm text-info">{t('appearance.restart')}</p>
      </If>
      <Button variant="secondary" isDisabled={isPending} onPress={() => save(normalizeAppearance(APPEARANCE_DEFAULTS))} data-testid="dsh-appearance-reset">
        {t('appearance.reset')}
      </Button>
    </div>
  )
}
