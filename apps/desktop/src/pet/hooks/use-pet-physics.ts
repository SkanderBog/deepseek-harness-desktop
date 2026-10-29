import type { PetPhysicsEvent, PetRef, PhysicsParams } from 'dsh-pet-component'
import type { DragSample, ThrowBounds, ThrowState } from './use-pet-physics.helpers'
import { useRafFn } from '@reause/core'
import { invoke } from '@tauri-apps/api/core'
import { currentMonitor, getCurrentWindow } from '@tauri-apps/api/window'
import { type } from '@tauri-apps/plugin-os'
import { useEffect, useRef } from 'react'
import { useListen } from '@/hooks/use-listen'
import { monitorScreenWorkArea, nativeScreenPosition, screenCoordinateScale, windowScreenPosition } from '@/utils/screen-coordinates'
import { reportPetIssue } from '../utils/log'
import { bodyBounds, estimateReleaseVelocity, stepThrow } from './use-pet-physics.helpers'

const TRAIL_LIMIT = 64

export function usePetPhysics(pet: PetRef, kind: 'dsh' | 'codex' | undefined, throwEnabled: boolean): PetPhysicsControls {
  const trailRef = useRef<DragSample[]>([])
  const pressedRef = useRef(false)
  const flightRef = useRef<Flight | null>(null)
  const flightIdRef = useRef(0)
  const kindRef = useRef(kind)
  kindRef.current = kind
  const throwEnabledRef = useRef(throwEnabled)
  throwEnabledRef.current = throwEnabled
  const rafRef = useRef<{ pause: () => void, resume: () => void } | null>(null)

  const raf = useRafFn(({ delta }) => {
    const flight = flightRef.current
    if (flight === null) {
      rafRef.current?.pause()
      return
    }

    const next = stepThrow(flight.state, delta / 1000, flight.bounds, flight.physics)
    flight.state = { x: next.x, y: next.y, vx: next.vx, vy: next.vy }
    void getCurrentWindow()
      .setPosition(nativeScreenPosition(
        Math.round((next.x - flight.originX) * flight.scale),
        Math.round((next.y - flight.originY) * flight.scale),
      ))
      .then(() => next.atRest ? invoke('persist_pet_window_position') : undefined)
      .catch(() => {})

    if (next.landed)
      pet.squash(next.impactSpeed)
    if (next.atRest)
      flightRef.current = null
  }, { immediate: false })
  rafRef.current = raf

  useEffect(() => {
    // keep:effect 拖拽的每帧 setPosition 同样触发 Moved，甩动轨迹以窗口位移为准
    const appWindow = getCurrentWindow()
    const logical = type() === 'macos'
    let disposed = false
    let sampleRevision = 0
    let sampling = false
    let samplePending = false
    let nativeScale: number | undefined
    const unlisteners: Array<() => void> = []

    function setNativeScale(scale: number): void {
      if (nativeScale !== undefined && nativeScale !== scale) {
        trailRef.current = []
        sampleRevision++
      }
      nativeScale = scale
    }

    function appendSample(sample: DragSample): void {
      const samples = trailRef.current
      samples.push(sample)
      if (samples.length > TRAIL_LIMIT)
        samples.splice(0, samples.length - TRAIL_LIMIT)
    }

    function queueSample(): void {
      samplePending = true
      if (sampling)
        return
      sampling = true
      samplePending = false
      const samples = trailRef.current
      const revision = sampleRevision
      void windowScreenPosition(appWindow).then((position) => {
        if (disposed || !pressedRef.current || trailRef.current !== samples || revision !== sampleRevision)
          return
        setNativeScale(position.scaleFactor)
        appendSample({ t: performance.now(), x: position.x, y: position.y })
      }).catch(error => reportPetIssue('fling-position', error)).finally(() => {
        sampling = false
        if (!disposed && pressedRef.current && samplePending)
          queueSample()
      })
    }

    const subscriptions = [appWindow.onMoved((event) => {
      if (!pressedRef.current)
        return
      if (!logical) {
        const scale = window.devicePixelRatio
        appendSample({ t: performance.now(), x: event.payload.x / scale, y: event.payload.y / scale })
        return
      }
      queueSample()
    })]
    if (logical) {
      subscriptions.push(appWindow.onScaleChanged(({ payload }) => {
        setNativeScale(payload.scaleFactor)
      }))
    }
    for (const subscription of subscriptions) {
      void subscription.then((unlisten) => {
        if (disposed)
          unlisten()
        else
          unlisteners.push(unlisten)
      }).catch(error => reportPetIssue('fling-moved', error))
    }

    return () => {
      disposed = true
      stopFlight()
      for (const unlisten of unlisteners)
        unlisten()
    }
  }, [])

  useListen<MouseButtonState>('device-mouse-button', ({ payload }) => {
    if (payload.pressed)
      return
    onRelease()
  })

  function stopFlight(): void {
    flightIdRef.current += 1
    if (flightRef.current === null)
      return
    flightRef.current = null
    rafRef.current?.pause()
  }

  return { onFling, onGrab, onRelease }

  function onGrab(): void {
    pressedRef.current = true
    trailRef.current = []
    stopFlight()
  }

  function onRelease(): void {
    if (!pressedRef.current)
      return
    pressedRef.current = false

    const trail = trailRef.current
    trailRef.current = []
    if (!throwEnabledRef.current)
      return
    const release = estimateReleaseVelocity(trail, performance.now(), 1)
    if (release !== null)
      pet.fling(release)
  }

  function onFling(event: PetPhysicsEvent): void {
    const id = flightIdRef.current + 1
    flightIdRef.current = id
    flightRef.current = null
    rafRef.current?.pause()
    void startFlight(event, id)
  }

  async function startFlight(event: PetPhysicsEvent, id: number): Promise<void> {
    try {
      const [monitor, position] = await Promise.all([currentMonitor(), windowScreenPosition(getCurrentWindow())])
      if (monitor === null || id !== flightIdRef.current)
        return

      const scale = screenCoordinateScale()
      const area = monitorScreenWorkArea(monitor)
      const stage = {
        x: area.x / scale,
        y: area.y / scale,
        width: area.width / scale,
        height: area.height / scale,
      }
      const local = bodyBounds(event.geometry, stage.width, stage.height, kindRef.current === 'dsh')
      const origin = { x: position.x / scale, y: position.y / scale }

      flightRef.current = {
        state: {
          x: origin.x + event.geometry.x,
          y: origin.y + event.geometry.y,
          vx: event.vx * event.physics.throwPower,
          vy: event.vy * event.physics.throwPower,
        },
        physics: event.physics,
        bounds: {
          minX: stage.x + local.minX,
          minY: stage.y + local.minY,
          maxX: stage.x + local.maxX,
          maxY: stage.y + local.maxY,
        },
        originX: event.geometry.x,
        originY: event.geometry.y,
        scale,
      }
      rafRef.current?.resume()
    }
    catch (error) {
      reportPetIssue('fling', error)
    }
  }
}

interface Flight {
  state: ThrowState
  physics: PhysicsParams
  bounds: ThrowBounds
  originX: number
  originY: number
  scale: number
}

interface MouseButtonState {
  pressed: boolean
}

export interface PetPhysicsControls {
  onFling: (event: PetPhysicsEvent) => void
  onGrab: () => void
  onRelease: () => void
}
