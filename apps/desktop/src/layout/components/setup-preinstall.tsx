import type { IconComponent } from './loadable'
import type { PluginProcess } from '@/store/modules/plugins'
import type { PreinstallPlugin } from '@/store/modules/preinstall'
import { ArrowUpRightFromSquare, ArrowUturnCcwLeft, Comments, Copy, Cubes3Overlap, FileText, Gear, Layers, LayoutSideContentLeft, PlugConnection, ShoppingCart, Sparkles, Xmark } from '@gravity-ui/icons'
import { Button, Card, Checkbox, Chip, ScrollShadow, Spinner, Switch, Typography } from '@heroui/react'
import { useMount } from '@reause/core'
import { invoke } from '@tauri-apps/api/core'
import { useState } from 'react'
import { useTranslation } from 'react-i18next'
import { Else, If, Then } from 'react-if-lite'
import { useStore } from 'valtio-define'
import { Ellipsis } from '@/components/ellipsis'
import { Empty } from '@/components/empty'
import { Logs } from '@/components/logs'
import { useListen } from '@/hooks/use-listen'
import { useDshPluginsManager } from '@/hooks/use-plugins-manager'
import { store } from '@/store'
import { writeClipboardText } from '@/utils/clipboard'

function initialCheckedSet(plugins: readonly PreinstallPlugin[], isFirstTime: boolean): Set<string> {
  return new Set(plugins.filter((p) => {
    if (p.unsupported)
      return false
    if (p.installed)
      return true
    if (!isFirstTime)
      return false
    if (p.defaultUnchecked)
      return false
    return p.recommended || p.fix || p.defaultChecked
  }).map(p => p.id))
}

const PLUGIN_ICONS: Record<string, IconComponent | undefined> = {
  'dshmarket': ShoppingCart,
  'dsh-better-sidebar': LayoutSideContentLeft,
  'dsh-rewind-plugin': ArrowUturnCcwLeft,
  'billion-context': Cubes3Overlap,
  'dsh-office-kit': FileText,
  'dsh-ops': Gear,
  '@xmanrui/dsh-im': Comments,
  '@changfenhuang/dsh-genui': Sparkles,
  'dsh-context': Layers,
}

function PluginCard({ plugin, checked, toUninstall, disabled, onToggle, onOpenRepo }: {
  plugin: PreinstallPlugin
  checked: boolean
  toUninstall: boolean
  disabled: boolean
  onToggle: (id: string, checked: boolean) => void
  onOpenRepo: (id: string) => void
}) {
  const { t } = useTranslation()
  const PluginIcon = PLUGIN_ICONS[plugin.id] ?? PlugConnection

  return (
    <Card
      className={`h-[120px] gap-1 bg-panel2 p-3 shadow-none transition-colors ${plugin.unsupported ? 'opacity-60' : 'hover:border-line-strong'}`}
    >
      <div className="flex min-w-0 items-center gap-1.5">
        <PluginIcon className="size-3.5" />
        <span className="min-w-0 flex-1 truncate text-[13px] font-medium text-ink">
          {plugin.name}
        </span>
        <If cond={plugin.recommended && !plugin.installed && !toUninstall && !plugin.unsupported}>
          <Chip size="sm" variant="soft" color="success" className="shrink-0 font-medium">
            {t('preinstall.recommend')}
          </Chip>
        </If>
        <If cond={plugin.fix && !plugin.installed && !toUninstall && !plugin.unsupported}>
          <Chip size="sm" variant="soft" color="warning" className="shrink-0 font-medium">
            {t('preinstall.fix')}
          </Chip>
        </If>
        <If cond={plugin.unsupported}>
          <Chip size="sm" variant="soft" color="danger" className="shrink-0 font-medium">
            {t('preinstall.unsupported_core')}
          </Chip>
        </If>
        <If cond={plugin.installed && !toUninstall}>
          <Chip size="sm" variant="soft" color="default" className="shrink-0 font-medium">
            {t('preinstall.installed')}
          </Chip>
        </If>
        <If cond={toUninstall}>
          <Chip size="sm" variant="soft" color="warning" className="shrink-0 font-medium">
            {t('preinstall.to_uninstall')}
          </Chip>
        </If>
      </div>

      <If cond={plugin.description !== ''}>
        <Ellipsis lineClamp={2} className="text-[11px] leading-[17px] text-muted my-auto">
          {plugin.description}
        </Ellipsis>
      </If>

      <div className="flex items-center justify-between gap-2">
        <Button
          isIconOnly
          size="sm"
          variant="ghost"
          className="size-6 shrink-0 text-muted"
          aria-label={t('preinstall.open_repo', { name: plugin.name })}
          onPress={() => onOpenRepo(plugin.id)}
        >
          <ArrowUpRightFromSquare className="size-3" />
        </Button>
        <Switch
          size="sm"
          isSelected={checked}
          isDisabled={disabled || plugin.unsupported}
          onChange={isSelected => onToggle(plugin.id, isSelected)}
          aria-label={plugin.name}
        >
          <Switch.Content>
            <Switch.Control>
              <Switch.Thumb />
            </Switch.Control>
          </Switch.Content>
        </Switch>
      </div>
    </Card>
  )
}

function LogPanel({ logs }: { logs: readonly string[] }) {
  const { t } = useTranslation()
  const text = logs.join('\n')

  async function copyLogs() {
    try {
      await writeClipboardText(text || '', t('messages.log_copied'))
    }
    catch (err) {
      console.error('[Harness] copy preinstall logs failed:', err)
    }
  }

  return (
    <Logs
      logs={logs}
      limit={100}
      bodyClassName="max-h-[240px]"
      header={(
        <Button
          size="sm"
          variant="ghost"
          isIconOnly
          className="size-6 shrink-0"
          aria-label={t('buttons.copy')}
          onPress={copyLogs}
        >
          <Copy className="size-3.5" />
        </Button>
      )}
    />
  )
}

function refusedVersions(process: PluginProcess): string {
  return (process.refusal?.versions ?? []).map(item => `${item.name}@${item.version}`).join('、')
}

export function PreinstallSetup() {
  const { t } = useTranslation()
  const preinstall = useStore(store.preinstall)
  const manager = useDshPluginsManager({ toast: false, restartOnSettle: false })
  const [selected, setSelected] = useState<Set<string>>(() => new Set())
  const [touched, setTouched] = useState(false)
  const [allowed, setAllowed] = useState<Set<string>>(() => new Set())
  const [granting, setGranting] = useState(false)
  const [failures, setFailures] = useState<string[]>([])

  const approvals = manager.pendingApprovals
  const incompatibleApprovals = approvals.filter(process => process.refusal?.kind === 'incompatible')
  const policyApprovals = approvals.filter(process => process.refusal?.kind !== 'incompatible')

  useMount(() => {
    void store.preinstall.load()
  })

  useListen<{ line: string }>('preinstall-log', (event) => {
    store.preinstall.pushLog(event.payload.line)
  })

  const effectiveSelected = !touched
    ? initialCheckedSet(preinstall.plugins, preinstall.isFirstTime)
    : selected

  function toggle(id: string, checked: boolean) {
    const seed = !touched ? initialCheckedSet(preinstall.plugins, preinstall.isFirstTime) : null
    setTouched(true)
    setSelected((prev) => {
      const next = new Set(seed ?? prev)
      if (checked) {
        next.add(id)
      }
      else {
        next.delete(id)
      }
      return next
    })
  }

  function openRepo(id: string) {
    void invoke('open_preinstall_repo', { id }).catch((err) => {
      console.error('[Harness] open preinstall repo failed:', err)
    })
  }

  function collectFailures(results: Awaited<ReturnType<typeof manager.install>>): string[] {
    return results
      .filter(result => !result.ok && result.reason !== 'already-absent' && result.reason !== 'cancelled')
      .map(result => `${result.process.name}: ${result.error ?? ''}`)
  }

  async function handleConfirm() {
    const toInstall = preinstall.plugins
      .filter(p => effectiveSelected.has(p.id) && !p.installed)
      .map(p => p.id)
    const toUninstall = preinstall.plugins
      .filter(p => p.installed && !effectiveSelected.has(p.id) && !p.unsupported)
      .map(p => p.id)
    setFailures([])
    const failed: string[] = []
    if (toUninstall.length > 0)
      failed.push(...collectFailures(await manager.uninstall(toUninstall)))
    if (toInstall.length > 0)
      failed.push(...collectFailures(await manager.install(toInstall)))
    await store.preinstall.load()
    if (failed.length > 0) {
      setFailures(failed)
      return
    }
    await store.preinstall.finish()
  }

  function handleSkip() {
    void store.preinstall.skip()
  }

  function toggleAllowed(id: string, checked: boolean) {
    setAllowed((prev) => {
      const next = new Set(prev)
      if (checked) {
        next.add(id)
      }
      else {
        next.delete(id)
      }
      return next
    })
  }

  async function handleGrant(targets: PluginProcess[]) {
    const picked = targets.filter(process => allowed.has(process.id))
    if (granting || picked.length === 0)
      return
    setGranting(true)
    try {
      const failed = collectFailures(await manager.approve(picked.map(process => process.spec)))
      if (failed.length > 0)
        setFailures(failed)
    }
    finally {
      setGranting(false)
    }
  }

  const toInstallCount = preinstall.plugins.filter(p => effectiveSelected.has(p.id) && !p.installed).length
  const toUninstallCount = preinstall.plugins.filter(p => p.installed && !effectiveSelected.has(p.id) && !p.unsupported).length
  const hasChanges = toInstallCount > 0 || toUninstallCount > 0
  const installing = approvals.length === 0 && manager.processes.length > 0
  const blocked = approvals.length > 0
  const failing = failures.length > 0

  return (
    <div className="flex h-full w-full items-center justify-center bg-startup">
      <div className="flex w-[min(880px,92vw)] flex-col gap-4">
        <header className="flex flex-col items-center gap-1 text-center">
          <Typography type="h4" className="!text-ink">{t('preinstall.title')}</Typography>
          <Typography color="muted" type="body-sm" className="max-w-[440px]">{t('preinstall.subtitle')}</Typography>
        </header>

        <If
          cond={installing}
          else={(
            <If
              cond={blocked || failing}
              else={(
                <>
                  <ScrollShadow className="max-h-[min(50vh,420px)]" size={36}>
                    <div className="grid grid-cols-1 gap-2.5 pr-1 sm:grid-cols-2 lg:grid-cols-3">
                      <If
                        cond={preinstall.loadError === ''}
                        else={(
                          <div className="flex flex-col items-center gap-2 rounded-lg border border-danger/30 bg-danger/5 p-4 text-center sm:col-span-2 lg:col-span-3">
                            <p className="text-xs font-medium text-danger">{t('preinstall.load_failed')}</p>
                            <p className="max-h-[80px] max-w-full overflow-y-auto break-all font-mono text-[11px] text-muted">
                              {preinstall.loadError}
                            </p>
                            <Button
                              className="h-8"
                              size="sm"
                              variant="primary"
                              onPress={() => void store.preinstall.load()}
                              isDisabled={preinstall.loading}
                            >
                              {t('app.retry')}
                            </Button>
                          </div>
                        )}
                      >
                        <If
                          cond={preinstall.plugins.length > 0}
                          else={(
                            <Empty className="sm:col-span-2 lg:col-span-3">{t('preinstall.empty')}</Empty>
                          )}
                        >
                          {preinstall.plugins.map((plugin) => {
                            const checked = effectiveSelected.has(plugin.id)
                            const toUninstall = plugin.installed && !checked && !plugin.unsupported
                            return (
                              <PluginCard
                                key={plugin.id}
                                plugin={plugin}
                                checked={checked}
                                toUninstall={toUninstall}
                                disabled={installing}
                                onToggle={toggle}
                                onOpenRepo={openRepo}
                              />
                            )
                          })}
                        </If>
                      </If>
                    </div>
                  </ScrollShadow>

                  <If cond={preinstall.plugins.length > 0 && !installing && !failing}>
                    <p className="text-center text-xs text-muted">{t('preinstall.can_uncheck_hint')}</p>
                  </If>

                  <div className="flex items-center justify-end gap-2">
                    <If cond={hasChanges}>
                      <Button className="h-8" size="sm" variant="tertiary" data-testid="dsh-setup-preinstall-skip" onPress={handleSkip} isDisabled={installing}>
                        {t('preinstall.skip')}
                      </Button>
                    </If>
                    <If cond={!hasChanges}>
                      <Then>
                        <Button
                          className="h-8"
                          size="sm"
                          variant="primary"
                          data-testid="dsh-setup-preinstall-skip"
                          onPress={handleSkip}
                          isDisabled={installing}
                        >
                          {t('preinstall.skip')}
                        </Button>
                      </Then>
                      <Else>
                        <Button
                          className="h-8"
                          size="sm"
                          variant="primary"
                          onPress={handleConfirm}
                          isDisabled={installing}
                        >
                          {t('preinstall.confirm')}
                        </Button>
                      </Else>
                    </If>
                  </div>
                </>
              )}
            >
              <If
                cond={blocked}
                else={(
                  <div className="flex flex-col gap-2.5">
                    <div className="flex flex-col gap-2 rounded-md border border-danger/30 bg-danger/5 p-3">
                      <p className="text-xs font-medium text-danger">{t('preinstall.failed')}</p>
                      <p className="max-h-[120px] overflow-y-auto break-all font-mono text-[11px] leading-relaxed text-muted">
                        {failures.join('\n')}
                      </p>
                    </div>
                    <LogPanel logs={preinstall.logs} />
                    <div className="flex items-center justify-end gap-2">
                      <Button className="h-8" size="sm" variant="tertiary" onPress={handleSkip} isDisabled={installing}>
                        {t('preinstall.skip')}
                      </Button>
                      <Button
                        className="h-8"
                        size="sm"
                        variant="primary"
                        onPress={handleConfirm}
                        isDisabled={installing || !hasChanges}
                      >
                        {t('app.retry')}
                      </Button>
                    </div>
                  </div>
                )}
              >
                <div className="flex flex-col gap-2.5">
                  <If cond={incompatibleApprovals.length > 0}>
                    <div className="flex flex-col gap-2 rounded-md border border-danger/30 bg-danger/5 p-3">
                      <p className="text-xs font-medium text-danger">{t('preinstall.incompatible_title')}</p>
                      <p className="text-[11px] leading-relaxed text-muted">{t('preinstall.incompatible_desc')}</p>
                      <div className="mt-0.5 flex flex-col gap-1.5">
                        {incompatibleApprovals.map(process => (
                          <label key={process.id} className="flex cursor-pointer items-center gap-2">
                            <Checkbox
                              className="shrink-0"
                              isSelected={allowed.has(process.id)}
                              onChange={(value: boolean) => toggleAllowed(process.id, value)}
                              aria-label={process.name}
                            >
                              <Checkbox.Content>
                                <Checkbox.Control>
                                  <Checkbox.Indicator />
                                </Checkbox.Control>
                              </Checkbox.Content>
                            </Checkbox>
                            <span className="font-mono text-[11px] text-ink">{refusedVersions(process)}</span>
                          </label>
                        ))}
                      </div>
                    </div>
                  </If>

                  <If cond={policyApprovals.length > 0}>
                    <div className="flex flex-col gap-2 rounded-md border border-warning/30 bg-warning/5 p-3">
                      <p className="text-xs font-medium text-warning">{t('preinstall.policy_title')}</p>
                      <p className="text-[11px] leading-relaxed text-muted">{t('preinstall.policy_desc')}</p>
                      <div className="mt-0.5 flex flex-col gap-1.5">
                        {policyApprovals.map(process => (
                          <label key={process.id} className="flex cursor-pointer items-center gap-2">
                            <Checkbox
                              className="shrink-0"
                              isSelected={allowed.has(process.id)}
                              onChange={(value: boolean) => toggleAllowed(process.id, value)}
                              aria-label={process.name}
                            >
                              <Checkbox.Content>
                                <Checkbox.Control>
                                  <Checkbox.Indicator />
                                </Checkbox.Control>
                              </Checkbox.Content>
                            </Checkbox>
                            <span className="font-mono text-[11px] text-ink">{refusedVersions(process) || process.name}</span>
                          </label>
                        ))}
                      </div>
                    </div>
                  </If>

                  <LogPanel logs={preinstall.logs} />
                  <div className="flex items-center justify-end gap-2">
                    <Button className="h-8" size="sm" variant="tertiary" onPress={handleSkip} isDisabled={granting}>
                      {t('preinstall.skip')}
                    </Button>
                    <Button
                      className="h-8"
                      size="sm"
                      variant="primary"
                      onPress={() => void handleGrant(approvals)}
                      isDisabled={granting || allowed.size === 0}
                    >
                      <If cond={granting}>
                        <Spinner size="sm" color="current" />
                        <span>{t('preinstall.granting')}</span>
                      </If>
                      <If cond={!granting}>
                        <span>{t('preinstall.confirm')}</span>
                      </If>
                    </Button>
                  </div>
                </div>
              </If>
            </If>
          )}
        >
          <div className="flex flex-col gap-2.5">
            <div className="flex flex-col items-center gap-3">
              <Spinner size="md" color="current" />
              <p className="text-xs text-muted">{t('preinstall.installing')}</p>
            </div>
            <LogPanel logs={preinstall.logs} />
            <div className="flex items-center justify-center">
              <Button
                className="h-8"
                size="sm"
                variant="tertiary"
                onPress={() => void manager.cancel()}
                isDisabled={manager.cancelling || manager.processes.length === 0}
              >
                <Xmark className="size-3.5" />
                {manager.cancelling ? t('preinstall.cancelling') : t('preinstall.cancel')}
              </Button>
            </div>
          </div>
        </If>
      </div>
    </div>
  )
}
