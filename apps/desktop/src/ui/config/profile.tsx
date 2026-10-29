import type { Profile } from '@/types'
import { CopyArrowRight, Plus } from '@gravity-ui/icons'
import { AlertDialog, Button, Checkbox, Chip, Description, Input, Label } from '@heroui/react'
import { useOverlay } from '@overlastic/react'
import { useToggle } from '@reause/core'
import { useMutation, useQuery, useQueryClient } from '@tanstack/react-query'
import { invoke } from '@tauri-apps/api/core'
import { useState } from 'react'

import { useTranslation } from 'react-i18next'
import { If } from 'react-if-lite'
import { Ellipsis } from '@/components/ellipsis'
import { Item } from '@/components/item'
import { Modal } from '@/components/modal'
import { Panel } from '@/components/panel'
import { queryKeys } from '@/config/query-keys'
import { useInvalidateOnSettingUpdated } from '@/hooks/use-invalidate-on-setting-updated'
import { store } from '@/store'
import { waitForHarnessStopped } from '@/store/modules/harness'
import { ConfigBackup } from '@/ui/config/backup'
import { ProfileMigrateDialog } from '@/ui/dialog/profile-migrate'
import { normalizeProfileId } from '@/utils/profile-id'
import { silence } from '@/utils/silence'
import { toast } from '@/utils/toast'

type ProfileView = 'list' | { profile: string }

const PROFILE_ERROR_KEYS: Record<string, string> = {
  PROFILE_EMPTY_NAME: 'profiles.error_empty_name',
  PROFILE_NAME_TOO_LONG: 'profiles.error_name_too_long',
  PROFILE_RESERVED: 'profiles.error_reserved',
  PROFILE_NOT_FOUND: 'profiles.error_not_found',
}

export function ConfigProfile() {
  const queryClient = useQueryClient()
  const { data: profileList, isLoading, error: profileError, refetch } = useQuery({
    queryKey: queryKeys.profiles,
    queryFn: () => invoke<Profile[]>('get_profiles'),
  })
  useInvalidateOnSettingUpdated(queryKeys.profiles)

  function invalidate() {
    void queryClient.invalidateQueries({ queryKey: queryKeys.profiles })
  }

  const create = useMutation({
    mutationFn: (name: string) => invoke<Profile>('create_profile', { name }),
    onSuccess: invalidate,
  })
  const activate = useMutation({
    mutationFn: (id: string) => store.harness.switchProfile(id),
    onSuccess: invalidate,
  })
  const remove = useMutation({
    mutationFn: (id: string) => invoke<void>('remove_profile', { id }),
    onSuccess: invalidate,
  })
  const reset = useMutation({
    mutationFn: (id: string) => invoke<void>('reset_profile', { id }),
    onSuccess: invalidate,
  })
  const clone = useMutation({
    mutationFn: (params: { sourceId: string, name: string }) => invoke<Profile>('clone_profile', params),
    onSuccess: invalidate,
  })

  async function createProfile(name: string): Promise<Profile> {
    const created = await create.mutateAsync(name)
    await refetch()
    return created
  }

  async function activateProfile(id: string): Promise<void> {
    await activate.mutateAsync(id)
    await refetch()
  }

  async function removeProfile(id: string) {
    await remove.mutateAsync(id)
    await refetch()
  }

  async function resetProfile(id: string) {
    await reset.mutateAsync(id)
    await refetch()
  }

  async function cloneProfile(sourceId: string, name: string): Promise<Profile> {
    const created = await clone.mutateAsync({ sourceId, name })
    await refetch()
    return created
  }

  const profiles = profileList ?? []
  const loading = isLoading
  const error = profileError ? String(profileError) : ''
  const activeProfile = profiles.find(profile => profile.active)
  const busy = create.isPending || activate.isPending || remove.isPending || reset.isPending || clone.isPending

  const migrateBlocked = busy || !activeProfile || profiles.length < 2

  const [dialogHolder, openDialog] = useOverlay(Modal, { type: 'holder' })

  const { t } = useTranslation()
  const [creating, toggleCreating] = useToggle()
  const [name, setName] = useState('')
  const [activeView, setActiveView] = useState<ProfileView>('list')

  const [cloning, setCloning] = useState<{ sourceId: string, sourceName: string } | null>(null)
  const [cloneName, setCloneName] = useState('')

  const [migrateDialogHolder, openMigrateDialog] = useOverlay(ProfileMigrateDialog, { type: 'holder' })

  function profileNameHint(value: string): string | null {
    if (value.trim() === '' || normalizeProfileId(value) !== '')
      return null
    return t('profiles.name_invalid_hint')
  }

  function profileErrorText(error: unknown, name: string): string {
    const code = String(error).split(':')[0]?.trim() ?? ''
    if (code === 'PROFILE_INVALID_NAME')
      return t('profiles.name_invalid_hint')
    if (code === 'PROFILE_EXISTS')
      return t('profiles.clone_exists', { name })
    const key = PROFILE_ERROR_KEYS[code]
    return key ? t(key) : String(error)
  }

  const createNameHint = profileNameHint(name)
  const cloneNameHint = profileNameHint(cloneName)

  function suggestCloneName(base: string): string {
    const taken = new Set(profiles.map(p => p.id))
    for (let n = 1; n <= 1000; n++) {
      const candidate = `${base}-${n}`
      if (!taken.has(candidate))
        return candidate
    }
    return `${base}-1`
  }

  function openCloneDialog(profile: { id: string, name: string }) {
    setCloning({ sourceId: profile.id, sourceName: profile.name })
    setCloneName(suggestCloneName(profile.id))
  }

  async function commitClone() {
    if (!cloning)
      return
    const trimmed = cloneName.trim()
    if (!trimmed) {
      toast(t('profiles.clone_empty'), {})
      return
    }
    if (cloneNameHint !== null)
      return
    try {
      await cloneProfile(cloning.sourceId, trimmed)
      setCloning(null)
      setCloneName('')
      toast(t('profiles.clone_success', { name: trimmed }), {
        variant: 'accent',
        description: t('profiles.clone_success_hint'),
        timeout: 10_000,
      })
    }
    catch (err) {
      console.error('[ConfigProfile] clone failed:', err)
      toast(t('profiles.clone_failed'), { description: profileErrorText(err, trimmed) })
    }
  }

  async function onActivate(id: string) {
    const target = profiles.find(p => p.id === id)
    if (!target || target.active || busy)
      return
    try {
      await openDialog({
        status: 'warning',
        title: t('profiles.activate_confirm_title'),
        description: (
          <p>
            {t('profiles.activate_confirm_desc', { name: target.name })}
          </p>
        ),
      })
    }
    catch (e) {
      silence(e, 'profile activate: dialog cancelled')
      return
    }
    try {
      await activateProfile(id)
    }
    catch (err) {
      console.error('[ConfigProfile] activate failed:', err)
      toast(t('profiles.activate_failed', { name: target.name }), {})
    }
  }

  function startCreate() {
    toggleCreating(true)
    setName('')
  }

  function cancelCreate() {
    toggleCreating(false)
    setName('')
  }

  async function commitCreate() {
    const trimmed = name.trim()
    if (!trimmed)
      return
    if (createNameHint !== null)
      return
    try {
      await createProfile(trimmed)
      toggleCreating(false)
      setName('')
    }
    catch (err) {
      console.error('[ConfigProfile] create failed:', err)
      toast(t('profiles.create_failed'), { description: profileErrorText(err, trimmed) })
    }
  }

  async function onRemove(id: string) {
    const target = profiles.find(p => p.id === id)
    if (!target || busy)
      return
    try {
      await openDialog({
        title: t('profiles.remove_confirm_title'),
        status: 'danger',
        description: (
          <p>
            {t('profiles.remove_confirm_desc', { name: target.name })}
          </p>
        ),
        confirmText: t('profiles.remove_confirm'),
      })
    }
    catch (e) {
      silence(e, 'profile remove: dialog cancelled')
      return
    }
    try {
      await removeProfile(id)
    }
    catch (err) {
      console.error('[ConfigProfile] remove failed:', err)
      toast(t('profiles.remove_failed'), {})
    }
  }

  async function onReset(id: string) {
    const target = profiles.find(p => p.id === id)
    if (!target || busy)
      return
    try {
      await openDialog({
        status: 'danger',
        title: t('profiles.reset_confirm_title'),
        description: (
          <p>
            {t('profiles.reset_confirm_desc', { name: target.name })}
          </p>
        ),
        confirmText: t('profiles.reset_confirm'),
      })
    }
    catch (e) {
      silence(e, 'profile reset: dialog cancelled')
      return
    }
    if (target.active) {
      toast(t('profiles.reset_stopped_toast'), { variant: 'accent' })
      try {
        await invoke('shutdown_harness')
      }
      catch (e) {
        console.warn('[ConfigProfile] shutdown_harness failed (may already be stopped):', e)
      }
      await waitForHarnessStopped()
    }
    try {
      await resetProfile(id)
      if (target.active) {
        invoke('launch_harness').catch((e) => {
          console.warn('[ConfigProfile] launch_harness failed:', e)
        })
      }
      const key = toast(t('profiles.reset_success', { name: target.name }), {
        variant: 'accent',
        description: t('profiles.reset_success_hint'),
        timeout: 10_000,
        actionProps: {
          children: t('app.restart'),
          onPress: () => {
            store.harness.restart()
            toast.close(key)
          },
        },
      })
    }
    catch (err) {
      console.error('[ConfigProfile] reset failed:', err)
      toast(t('profiles.reset_failed'), {})
    }
  }

  if (activeView !== 'list') {
    return (
      <ConfigBackup onBack={() => setActiveView('list')} />
    )
  }

  return (
    <div className="space-y-3">
      <Panel.Header
        title={t('profiles.title')}
        description={t('profiles.tooltip')}
        testId="dsh-config-panel-title"
        action={(
          <Button
            size="sm"
            variant="primary"
            isDisabled={migrateBlocked}
            onPress={() => void openMigrateDialog()}
          >
            <CopyArrowRight className="size-3.5" />
            {t('profiles.migrate')}
          </Button>
        )}
      />

      <Panel.Loadable loading={loading} error={error}>
        <div className="flex flex-col gap-4">
          {profiles.map(profile => (
            <Item
              key={profile.id}
              onClick={() => onActivate(profile.id)}
              left={(
                <>
                  <Label className="min-w-0 truncate text-sm font-medium text-ink">
                    {profile.name}
                  </Label>
                  <If cond={profile.default}>
                    <Description className="min-w-0 text-xs text-muted">
                      <Ellipsis>{t('profiles.default_desc')}</Ellipsis>
                    </Description>
                  </If>
                </>
              )}
              right={(
                <>
                  <Checkbox
                    isSelected={profile.active}
                    isDisabled={busy}
                    onChange={() => onActivate(profile.id)}
                    aria-label={profile.name}
                    className="shrink-0"
                  >
                    <Checkbox.Content>
                      <Checkbox.Control>
                        <Checkbox.Indicator />
                      </Checkbox.Control>
                    </Checkbox.Content>
                  </Checkbox>
                  <Chip
                    size="sm"
                    onClick={(event) => {
                      event.stopPropagation()
                      setActiveView({ profile: profile.id })
                    }}
                  >
                    {t('backup.manage')}
                  </Chip>
                  <Chip
                    className={busy ? 'cursor-not-allowed opacity-50' : 'cursor-pointer'}
                    color="accent"
                    size="sm"
                    onClick={(event) => {
                      event.stopPropagation()
                      if (!busy)
                        openCloneDialog(profile)
                    }}
                  >
                    {t('profiles.clone')}
                  </Chip>
                  <Chip
                    className={busy ? 'cursor-not-allowed opacity-50' : 'cursor-pointer'}
                    variant="primary"
                    color="danger"
                    size="sm"
                    onClick={(event) => {
                      event.stopPropagation()
                      if (busy)
                        return
                      if (profile.default) {
                        onReset(profile.id)
                        return
                      }
                      onRemove(profile.id)
                    }}
                  >
                    {profile.default ? t('profiles.reset') : t('profiles.remove')}
                  </Chip>
                </>
              )}
            />
          ))}
          <If
            cond={!creating}
            else={(
              <div className="flex flex-col gap-1">
                <div className="flex items-center gap-2 px-1">
                  <Input
                    autoFocus
                    variant="secondary"
                    className="h-8 flex-1"
                    placeholder={t('profiles.name_placeholder')}
                    value={name}
                    onChange={e => setName(e.target.value)}
                    onKeyDown={(e) => {
                      if (e.key === 'Enter')
                        commitCreate()
                    }}
                  />
                  <Button size="sm" variant="tertiary" className="h-8" onPress={cancelCreate}>
                    {t('profiles.create_cancel')}
                  </Button>
                  <Button
                    size="sm"
                    variant="primary"
                    className="h-8"
                    isDisabled={!name.trim() || createNameHint !== null || busy}
                    onPress={commitCreate}
                  >
                    {t('profiles.create_confirm')}
                  </Button>
                </div>
                <If
                  cond={createNameHint !== null}
                  then={(
                    <p className="px-1 text-xs text-muted">{createNameHint}</p>
                  )}
                />
              </div>
            )}
          >
            <Button
              onClick={startCreate}
              variant="tertiary"
              className="flex w-full"
              isDisabled={busy}
            >
              <Plus className="size-3.5" />
              <span>{t('profiles.new_profile')}</span>
            </Button>
          </If>
        </div>
      </Panel.Loadable>
      {dialogHolder}
      {migrateDialogHolder}

      <AlertDialog
        isOpen={cloning !== null}
        onOpenChange={(open) => {
          if (!open)
            setCloning(null)
        }}
      >
        <AlertDialog.Backdrop>
          <AlertDialog.Container>
            <AlertDialog.Dialog className="sm:max-w-[400px]">
              <AlertDialog.CloseTrigger />
              <AlertDialog.Header>
                <AlertDialog.Icon status="accent" />
                <AlertDialog.Heading>{t('profiles.clone_dialog_title')}</AlertDialog.Heading>
              </AlertDialog.Header>
              <AlertDialog.Body>
                <p className="text-xs text-muted">
                  {cloning && t('profiles.clone_dialog_desc', { name: cloning.sourceName })}
                </p>
                <Input
                  autoFocus
                  variant="secondary"
                  className="h-8 w-full my-2"
                  placeholder={t('profiles.clone_name_placeholder')}
                  value={cloneName}
                  onChange={e => setCloneName(e.target.value)}
                  onKeyDown={(e) => {
                    if (e.key === 'Enter')
                      commitClone()
                  }}
                />
                <If
                  cond={cloneNameHint !== null}
                  then={(
                    <p className="mb-1 text-xs text-muted">{cloneNameHint}</p>
                  )}
                />
                {cloning && (
                  <p className="text-xs text-muted">
                    {t('profiles.clone_default_hint', { name: suggestCloneName(cloning.sourceId) })}
                  </p>
                )}
              </AlertDialog.Body>
              <AlertDialog.Footer>
                <Button variant="tertiary" onPress={() => setCloning(null)}>
                  {t('profiles.clone_cancel')}
                </Button>
                <Button
                  variant="primary"
                  isDisabled={!cloneName.trim() || cloneNameHint !== null || busy}
                  onPress={commitClone}
                >
                  {busy ? t('profiles.clone_cloning') : t('profiles.clone_confirm')}
                </Button>
              </AlertDialog.Footer>
            </AlertDialog.Dialog>
          </AlertDialog.Container>
        </AlertDialog.Backdrop>
      </AlertDialog>
    </div>
  )
}
