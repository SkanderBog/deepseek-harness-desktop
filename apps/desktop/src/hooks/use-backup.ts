import { useMutation, useQuery, useQueryClient } from '@tanstack/react-query'
import { invoke } from '@tauri-apps/api/core'
import { queryKeys } from '@/config/query-keys'
import { useInvalidateOnSettingUpdated } from '@/hooks/use-invalidate-on-setting-updated'

export interface BackupInfo {
  timestamp: string
  path: string
  size: number
  includeCredentials: boolean
}

export interface UseBackupsResult {
  backups: BackupInfo[]
  loading: boolean
  error: string
  createBackup: (includeCredentials: boolean) => Promise<BackupInfo>
  restoreBackup: (timestamp: string, asNew: boolean) => Promise<void>
  deleteBackup: (timestamp: string) => Promise<void>
  busy: boolean
  creating: boolean
  restoring: boolean
  deleting: boolean
  exporting: boolean
  recoveryBackup: BackupInfo | undefined
  exportRecoveryBackup: (includeCredentials: boolean) => Promise<BackupInfo>
}

export function useBackups(): UseBackupsResult {
  const queryClient = useQueryClient()

  const { data, isLoading, error, refetch } = useQuery({
    queryKey: queryKeys.backups,
    queryFn: () => invoke<BackupInfo[]>('list_backups'),
  })

  useInvalidateOnSettingUpdated(queryKeys.backups)

  function invalidate() {
    void queryClient.invalidateQueries({ queryKey: queryKeys.backups })
  }

  const create = useMutation({
    mutationFn: (includeCredentials: boolean) =>
      invoke<BackupInfo>('backup_profile', { includeCredentials }),
    onSuccess: invalidate,
  })
  const restore = useMutation({
    mutationFn: ({ timestamp, asNew }: { timestamp: string, asNew: boolean }) =>
      invoke<void>('restore_profile', { timestamp, asNew }),
    onSuccess: invalidate,
  })
  const remove = useMutation({
    mutationFn: (timestamp: string) => invoke<void>('delete_backup', { timestamp }),
    onSuccess: invalidate,
  })
  const recovery = useMutation({
    mutationFn: (includeCredentials: boolean) =>
      invoke<BackupInfo>('export_recovery_backup', { includeCredentials }),
  })
  return {
    backups: data ?? [],
    loading: isLoading,
    error: error ? String(error) : '',
    createBackup: async (includeCredentials) => {
      const created = await create.mutateAsync(includeCredentials)
      await refetch()
      return created
    },
    restoreBackup: async (timestamp, asNew) => {
      await restore.mutateAsync({ timestamp, asNew })
      await refetch()
    },
    deleteBackup: async (timestamp) => {
      await remove.mutateAsync(timestamp)
      await refetch()
    },
    busy: create.isPending || restore.isPending || remove.isPending || recovery.isPending,
    creating: create.isPending,
    restoring: restore.isPending,
    deleting: remove.isPending,
    exporting: recovery.isPending,
    recoveryBackup: recovery.data,
    exportRecoveryBackup: recovery.mutateAsync,
  }
}
