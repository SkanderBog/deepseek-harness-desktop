import { useMount, useUnmount } from '@reause/core'
import { invoke } from '@tauri-apps/api/core'
import { useRef, useState } from 'react'
import { useListen } from '@/hooks/use-listen'
import { PET_SIZE_DEFAULT_PERCENT, PET_SIZE_MAX_PERCENT, PET_SIZE_MIN_PERCENT } from '../constants'

export interface PetStatus {
  enabled: boolean
  visible: boolean
  active_pet: string
  pet_size?: number | null
  throw_enabled?: boolean | null
}

export function usePetStatus(): PetStatus | null {
  const [status, setStatus] = useState<PetStatus | null>(null)

  const revisionRef = useRef(0)
  useUnmount(() => {
    revisionRef.current++
  })
  useMount(() => {
    const initialRevision = revisionRef.current
    void invoke<PetStatus>('get_pet_status')
      .then((initial) => {
        if (revisionRef.current === initialRevision)
          setStatus(initial)
      })
      .catch((error) => {
        console.warn('[pet] PET_STATUS_LOAD_FAILED:', error)
      })
  })

  useListen<PetStatus>('pet://status', (event) => {
    revisionRef.current++
    setStatus(event.payload)
  })

  return status
}

export function normalizeSizePercent(value: number | null | undefined): number {
  if (typeof value !== 'number' || !Number.isFinite(value))
    return PET_SIZE_DEFAULT_PERCENT
  return Math.min(PET_SIZE_MAX_PERCENT, Math.max(PET_SIZE_MIN_PERCENT, value))
}
