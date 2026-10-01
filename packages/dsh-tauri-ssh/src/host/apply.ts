import type z from 'schemastery'
import type { Config as SshRemoteConfig } from './config/schema'
import type { SshHostContext } from './types/index'
import { SSH_PLUGIN_NAME } from '../shared/constants'
import { clearHostRuntime, setCurrentHostInstance, setHostConfig, setMachineDeps, setSyncDeps } from './config/runtime'
import { ConfigSchema } from './config/schema'
import { routes } from './routes'
import { machine } from './service/machine'
import { transport } from './service/transport'
import { packSkills, profileAllowlistReader, profileDependenciesReader, skillRootsScanner } from './utils/local'

const SSH_START_EFFECT = `${SSH_PLUGIN_NAME}: start`

const SSH_ROUTES_EFFECT = `${SSH_PLUGIN_NAME}: routes`

const SSH_RUNTIME_EFFECT = `${SSH_PLUGIN_NAME}: host runtime`

export const name = SSH_PLUGIN_NAME

export const inject = ['webServer', 'connection']

export const Config: z<SshRemoteConfig> = ConfigSchema

export function apply(ctx: SshHostContext, config: SshRemoteConfig): void {
  setCurrentHostInstance(ctx)
  setHostConfig(config)
  setMachineDeps({
    transport,
    emitStatus: () => {}, // keep: 进度出口注入点，故意留空——状态另经 status() 与事件环读取
    localAllowlist: profileAllowlistReader(),
  })
  setSyncDeps({
    profileDependencies: profileDependenciesReader(),
    scanSkills: skillRootsScanner(),
    packSkills,
    ...config.installTimeoutMs === undefined ? {} : { commandTimeoutMs: config.installTimeoutMs },
  })

  ctx.effect(() => {
    void machine.start().catch(() => undefined)
  }, SSH_START_EFFECT)

  ctx.effect(() => routes(ctx), SSH_ROUTES_EFFECT)

  ctx.effect(() => () => {
    void machine.dispose()
    clearHostRuntime()
  }, SSH_RUNTIME_EFFECT)
}

export default { apply, Config, inject, name }
