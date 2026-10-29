import type { PetRef, PetRenderMotion } from 'dsh-pet-component'
import type { MouseEvent } from 'react'
import { useEventListener } from '@reause/core'
import { invoke } from '@tauri-apps/api/core'
import { Pet, useControllablePet } from 'dsh-pet-component'
import { useRef } from 'react'
import { If } from 'react-if-lite'
import { useOmitIgnoreCursorEvents } from '@/hooks/use-omit-ignore-cursor-events'
import { useWindowDraggable } from '@/hooks/use-window-draggable'
import { Hint } from '@/ui/pet/hint'
import { useWakelockRelease } from '../hooks/use-wakelock-release'
import { PET_BASE_WIDTH, PET_DSH_ASPECT } from './constants'
import { useBubbleTracker } from './hooks/use-bubble-tracker'
import { usePetPhysics } from './hooks/use-pet-physics'
import { usePetSource } from './hooks/use-pet-source'
import { normalizeSizePercent, usePetStatus } from './hooks/use-pet-status'
import { usePetWindowSize } from './hooks/use-pet-window'
import { usePetWindowClamp } from './hooks/use-pet-window-clamp'
import { reportPetIssue } from './utils/log'

export function App() {
  const petRef = useRef<PetRef>(null)
  const pet = useControllablePet(petRef)
  const status = usePetStatus()
  const activedPet = status?.active_pet ?? ''
  const { source, error } = usePetSource(activedPet)
  const hitboxRef = useRef<HTMLDivElement>(null)
  const { onFling, onGrab, onRelease } = usePetPhysics(pet, source?.kind, status?.throw_enabled === true)
  const { clampPosition, refreshWorkAreas } = usePetWindowClamp(pet, source?.kind)
  const draggable = useWindowDraggable({ onGrab: handleGrab, onRelease, clampPosition })

  function handleGrab(): void {
    refreshWorkAreas()
    onGrab()
  }

  const visible = status === null || (status.enabled !== false && status.visible !== false)
  const width = (source?.width ?? PET_BASE_WIDTH) * normalizeSizePercent(status?.pet_size) / 100

  useBubbleTracker(pet, source)
  usePetWindowSize(width, source?.aspect ?? PET_DSH_ASPECT, visible)
  useOmitIgnoreCursorEvents(hitboxRef, draggable.dragging)
  useEventListener('contextmenu', event => event.preventDefault())
  useWakelockRelease()

  function handleDoubleClick(): void {
    void invoke('pet_show_main_window').catch(error => reportPetIssue('show main window', error))
  }

  function handleContextMenu(event: MouseEvent<HTMLDivElement>): void {
    event.preventDefault()
    void invoke('pet_show_context_menu').catch(error => reportPetIssue('context menu', error))
  }

  const motion: PetRenderMotion | undefined = draggable.dragging
    ? (draggable.direction === undefined ? undefined : `moving-${draggable.direction}`)
    : undefined

  return (
    <main className={`pointer-events-none fixed inset-0 flex items-end justify-center ${visible ? '' : 'invisible'}`}>
      {source && (
        <Pet
          ref={petRef}
          kind={source.kind}
          config={source.config}
          uri={source.uri}
          ext={source.ext}
          motion={source?.kind === 'codex' ? motion : undefined}
          size={width}
          dragging={draggable.dragging}
          onFling={onFling}
          onDoubleClick={handleDoubleClick}
          onContextMenu={handleContextMenu}
          cache={source.cache}
          hidden={!visible}
          hitboxRef={hitboxRef}
          onHitboxPointerDown={draggable.onPointerDown}
          onHitboxPointerUp={draggable.onPointerUp}
          onError={reportPetAssetError}
        />
      )}
      <If cond={error !== null} then={<Hint petId={activedPet} />} />
    </main>
  )
}

function reportPetAssetError(error: unknown): void {
  reportPetIssue('asset', error)
}
