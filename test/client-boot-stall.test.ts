import { describe, expect, it } from 'vitest'
import { readSource } from './setup/read-source'

const constants = readSource('src/store/modules/harness/constants.ts')
const store = readSource('src/store/modules/harness/store.ts')
const iframe = readSource('src/layout/components/iframe.tsx')

describe('客户端 boot 停摆的壳层兜底', () => {
  it('按 iframe 代计时，不依赖帧内室面识别', () => {
    expect(constants).toMatch(/export const CLIENT_BOOT_WATCHDOG_TIMEOUT = \d+/)
    expect(iframe).toContain('CLIENT_BOOT_WATCHDOG_TIMEOUT')
    expect(iframe).toContain('!remoteMode && mounted && harness.serviceHealthy && harness.iframeAlive')
    expect(iframe).toContain('!harness.iframeBootReady')
    expect(iframe).toContain('store.harness.recoverIframeBoot()')
  })

  it('完成确认绑定到具体那一代 iframe，重挂即失效', () => {
    expect(store).toContain('this.iframeBootReadyKey = this.iframeKey')
    expect(store).toMatch(
      /iframeBootReady\(\): boolean \{\s*return this\.iframeBootReadyKey === this\.iframeKey/,
    )
  })

  it('有界重载耗尽后只自动重启一次服务', () => {
    expect(store).toContain('let clientBootRestartSpent = false')
    expect(store).toContain('if (!this.busyAction && consumeClientBootRestart())')
    expect(store).toMatch(/client boot stalled after every bounded reload; restarting the service once/)
  })
})
