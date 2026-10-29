import type { PetGeometry, PetRef } from 'dsh-pet-component'
import { availableMonitors } from '@tauri-apps/api/window'
import { useEffect, useRef } from 'react'
import { monitorScreenWorkArea, screenCoordinateScale } from '@/utils/screen-coordinates'
import { bodyBounds } from './use-pet-physics.helpers'

export interface PetWorkArea {
  x: number
  y: number
  width: number
  height: number
}

export interface PetWindowPosition {
  x: number
  y: number
}

export function clampPetWindowPosition(
  position: PetWindowPosition,
  geometry: PetGeometry,
  dsh: boolean,
  areas: readonly PetWorkArea[],
  scale: number,
  target: PetWindowPosition = position,
): PetWindowPosition {
  if (areas.length === 0)
    return position
  const area = selectWorkArea(target, geometry, areas, scale)
  const stage = {
    x: area.x / scale,
    y: area.y / scale,
    width: area.width / scale,
    height: area.height / scale,
  }
  const local = bodyBounds(geometry, stage.width, stage.height, dsh)
  return {
    x: clamp(position.x, (stage.x + local.minX - geometry.x) * scale, (stage.x + local.maxX - geometry.x) * scale),
    y: clamp(position.y, (stage.y + local.minY - geometry.y) * scale, (stage.y + local.maxY - geometry.y) * scale),
  }
}

function clamp(value: number, min: number, max: number): number {
  return Math.min(Math.max(value, min), Math.max(min, max))
}

function selectWorkArea(
  position: PetWindowPosition,
  geometry: PetGeometry,
  areas: readonly PetWorkArea[],
  scale: number,
): PetWorkArea {
  const centerX = position.x + ((geometry.body.left + geometry.body.right) / 2) * scale
  const centerY = position.y + ((geometry.body.top + geometry.body.bottom) / 2) * scale
  const containing = areas.find(area =>
    centerX >= area.x
    && centerX < area.x + area.width
    && centerY >= area.y
    && centerY < area.y + area.height)
  if (containing !== undefined)
    return containing
  return areas.reduce((best, area) =>
    distanceSquared(area, centerX, centerY) < distanceSquared(best, centerX, centerY) ? area : best)
}

function distanceSquared(area: PetWorkArea, x: number, y: number): number {
  const right = area.x + area.width
  const bottom = area.y + area.height
  const dx = x < area.x ? area.x - x : x >= right ? x - right + 1 : 0
  const dy = y < area.y ? area.y - y : y >= bottom ? y - bottom + 1 : 0
  return dx * dx + dy * dy
}

function loadWorkAreas(areasRef: { current: readonly PetWorkArea[] }): void {
  void availableMonitors()
    .then((monitors) => { areasRef.current = monitors.map(monitorScreenWorkArea) })
    .catch(() => {})
}

export interface PetWindowClampControls {
  clampPosition: (position: PetWindowPosition, target?: PetWindowPosition) => PetWindowPosition
  refreshWorkAreas: () => void
}

export function usePetWindowClamp(pet: PetRef, kind: 'dsh' | 'codex' | undefined): PetWindowClampControls {
  const areasRef = useRef<readonly PetWorkArea[]>([])
  const kindRef = useRef(kind)
  kindRef.current = kind

  // keep:effect 挂载时先取一次显示器列表，拖拽可能早于下一次抓取刷新
  useEffect(() => {
    loadWorkAreas(areasRef)
  }, [])

  function refreshWorkAreas(): void {
    loadWorkAreas(areasRef)
  }

  function clampPosition(position: PetWindowPosition, target: PetWindowPosition = position): PetWindowPosition {
    const geometry = pet.geometry
    if (geometry === null)
      return position
    return clampPetWindowPosition(
      position,
      geometry,
      kindRef.current === 'dsh',
      areasRef.current,
      screenCoordinateScale(),
      target,
    )
  }

  return { clampPosition, refreshWorkAreas }
}
