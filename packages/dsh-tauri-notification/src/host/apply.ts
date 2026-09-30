import type { HostContext } from 'dsh-tauri'
import { PLUGIN_ID } from '../shared/constants'
import { handleSessionEvent } from './events/session-event'
import { routes } from './routes'
import { clearTurnEndFacts } from './service/turn-end'

export function apply(ctx: HostContext): void {
  ctx.on('session/event', handleSessionEvent)

  ctx.effect(() => routes(ctx), `${PLUGIN_ID}: routes`)
  ctx.effect(() => () => clearTurnEndFacts(), `${PLUGIN_ID}: host runtime`)
}
