import type { CodexPetConfig, DshPetConfig, PetConfig } from 'dsh-pet-component'
import { useUnmount, useWatch } from '@reause/core'
import { convertFileSrc, invoke } from '@tauri-apps/api/core'
import { useRef, useState } from 'react'
import { useListen } from '@/hooks/use-listen'
import { PET_CODEX_ASPECT, PET_DSH_ASPECT } from '../constants'
import { reportPetIssue } from '../utils/log'

interface PresetPetItem {
  id: string
  name: string
  desc?: string
  image?: string
  kind?: 'dsh' | 'codex'
  size?: number
  config: string
  uri: { default: string, mac?: string }
  ext?: { default: string, mac?: string }
}

type PetAsset = {
  kind: 'codex'
  id: string
  spritesheet: string
  sprite_version_number: number
  columns: number
  rows: number
} | {
  kind: 'dsh'
  id: string
  config: DshPetConfig
  uri: { default: string, mac?: string }
  ext: { default: string, mac?: string }
  size?: number
}

export interface PetSource {
  kind: 'dsh' | 'codex'
  config: string | PetConfig
  uri: string | { default: string, mac?: string }
  ext?: { default: string, mac?: string }
  cache: boolean
  aspect: number
  width: number
}

export interface PetSourceState {
  source: PetSource | null
  error: string | null
}

function presetSource(item: PresetPetItem): PetSource {
  const kind = item.kind ?? 'dsh'
  return {
    kind,
    config: item.config,
    uri: item.uri,
    ext: item.ext,
    cache: true,
    aspect: kind === 'codex' ? PET_CODEX_ASPECT : PET_DSH_ASPECT,
    width: item.size ?? 220,
  }
}

export function assetSource(asset: PetAsset): PetSource {
  if (asset.kind === 'dsh') {
    const uri: PetSource['uri'] = {
      default: `${convertFileSrc(asset.uri.default)}/{name}.{ext}`,
    }
    if (asset.uri.mac !== undefined)
      uri.mac = `${convertFileSrc(asset.uri.mac)}/{name}.{ext}`
    return {
      kind: 'dsh',
      config: asset.config,
      uri,
      ext: asset.ext,
      cache: false,
      aspect: PET_DSH_ASPECT,
      width: asset.size ?? 220,
    }
  }
  const config: CodexPetConfig = {
    spriteVersionNumber: asset.sprite_version_number === 1 ? 1 : 2,
    columns: asset.columns,
    rows: asset.rows,
  }
  return {
    kind: 'codex',
    config,
    uri: asset.spritesheet,
    cache: true,
    aspect: PET_CODEX_ASPECT,
    width: 220,
  }
}

export function usePetSource(activePet: string): PetSourceState {
  const [resolved, setResolved] = useState<{
    id: string
    value: PetSource | null
    error: string | null
  } | null>(null)

  const [catalogRevision, setCatalogRevision] = useState(0)
  useListen<string>('pet://catalog', ({ payload }) => {
    if (payload === activePet)
      setCatalogRevision(revision => revision + 1)
  })

  const requestRef = useRef(0)
  useUnmount(() => {
    requestRef.current++
  })

  useWatch([activePet, catalogRevision], ([id]) => {
    const request = ++requestRef.current
    if (id === '')
      return
    const task = id.includes(':')
      ? invoke<PetAsset>('get_pet_asset', { id }).then(assetSource)
      : invoke<PresetPetItem[]>('list_preset_pets')
          .then(catalog => catalog.find(item => item.id === id))
          .then(item => (item === undefined ? null : presetSource(item)))
    void task
      .then((value) => {
        if (request !== requestRef.current)
          return
        setResolved({
          id,
          value,
          error: value === null ? `PET_NOT_FOUND: ${id}` : null,
        })
      })
      .catch((error) => {
        if (request !== requestRef.current)
          return
        reportPetIssue(`resolve ${id}`, error)
        setResolved({ id, value: null, error: String(error) })
      })
  }, { immediate: true })

  const current = resolved !== null && resolved.id === activePet ? resolved : null
  return {
    source: current?.value ?? null,
    error: current?.error ?? null,
  }
}
