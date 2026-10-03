import type { ClientContext } from 'dsh-tauri/client'
import { PLUGIN_ID } from '../shared/constants'
import { registerMobilePreferences } from './register/preferences'
import { registerStyles } from './register/styles'

export const name = PLUGIN_ID
export const inject = ['slots', 'locale']

export function apply(ctx: ClientContext): void {
  ctx.effect(registerStyles, `${PLUGIN_ID}: styles`)
  ctx.effect(registerMobilePreferences, `${PLUGIN_ID}: preferences`)
}
