import type { PetGeometry, PetVelocity, PhysicsParams } from 'dsh-pet-component'

export interface DragSample {
  t: number
  x: number
  y: number
}

export interface ThrowState extends PetVelocity {
  x: number
  y: number
}

export interface ThrowBounds {
  minX: number
  minY: number
  maxX: number
  maxY: number
}

export interface ThrowStep extends ThrowState {
  atRest: boolean
  landed: boolean
  impactSpeed: number
}

export function estimateReleaseVelocity(trail: readonly DragSample[], now: number, throwPower: number): PetVelocity | null {
  const recent = trail.filter(sample => sample.t >= now - 150)
  const first = recent[0]
  const last = recent.at(-1)
  if (!first || !last || now - last.t > 150 || last.t - first.t < 20)
    return null

  const dt = (last.t - first.t) / 1000
  const vx = (last.x - first.x) / dt
  const vy = (last.y - first.y) / dt
  const baseSpeed = Math.hypot(vx, vy)
  if (baseSpeed < 1e-6)
    return null

  const segments: { speed: number, end: number }[] = []
  let previous = first
  for (const sample of recent.slice(1)) {
    if (sample.t - previous.t >= 8) {
      segments.push({
        speed: Math.hypot(sample.x - previous.x, sample.y - previous.y) / (sample.t - previous.t) * 1000,
        end: sample.t,
      })
      previous = sample
    }
  }

  const peak = segments.length > 0 ? Math.max(...segments.map(segment => segment.speed)) : baseSpeed
  const startSegment = segments[0]
  const endSegment = segments.at(-1)
  const acceleration = startSegment && endSegment && segments.length >= 2
    ? (endSegment.speed - startSegment.speed) / Math.max((endSegment.end - startSegment.end) / 1000, 0.02)
    : 0
  const beforeClamp = (baseSpeed * 0.5 + peak * 0.5) * (1 + Math.min(1, Math.max(0, acceleration) / 8000) * 0.6)
  const speed = 3600 * (1 - Math.exp(-beforeClamp / 3600)) * throwPower
  if (speed < 500)
    return null

  return { vx: vx / baseSpeed * speed, vy: vy / baseSpeed * speed }
}

export function bodyBounds(geometry: PetGeometry, width: number, height: number, dsh: boolean): ThrowBounds {
  const feet = dsh ? geometry.height * 330 / 360 : geometry.body.bottom - geometry.y
  return {
    minX: -(geometry.body.left - geometry.x),
    maxX: width - (geometry.body.right - geometry.x),
    minY: 0,
    maxY: height - feet,
  }
}

export function stepThrow(state: ThrowState, delta: number, bounds: ThrowBounds, physics: PhysicsParams): ThrowStep {
  const dt = Math.min(0.05, Math.max(0, delta))
  const maxX = Math.max(bounds.minX, bounds.maxX)
  const maxY = Math.max(bounds.minY, bounds.maxY)
  let { x, y, vx, vy } = state
  let bounced = false

  vy += physics.gravity * dt
  x += vx * dt
  y += vy * dt

  if (maxX === bounds.minX) {
    x = maxX
    vx = 0
  }
  else if (x < bounds.minX) {
    x = bounds.minX
    vx = Math.abs(vx) * physics.restitution
    bounced = true
  }
  else if (x > maxX) {
    x = maxX
    vx = -Math.abs(vx) * physics.restitution
    bounced = true
  }

  if (y < bounds.minY && physics.ceilingBounce) {
    y = bounds.minY
    vy = Math.abs(vy) * physics.restitution
    bounced = true
  }

  const landed = state.y < maxY - 1 && y >= maxY
  const impactSpeed = Math.abs(state.vy)

  if (y >= maxY) {
    y = maxY
    vy = Math.abs(vy) < 40 ? 0 : -Math.abs(vy) * physics.restitution
    vx *= Math.max(0, 1 - physics.groundFriction * dt)
    bounced = true
  }

  if (maxY === bounds.minY) {
    y = maxY
    vy = 0
  }

  const atRest = (y >= maxY - 1 && Math.abs(vx) < 15 && Math.abs(vy) < 1)
    || (bounced && Math.hypot(vx, vy) < 40 && Math.abs(vy) < 1)

  return { x, y, vx, vy, atRest, landed, impactSpeed }
}
