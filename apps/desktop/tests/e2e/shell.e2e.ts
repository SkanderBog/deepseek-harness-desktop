import { describe, expect, it } from 'vitest'
import { page } from './setup/shell'

describe('electron 主窗口', () => {
  it('主窗口渲染出 React 根节点', async () => {
    await expect
      .poll(() => page.evaluate(() => document.querySelectorAll('#root *').length), { message: '主窗口应挂载 React 树', timeout: 60_000 })
      .toBeGreaterThan(0)
  })

  it('Tauri IPC 往返返回真实命令结果', async () => {
    const dev = await page.evaluate(() => window.__TAURI_INTERNALS__.invoke<boolean>('is_dev_build'))
    expect(dev, 'debug 产物应返回 is_dev_build=true').toBe(true)
  })
})
