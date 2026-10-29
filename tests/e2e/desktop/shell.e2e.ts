import type { AddressInfo } from 'node:net'
import type { Browser, Page } from 'playwright'
import type { ChildProcess } from 'node:child_process'
import { spawn } from 'node:child_process'
import { existsSync } from 'node:fs'
import { mkdir, mkdtemp, rm } from 'node:fs/promises'
import { createServer } from 'node:net'
import { tmpdir } from 'node:os'
import path from 'node:path'
import { fileURLToPath } from 'node:url'
import { chromium } from 'playwright'
import { afterAll, beforeAll, describe, expect, it } from 'vitest'

const ROOT = path.resolve(fileURLToPath(new URL('.', import.meta.url)), '../../..')
const BINARIES = [
  'apps/desktop/src-tauri/target/debug/deepseek-harness-desktop.exe',
  'apps/desktop/src-tauri/target/debug/deepseek-harness-desktop',
  'target/debug/deepseek-harness-desktop.exe',
  'target/debug/deepseek-harness-desktop',
]
const MAIN_ORIGIN = 'tauri://localhost'

let binary: string | undefined
let home = ''
let app: ChildProcess | undefined
let browser: Browser | undefined
let page: Page

function freePort(): Promise<number> {
  return new Promise((resolve, reject) => {
    const server = createServer()
    server.once('error', reject)
    server.listen(0, '127.0.0.1', () => {
      const { port } = server.address() as AddressInfo
      server.close(() => resolve(port))
    })
  })
}

async function waitForEndpoint(port: number): Promise<string> {
  const endpoint = `http://127.0.0.1:${port}`
  const deadline = Date.now() + 120_000
  let lastError = 'no CDP target yet'
  while (Date.now() < deadline) {
    if (app?.exitCode !== null || app?.signalCode) throw new Error(`主进程已退出（code=${app?.exitCode}），${lastError}`)
    try {
      const response = await fetch(`${endpoint}/json/list`, { signal: AbortSignal.timeout(1000) })
      const targets = await response.json() as { type: string, url: string }[]
      if (targets.some(target => target.type === 'page' && target.url.startsWith(MAIN_ORIGIN))) return endpoint
      lastError = `targets=${JSON.stringify(targets.map(target => target.url))}`
    }
    catch (error) {
      lastError = String(error)
    }
    await new Promise(resolve => setTimeout(resolve, 500))
  }
  throw new Error(`等待 Electron CDP 超时：${lastError}`)
}

beforeAll(async () => {
  binary = BINARIES.map(candidate => path.join(ROOT, candidate)).find(existsSync)
  expect(binary, '缺少 debug 产物，先执行 pnpm --filter dsh-desktop build 与 cargo build --manifest-path apps/desktop/src-tauri/Cargo.toml').toBeTruthy()
  home = await mkdtemp(path.join(tmpdir(), 'dsh-e2e-shell-'))
  await mkdir(path.join(home, 'appdata'), { recursive: true })
  const port = await freePort()
  app = spawn(binary!, [], {
    stdio: ['ignore', 'pipe', 'pipe'],
    env: {
      ...process.env,
      DSH_E2E_HOME: home,
      APPDATA: path.join(home, 'appdata'),
      HOME: path.join(home, 'appdata'),
      XDG_CONFIG_HOME: path.join(home, 'appdata'),
      TAURI_WEBDRIVER_PORT: '4445',
      TAURI_RUNTIME_ELECTRON_OWNER: 'dsh-e2e',
      TAURI_RUNTIME_ELECTRON_ARGS: `--remote-debugging-address=127.0.0.1 --remote-debugging-port=${port}`,
    },
  })
  const endpoint = await waitForEndpoint(port)
  browser = await chromium.connectOverCDP(endpoint)
  const main = browser.contexts()[0]?.pages().find(candidate => candidate.url().startsWith(MAIN_ORIGIN))
  expect(main, '未找到主窗口渲染页').toBeTruthy()
  page = main!
})

afterAll(async () => {
  await browser?.close().catch(() => undefined)
  if (app && app.exitCode === null) {
    if (process.platform === 'win32') spawn('taskkill', ['/pid', String(app.pid), '/T', '/F'], { stdio: 'ignore' })
    else app.kill('SIGKILL')
  }
  if (home) await rm(home, { recursive: true, force: true })
})

describe('electron 主窗口', () => {
  it('主窗口渲染出 React 根节点', async () => {
    await expect
      .poll(() => page.evaluate(() => document.querySelectorAll('#root *').length), { message: '主窗口应挂载 React 树', timeout: 60_000 })
      .toBeGreaterThan(0)
  })

  it('Tauri IPC 往返返回真实命令结果', async () => {
    const dev = await page.evaluate(() => (window as unknown as { __TAURI_INTERNALS__: { invoke: (cmd: string) => Promise<boolean> } }).__TAURI_INTERNALS__.invoke('is_dev_build'))
    expect(dev, 'debug 产物应返回 is_dev_build=true').toBe(true)
  })
})
