import type { Page } from 'playwright'
import process from 'node:process'
import { beforeAll, describe, expect, it } from 'vitest'
import { ASSEMBLY_TIMEOUT_MS, collectPageErrors, completePreinstall, installErrorCollector, invoke, pageErrors, SHELL_ROOT, shellAppScope, WINDOW_TIMEOUT_MS } from '../setup/shell'

interface RuntimeInfo {
  app_version: string
  node_version: string
  service_url: string
  data_dir: string
  log_path: string
  platform: string
  arch: string
}

interface AppConfig {
  active_profile: string
}

interface HealthCheck {
  status: string
  service_url: string
}

const app = shellAppScope()

beforeAll(async () => {
  await completePreinstall(app.get().window.page, ASSEMBLY_TIMEOUT_MS)
}, ASSEMBLY_TIMEOUT_MS)

async function waitHealthy(page: Page): Promise<HealthCheck> {
  let health: HealthCheck | undefined
  await expect.poll(async () => {
    try {
      health = await invoke<HealthCheck>(page, 'proxy_health_check')
      return health.status.startsWith('healthy - ')
    }
    catch (error) {
      if (String(error).includes('HARNESS_NOT_READY'))
        return false
      throw error
    }
  }, { timeout: WINDOW_TIMEOUT_MS, message: 'Harness 核心未达到就绪状态' }).toBe(true)
  return health!
}

describe('desktop boot and profile lifecycle', () => {
  it('boots an isolated Electron shell with a ready Harness runtime', async () => {
    const shell = app.get()
    await shell.window.page.context().addInitScript(collectPageErrors)
    await installErrorCollector(shell.window.page)
    await expect.poll(() => shell.window.page.locator(SHELL_ROOT).isVisible(), { timeout: WINDOW_TIMEOUT_MS, message: 'Electron 壳层必须完成首屏渲染' }).toBe(true)

    const health = await waitHealthy(shell.window.page)
    const info = await invoke<RuntimeInfo>(shell.window.page, 'get_runtime_info')
    expect(info.app_version, '运行时必须返回桌面应用版本').toBeTruthy()
    expect(info.node_version, '运行时必须返回 Node 版本').toBeTruthy()
    expect(info.service_url, '运行时必须返回 Harness 服务地址').toBe(health.service_url)
    expect(info.data_dir, '运行时必须返回隔离数据目录').toContain(shell.home)
    expect(info.log_path, '运行时必须返回服务日志路径').toContain(shell.home)
    expect(info.platform, '运行时必须返回平台').toBe(process.platform === 'win32' ? 'windows' : process.platform === 'darwin' ? 'macos' : 'linux')
    expect(info.arch, '运行时必须返回架构').toBeTruthy()
    expect(await pageErrors(shell.window.page), '启动过程不应产生页面异常').toEqual([])
  })

  it('reuses same-profile cores and isolates cloned-profile windows', async () => {
    const shell = app.get()
    const main = shell.window.page
    await waitHealthy(main)
    const originalConfig = await invoke<AppConfig>(main, 'get_app_config')
    const originalRuntime = await invoke<RuntimeInfo>(main, 'get_runtime_info')

    let cloneId: string | undefined
    let second: Awaited<ReturnType<typeof shell.openWindow>> | undefined
    let third: Awaited<ReturnType<typeof shell.openWindow>> | undefined
    try {
      const clone = await invoke<{ id: string }>(main, 'clone_profile', {
        sourceId: originalConfig.active_profile,
        name: `e2e-profile-${process.pid}-${Date.now()}`,
      })
      cloneId = clone.id

      second = await shell.openWindow()
      expect((await invoke<AppConfig>(second.page, 'get_app_config')).active_profile, '同档案窗口必须继承发起窗口档案').toBe(originalConfig.active_profile)
      expect((await invoke<RuntimeInfo>(second.page, 'get_runtime_info')).service_url, '同档案窗口必须复用核心服务地址').toBe(originalRuntime.service_url)
      await waitHealthy(second.page)

      await invoke(second.page, 'set_active_profile', { id: cloneId })
      await invoke(second.page, 'launch_harness')
      const clonedRuntime = await waitHealthy(second.page)
      expect((await invoke<AppConfig>(second.page, 'get_app_config')).active_profile, '切换后窗口必须绑定克隆档案').toBe(cloneId)
      expect(clonedRuntime.service_url, '克隆档案必须使用独立核心服务地址').not.toBe(originalRuntime.service_url)

      third = await shell.openWindow('create_app_window', second)
      expect((await invoke<AppConfig>(third.page, 'get_app_config')).active_profile, '克隆档案窗口必须继承克隆档案').toBe(cloneId)
      expect((await invoke<RuntimeInfo>(third.page, 'get_runtime_info')).service_url, '克隆档案窗口必须复用克隆核心').toBe(clonedRuntime.service_url)
      await waitHealthy(third.page)

      await shell.closeWindow(third.label)
      third = undefined
      await waitHealthy(second.page)

      await shell.closeWindow(second.label)
      second = undefined
      expect((await waitHealthy(main)).service_url, '关闭克隆档案窗口不得影响原档案核心').toBe(originalRuntime.service_url)
    }
    finally {
      try {
        if (third)
          await shell.closeWindow(third.label)
      }
      finally {
        try {
          if (second) {
            try {
              await invoke(second.page, 'set_active_profile', { id: originalConfig.active_profile })
            }
            finally {
              await shell.closeWindow(second.label)
            }
          }
        }
        finally {
          if (cloneId)
            await invoke(main, 'remove_profile', { id: cloneId })
        }
      }
    }
  })
})
