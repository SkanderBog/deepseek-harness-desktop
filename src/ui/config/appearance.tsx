import { Button, Label, ListBox, Select, Slider, Switch } from '@heroui/react'
import { useMutation } from '@tanstack/react-query'
import { useState } from 'react'
import { useTranslation } from 'react-i18next'
import { If } from 'react-if-lite'
import { useStore } from 'valtio-define'
import { Panel } from '@/components/panel'
import { store } from '@/store'
import { toast } from '@/utils/toast'
import { APPEARANCE_DEFAULTS, APPEARANCE_PALETTES, normalizeAppearance } from '../../../packages/dsh-tauri/src/shared/appearance'

export function ConfigAppearance() {
  const { t } = useTranslation()
  const { appearance: saved } = useStore(store.setting)
  const appearance = normalizeAppearance(saved)
  const [opacity, setOpacity] = useState<number>()
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
      <div className="flex items-center justify-between gap-4">
        <Label>{t('appearance.transparency')}</Label>
        <Switch
          aria-label={t('appearance.transparency')}
          isSelected={appearance.transparency}
          isDisabled={isPending}
          onChange={transparency => save({ ...appearance, transparency })}
        >
          <Switch.Content data-testid="dsh-appearance-transparency">
            <Switch.Control><Switch.Thumb /></Switch.Control>
          </Switch.Content>
        </Switch>
      </div>
      <If cond={appearance.transparency}>
        <Slider
          aria-label={t('appearance.opacity')}
          value={opacity ?? appearance.opacity}
          minValue={20}
          maxValue={100}
          step={1}
          isDisabled={isPending}
          onChange={value => setOpacity(Number(value))}
          onChangeEnd={value => save({ ...appearance, opacity: Number(value) }, { onSettled: () => setOpacity(undefined) })}
          data-testid="dsh-appearance-opacity"
        >
          <div className="flex items-center justify-between">
            <Label>{t('appearance.opacity')}</Label>
            <Slider.Output>{({ state }) => `${state.values[0]}%`}</Slider.Output>
          </div>
          <Slider.Track>
            <Slider.Fill />
            <Slider.Thumb />
          </Slider.Track>
        </Slider>
        <p className="text-xs text-muted">{t('appearance.opacity_description')}</p>
        <div className="flex items-center justify-between gap-4">
          <Label>{t('appearance.sidebar_only')}</Label>
          <Switch
            aria-label={t('appearance.sidebar_only')}
            isSelected={appearance.sidebarOnly}
            isDisabled={isPending}
            onChange={sidebarOnly => save({ ...appearance, sidebarOnly })}
          >
            <Switch.Content data-testid="dsh-appearance-sidebar-only">
              <Switch.Control><Switch.Thumb /></Switch.Control>
            </Switch.Content>
          </Switch>
        </div>
      </If>
      <If cond={appearance.transparency !== transparent}>
        <p role="status" data-testid="dsh-appearance-restart" className="text-sm text-info">{t('appearance.restart')}</p>
      </If>
      <Button variant="secondary" isDisabled={isPending} onPress={() => save(normalizeAppearance(APPEARANCE_DEFAULTS))} data-testid="dsh-appearance-reset">
        {t('appearance.reset')}
      </Button>
    </div>
  )
}
