import type { HarnessCore } from '@/types'
import { ArrowRotateRight, ChevronRight, CircleArrowDown as DownloadIcon, FolderOpen } from '@gravity-ui/icons'
import { Button, Checkbox, Chip, Description, Label, Spinner } from '@heroui/react'
import { useOverlay } from '@overlastic/react'
import { useToggle } from '@reause/core'
import { useMutation, useQuery, useQueryClient } from '@tanstack/react-query'
import { invoke } from '@tauri-apps/api/core'
import { Fragment, useState } from 'react'
import { useTranslation } from 'react-i18next'
import { If } from 'react-if-lite'
import { Empty } from '@/components/empty'
import { Item } from '@/components/item'
import { Modal } from '@/components/modal'
import { Panel } from '@/components/panel'
import { queryKeys } from '@/config/query-keys'
import { useInvalidateOnSettingUpdated } from '@/hooks/use-invalidate-on-setting-updated'
import { store } from '@/store'
import { DownloadCoreDialog } from '@/ui/dialog/update-core'
import { compareVersions, isCoreUnsupported, MIN_SUPPORTED_CORE_VERSION } from '@/utils/core-version'
import { silence } from '@/utils/silence'
import { toast } from '@/utils/toast'

export function ConfigCore() {
  const [dialogHolder, openDialog] = useOverlay(Modal, { type: 'holder' })
  const [downloadDialogHolder, openDownloadDialog] = useOverlay(DownloadCoreDialog, { type: 'holder' })

  const { t } = useTranslation()

  const queryClient = useQueryClient()
  const { data: coreList, isLoading, error: coreError, refetch } = useQuery({
    queryKey: queryKeys.cores,
    queryFn: () => invoke<HarnessCore[]>('get_cores'),
  })
  useInvalidateOnSettingUpdated(queryKeys.cores)

  function invalidate() {
    void queryClient.invalidateQueries({ queryKey: queryKeys.cores })
  }

  const activate = useMutation({
    mutationFn: (id: string) => invoke<HarnessCore>('set_active_core', { id }),
    onSuccess: invalidate,
  })
  const download = useMutation({
    mutationFn: (tag: string) => invoke<HarnessCore>('download_core', { tag }),
    onSuccess: invalidate,
  })
  const uninstall = useMutation({
    mutationFn: (id: string) => invoke<void>('remove_core', { id }),
    onSuccess: invalidate,
  })
  const updateLocal = useMutation({
    mutationFn: () => invoke<string>('update_local_core'),
    onSuccess: invalidate,
  })

  const cores = coreList ?? []
  const loading = isLoading
  const error = coreError ? String(coreError) : ''
  const busy = activate.isPending || download.isPending || uninstall.isPending || updateLocal.isPending

  const [busyId, setBusyId] = useState<string | null>(null)
  const [showIncompatible, setShowIncompatible] = useState(false)
  const [refreshing, toggleRefreshing] = useToggle()

  const rows = cores
    .filter(core => !(core.source === 'local' && !core.present))
    .sort((a, b) => {
      if (a.bundled !== b.bundled)
        return a.bundled ? -1 : 1
      if (a.source !== b.source)
        return a.source === 'local' ? -1 : 1
      if (a.source === 'local')
        return 0
      return -compareVersions(a.version || a.tag, b.version || b.tag)
    })
  const localCore = cores.find(c => c.source === 'local')
  const currentRows = rows.filter(core => !core.orphaned)
  const orphanRows = rows.filter(core => core.orphaned)
  // 加载随包内置插件（issue #596），与可用版本并列只会让用户误选。
  const orderedRows = [...currentRows, ...orphanRows]
  const incompatibleRows = orderedRows.filter(core => isUnsupportedCore(core))
  const displayRows = [...orderedRows.filter(core => !isUnsupportedCore(core)), ...incompatibleRows]
  const firstOrphanRow = displayRows.find(core => core.orphaned)

  const localVersion = localCore?.version ?? ''
  const latestVersion = cores.find(c => c.source === 'app' && !c.preview && !c.aboveRecommended)?.version ?? ''
  const hasLocalUpdate = !!(localCore?.present && localVersion && latestVersion && compareVersions(localVersion, latestVersion) < 0)

  async function runBusy(id: string, action: () => Promise<unknown>) {
    if (busy)
      return
    setBusyId(id)
    try {
      await action()
    }
    finally {
      setBusyId(null)
    }
  }

  async function onActivate(core: HarnessCore) {
    if (core.active || busy || !core.present)
      return
    // 低于最低支持基线的本地核心无法加载随包插件（issue #596）：桌面端此时使用
    if (isUnsupportedLocal(core)) {
      toast(t('core.local_unsupported_toast'), {
        variant: 'warning',
        description: t('core.local_unsupported_hint', { version: MIN_SUPPORTED_CORE_VERSION }),
        timeout: 10_000,
      })
      return
    }
    try {
      const isRiskyVersion = core.recommendedVersion !== null
        && (core.aboveRecommended || compareVersions(core.version, core.recommendedVersion) > 0)
      await openDialog({
        status: isRiskyVersion ? 'danger' : 'warning',
        title: isRiskyVersion ? t('core.recommended_warning_title') : t('core.switch_confirm_title'),
        description: (
          <p>
            <If
              cond={isRiskyVersion}
              then={t('core.recommended_warning_desc', { version: core.recommendedVersion ?? '' })}
              else={t('core.switch_confirm_desc', { version: displayVersion(core) })}
            />
          </p>
        ),
      })
    }
    catch (e) {
      silence(e, 'core switch: dialog cancelled')
      return
    }
    try {
      await runBusy(core.id, () => activate.mutateAsync(core.id))
      const key = toast(t('core.activate_toast', { version: displayVersion(core) }), {
        variant: 'accent',
        description: t('core.switch_restart_hint'),
        timeout: 10_000,
      })
      try {
        await store.harness.restart()
        toast.close(key)
      }
      catch (e) {
        silence(e, 'core switch: restart failure already shown by app error state')
      }
    }
    catch (err) {
      console.error('[ConfigCore] switch failed:', err)
      toast(t('core.switch_failed'), {})
    }
  }

  async function onDownload(core: HarnessCore) {
    if (busy)
      return
    try {
      await openDownloadDialog({
        tag: core.tag,
        version: displayVersion(core),
        runDownload: async (tag) => {
          const downloaded = await download.mutateAsync(tag)
          await refetch()
          return downloaded
        },
      })
      toast(t('core.downloaded_toast', { version: displayVersion(core) }), {})
    }
    catch (err) {
      console.error('[ConfigCore] download failed:', err)
    }
  }

  async function openCoreDir(core: HarnessCore) {
    if (!core.dir || busy)
      return
    try {
      await invoke('open_dir', { path: core.dir })
    }
    catch (err) {
      console.error('[ConfigCore] open dir failed:', err)
      toast(t('core.open_dir_failed'), {})
    }
  }

  async function onRemove(core: HarnessCore) {
    if (busy)
      return
    try {
      await openDialog({
        status: 'danger',
        title: t('core.remove_confirm_title'),
        description: (
          <p>
            {t('core.remove_confirm_desc', { version: displayVersion(core) })}
          </p>
        ),
        confirmText: t('core.uninstall'),
      })
    }
    catch (e) {
      silence(e, 'core switch: dialog cancelled')
      return
    }
    try {
      await runBusy(core.id, async () => {
        await uninstall.mutateAsync(core.id)
        await refetch()
      })
      toast(t('core.uninstalled_toast', { version: displayVersion(core) }), {})
    }
    catch (err) {
      console.error('[ConfigCore] remove failed:', err)
      toast(t('core.remove_failed'), {})
    }
  }

  async function onRefresh() {
    if (busy || refreshing)
      return
    toggleRefreshing(true)
    try {
      await refetch()
      toast(t('core.refreshed_toast'), {})
    }
    catch (err) {
      console.error('[ConfigCore] refresh failed:', err)
      toast(t('core.refresh_failed'), {})
    }
    finally {
      toggleRefreshing(false)
    }
  }

  function showLocalUpdateWarning() {
    toast(t('core.update_local_requires_app'), {
      variant: 'warning',
      description: t('core.update_local_requires_app_hint'),
      timeout: 10_000,
    })
  }

  async function onUpdateLocal() {
    if (busy)
      return
    if (localCore?.active) {
      showLocalUpdateWarning()
      return
    }
    setBusyId('local')
    try {
      const version = await updateLocal.mutateAsync()
      await refetch()
      toast(t('core.updated_toast', { version: version || '—' }), {
        variant: 'success',
        description: t('core.updated_switch_back_hint'),
        timeout: 10_000,
      })
    }
    catch (err) {
      const message = err instanceof Error ? err.message : String(err)
      if (message === 'CORE_LOCAL_UPDATE_REQUIRES_APP' || message.startsWith('CORE_LOCAL_UPDATE_REQUIRES_APP:')) {
        showLocalUpdateWarning()
        return
      }
      console.error('[ConfigCore] update local core failed:', err)
      toast(t('core.update_failed'), {})
    }
    finally {
      setBusyId(null)
    }
  }

  return (
    <div className="space-y-3">
      <Panel.Header
        title={t('core.title')}
        description={t('core.tooltip')}
        testId="dsh-config-panel-title"
        action={(
          <Button
            size="sm"
            variant="tertiary"
            isDisabled={busy || refreshing}
            aria-label={t('core.refresh')}
            onPress={onRefresh}
          >
            <If cond={refreshing} then={<Spinner size="sm" color="current" />} else={<ArrowRotateRight className="size-3.5" />} />
            {t('core.refresh')}
          </Button>
        )}
      />

      <Panel.Loadable loading={loading} error={error}>
        <div className="flex flex-col gap-4">
          <If cond={!localCore?.present}>
            <Empty>{t('core.local_missing_hint')}</Empty>
          </If>
          {displayRows.map(core => (
            <Fragment key={core.id}>
              <If cond={core === incompatibleRows[0]}>
                <button
                  type="button"
                  className="flex items-center gap-1 px-1 pt-2 text-left text-xs font-medium text-muted"
                  aria-expanded={showIncompatible}
                  onClick={() => setShowIncompatible(value => !value)}
                >
                  <ChevronRight className={showIncompatible ? 'size-3.5 rotate-90' : 'size-3.5'} />
                  {t('core.incompatible_title', { count: incompatibleRows.length })}
                </button>
              </If>
              <If cond={core.orphaned && (core === firstOrphanRow)}>
                <div className="px-1 pt-2 text-xs font-medium text-muted">
                  {t('core.orphaned_title')}
                </div>
              </If>
              <Item
                className={!showIncompatible && isUnsupportedCore(core) ? 'hidden' : undefined}
                onClick={core.present && !core.active ? () => onActivate(core) : undefined}
                left={(
                  <>
                    <Label className="min-w-0 truncate font-mono text-sm font-medium text-ink">
                      {displayVersion(core)}
                    </Label>
                    <If cond={core.source === 'local' || core.bundled}>
                      <Chip size="sm" variant="soft" color="accent" className="shrink-0 font-medium">
                        {t('core.local')}
                      </Chip>
                    </If>
                    <If cond={core.orphaned}>
                      <Chip size="sm" variant="soft" color="warning" className="shrink-0 font-medium">
                        {t('core.orphaned')}
                      </Chip>
                    </If>
                    <If cond={core.preview}>
                      <Chip size="sm" variant="soft" color="warning" className="shrink-0 font-medium">
                        {t('core.preview')}
                      </Chip>
                    </If>
                    <If cond={isUnsupportedLocal(core)}>
                      <Chip size="sm" variant="soft" color="danger" className="shrink-0 font-medium">
                        {t('core.local_unsupported')}
                      </Chip>
                    </If>
                    <If cond={core.present}>
                      <Button
                        size="sm"
                        variant="tertiary"
                        className="h-6 w-6 shrink-0 p-0"
                        isDisabled={busy}
                        aria-label={t('core.open_dir')}
                        onClick={(event) => {
                          event.stopPropagation()
                          openCoreDir(core)
                        }}
                      >
                        <FolderOpen className="size-3.5" />
                      </Button>
                    </If>
                    <If cond={!core.present}>
                      <Description className="min-w-0 text-xs text-muted">
                        {t('core.not_downloaded')}
                      </Description>
                    </If>
                  </>
                )}
                right={(
                  <>
                    <If cond={core.present}>
                      <Checkbox
                        isSelected={core.active}
                        isDisabled={busy}
                        onChange={() => onActivate(core)}
                        aria-label={core.version || core.id}
                        className="shrink-0"
                      >
                        <Checkbox.Content>
                          <Checkbox.Control>
                            <Checkbox.Indicator />
                          </Checkbox.Control>
                        </Checkbox.Content>
                      </Checkbox>
                    </If>
                    <If cond={!core.present && core.source === 'app'}>
                      <Button
                        size="sm"
                        variant="tertiary"
                        className="h-7 text-xs"
                        isDisabled={busy}
                        onClick={(event) => {
                          event.stopPropagation()
                          onDownload(core)
                        }}
                      >
                        <DownloadIcon className="size-3.5" />
                        {t('core.download')}
                      </Button>
                    </If>
                    <If cond={core.present && !core.active && core.removable}>
                      <Button
                        size="sm"
                        variant="tertiary"
                        className="h-7 text-xs"
                        isDisabled={busy}
                        onClick={(event) => {
                          event.stopPropagation()
                          onRemove(core)
                        }}
                      >
                        <If cond={busyId === core.id && busy} then={<Spinner size="sm" color="current" />} />
                        {t('core.uninstall')}
                      </Button>
                    </If>
                    <If cond={core.source === 'local' && core.present && hasLocalUpdate}>
                      <Button
                        size="sm"
                        variant="tertiary"
                        className="h-7 text-xs"
                        isDisabled={busy}
                        onClick={(event) => {
                          event.stopPropagation()
                          onUpdateLocal()
                        }}
                      >
                        <If cond={busyId === core.id && busy} then={<Spinner size="sm" color="current" />} else={<ArrowRotateRight className="size-3.5" />} />
                        {t('core.update_local')}
                      </Button>
                    </If>
                  </>
                )}
              />
            </Fragment>
          ))}
        </div>
      </Panel.Loadable>

      {dialogHolder}
      {downloadDialogHolder}
    </div>
  )
}

function displayVersion(version: HarnessCore): string {
  return version.version || (version.source === 'local' ? 'local' : 'app')
}

function isUnsupportedLocal(core: HarnessCore): boolean {
  return core.source === 'local' && core.present && isUnsupportedCore(core)
}

function isUnsupportedCore(core: HarnessCore): boolean {
  return isCoreUnsupported(core.version || core.tag)
}
