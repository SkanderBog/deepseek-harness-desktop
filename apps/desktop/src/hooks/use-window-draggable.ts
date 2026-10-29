import type { PointerEvent as ReactPointerEvent } from 'react'
import { useRafFn, useTimeoutFn } from '@reause/core'
import { invoke } from '@tauri-apps/api/core'
import { getCurrentWindow } from '@tauri-apps/api/window'
import { useRef, useState } from 'react'
import { useListen } from '@/hooks/use-listen'
import { nativeScreenPosition, screenCoordinateScale, windowScreenPosition } from '@/utils/screen-coordinates'

type DragDirection = 'left' | 'right'

interface DeviceMousePosition {
  x: number
  y: number
}

interface MouseButtonState {
  pressed: boolean
}

export interface UseWindowDraggableResult {
  dragging: boolean
  direction: DragDirection | undefined
  onPointerDown: (event: ReactPointerEvent<HTMLDivElement>) => void
  onPointerUp: (event: ReactPointerEvent<HTMLDivElement>) => void
}

export interface UseWindowDraggableOptions {
  clampPosition: (position: { x: number, y: number }, target: { x: number, y: number }) => { x: number, y: number }
  onGrab?: () => void
  onRelease?: () => void
}

const DRAG_START_THRESHOLD = 5
const DRAG_DIRECTION_THRESHOLD = 3
const FOLLOW_STIFFNESS = 200
const FOLLOW_DAMPING = 30
const MAX_FRAME_DELTA = 0.05
const MAX_SUBSTEP = 1 / 120
const SET_POSITION_INTERVAL = 16
const DRAG_DIRECTION_IDLE_TIMEOUT = 350
const DRAG_SESSION_TIMEOUT = 1500

interface Gesture {
  pointerId: number
  anchor: { x: number, y: number } | undefined
  base: { x: number, y: number } | undefined
  x: number
  y: number
  vx: number
  vy: number
  engaged: boolean
  directionAnchor: { x: number, y: number } | undefined
}

export function useWindowDraggable(options: UseWindowDraggableOptions): UseWindowDraggableResult {
  const onGrabRef = useRef(options.onGrab)
  onGrabRef.current = options.onGrab

  const onReleaseRef = useRef(options.onRelease)
  onReleaseRef.current = options.onRelease

  const clampRef = useRef(options.clampPosition)
  clampRef.current = options.clampPosition
  const gestureRef = useRef<Gesture | null>(null)
  const cursorRef = useRef<{ x: number, y: number } | undefined>(undefined)
  const pressedRef = useRef<boolean | undefined>(undefined)
  const rafRef = useRef<{ pause: () => void, resume: () => void } | null>(null)
  const appliedRef = useRef<{ x: number, y: number } | undefined>(undefined)
  const lastSentAtRef = useRef<number | undefined>(undefined)
  const [dragging, setDragging] = useState(false)
  const [direction, setDirection] = useState<DragDirection | undefined>(undefined)

  const { start: armDirectionTimer, stop: stopDirectionTimer } = useTimeoutFn(
    parkDirection,
    DRAG_DIRECTION_IDLE_TIMEOUT,
    { immediate: false },
  )
  const { start: armSessionTimer, stop: stopSessionTimer } = useTimeoutFn(
    handleSessionTimeout,
    DRAG_SESSION_TIMEOUT,
    { immediate: false },
  )

  const raf = useRafFn(({ delta }) => {
    const gesture = gestureRef.current
    const cursor = cursorRef.current
    if (gesture === null || !gesture.engaged || gesture.anchor === undefined || gesture.base === undefined || cursor === undefined) {
      rafRef.current?.pause()
      return
    }
    const targetX = gesture.base.x + (cursor.x - gesture.anchor.x)
    const targetY = gesture.base.y + (cursor.y - gesture.anchor.y)
    let remaining = Math.min(MAX_FRAME_DELTA, Math.max(0, delta / 1000))

    if (remaining <= 0)
      return
    while (remaining > 0) {
      const dt = Math.min(remaining, MAX_SUBSTEP)
      gesture.vx += ((targetX - gesture.x) * FOLLOW_STIFFNESS - gesture.vx * FOLLOW_DAMPING) * dt
      gesture.vy += ((targetY - gesture.y) * FOLLOW_STIFFNESS - gesture.vy * FOLLOW_DAMPING) * dt
      gesture.x += gesture.vx * dt
      gesture.y += gesture.vy * dt
      remaining -= dt
    }

    const clamped = clampRef.current({ x: gesture.x, y: gesture.y }, { x: targetX, y: targetY })
    gesture.x = clamped.x
    gesture.y = clamped.y

    const nextX = Math.round(gesture.x)
    const nextY = Math.round(gesture.y)
    if (appliedRef.current?.x === nextX && appliedRef.current.y === nextY)
      return
    if (lastSentAtRef.current !== undefined) {
      lastSentAtRef.current += delta
      if (lastSentAtRef.current < SET_POSITION_INTERVAL)
        return
    }
    lastSentAtRef.current = 0
    appliedRef.current = { x: nextX, y: nextY }
    void getCurrentWindow()
      .setPosition(nativeScreenPosition(nextX, nextY))
      .catch(() => {})
  }, { immediate: false })
  rafRef.current = raf

  function parkDirection(): void {
    setDirection(undefined)
  }

  function handleSessionTimeout(): void {
    if (gestureRef.current !== null && pressedRef.current === true) {
      armSessionTimer()
      return
    }
    endDrag()
  }

  function endDrag(): void {
    const gesture = gestureRef.current
    gestureRef.current = null
    pressedRef.current = undefined
    appliedRef.current = undefined
    lastSentAtRef.current = undefined
    stopDirectionTimer()
    stopSessionTimer()
    rafRef.current?.pause()
    setDragging(false)
    setDirection(undefined)
    if (gesture?.engaged === true) {
      void getCurrentWindow()
        .setPosition(nativeScreenPosition(Math.round(gesture.x), Math.round(gesture.y)))
        .then(() => invoke('persist_pet_window_position'))
        .catch(() => {})
    }
    if (gesture !== null)
      onReleaseRef.current?.()
  }

  function handlePointerDown(event: ReactPointerEvent<HTMLDivElement>): void {
    if (event.button !== 0 || event.isPrimary === false || gestureRef.current !== null)
      return

    onGrabRef.current?.()
    const latest = cursorRef.current
    const gesture: Gesture = {
      pointerId: event.pointerId,
      anchor: latest === undefined ? undefined : { x: latest.x, y: latest.y },
      base: undefined,
      x: 0,
      y: 0,
      vx: 0,
      vy: 0,
      engaged: false,
      directionAnchor: latest === undefined ? undefined : { x: latest.x, y: latest.y },
    }
    gestureRef.current = gesture

    armSessionTimer()

    void windowScreenPosition(getCurrentWindow()).then((position) => {
      if (gestureRef.current !== gesture)
        return
      gesture.base = { x: position.x, y: position.y }
    }).catch(() => {
      if (gestureRef.current === gesture)
        endDrag()
    })
  }

  function handlePointerUp(event: ReactPointerEvent<HTMLDivElement>): void {
    if (event.button !== 0 || gestureRef.current?.pointerId !== event.pointerId)
      return
    endDrag()
  }

  useListen<DeviceMousePosition>('device-mouse-move', ({ payload }) => {
    cursorRef.current = { x: payload.x, y: payload.y }
    const gesture = gestureRef.current
    if (gesture === null)
      return

    armSessionTimer()
    if (gesture.anchor === undefined) {
      gesture.anchor = { x: payload.x, y: payload.y }
      gesture.directionAnchor = { x: payload.x, y: payload.y }
      return
    }
    const scale = screenCoordinateScale()
    const dx = payload.x - gesture.anchor.x
    const dy = payload.y - gesture.anchor.y
    if (!gesture.engaged) {
      if (Math.hypot(dx, dy) < DRAG_START_THRESHOLD * scale || gesture.base === undefined)
        return
      gesture.x = gesture.base.x
      gesture.y = gesture.base.y
      gesture.vx = 0
      gesture.vy = 0
      gesture.engaged = true
      appliedRef.current = undefined
      lastSentAtRef.current = undefined
      setDragging(true)
      rafRef.current?.resume()
    }
    const last = gesture.directionAnchor
    if (last !== undefined && Math.abs(payload.x - last.x) >= DRAG_DIRECTION_THRESHOLD * scale) {
      setDirection(payload.x > last.x ? 'right' : 'left')
      gesture.directionAnchor = { x: payload.x, y: payload.y }
    }
    armDirectionTimer()
  })

  useListen<MouseButtonState>('device-mouse-button', ({ payload }) => {
    if (payload.pressed) {
      pressedRef.current = true
      return
    }
    pressedRef.current = false
    endDrag()
  })

  return {
    dragging,
    direction,
    onPointerDown: handlePointerDown,
    onPointerUp: handlePointerUp,
  }
}
