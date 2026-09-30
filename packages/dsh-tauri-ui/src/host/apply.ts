import type { HostContext } from './types'
import { PLUGIN_ID } from '../shared/constants'
import { setCurrentHostInstance } from './config/runtime'
import { handlePreStep } from './events/pre-step'
import { routes } from './routes'

const ROUTES_EFFECT = `${PLUGIN_ID}: routes`

const RUNTIME_EFFECT = `${PLUGIN_ID}: host runtime`

export function apply(ctx: HostContext): void {
  setCurrentHostInstance(ctx)

  ctx.on('agent/pre-step', handlePreStep)

  ctx.effect(() => routes(ctx), ROUTES_EFFECT)
  ctx.effect(() => () => setCurrentHostInstance(undefined), RUNTIME_EFFECT)
}
