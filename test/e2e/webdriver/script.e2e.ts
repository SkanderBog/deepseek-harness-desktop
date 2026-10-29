import type { DesktopApp } from '../support/desktop'
import { afterAll, beforeAll, describe, expect, it } from 'vitest'
import { startDesktopApp } from '../support/desktop'
import { SHELL_ROOT } from '../support/selectors'

describe('Windows WebDriver script integration', () => {
  let app: DesktopApp | undefined
  let browser: WebdriverIO.Browser

  beforeAll(async () => {
    app = await startDesktopApp()
    browser = app.browser
    await browser.$(SHELL_ROOT).waitForExist()
  })

  afterAll(async () => {
    await app?.stop()
  })

  it('detached native frame references return stale element errors and preserve the top-level context', async () => {
    const selector = '[data-testid="dsh-e2e-driver-frame"]'
    await browser.execute(() => {
      const frame = document.createElement('iframe')
      frame.dataset.testid = 'dsh-e2e-driver-frame'
      document.body.append(frame)
    })
    try {
      const frame = await browser.$(selector)
      const id = await frame.elementId
      await browser.execute((selector) => {
        document.querySelector(selector)!.remove()
      }, selector)
      const { protocol, hostname, port, path } = browser.options
      const base = `${protocol}://${hostname}:${port}${(path ?? '').replace(/\/+$/, '')}/session/${encodeURIComponent(browser.sessionId)}`
      for (const [route, body] of [
        [`element/${encodeURIComponent(id)}/name`, undefined],
        ['frame', { id: { 'element-6066-11e4-a52e-4f735466cecf': id } }],
      ] as const) {
        const response = await fetch(`${base}/${route}`, {
          method: body ? 'POST' : 'GET',
          headers: { 'Content-Type': 'application/json' },
          body: body ? JSON.stringify(body) : undefined,
          signal: AbortSignal.timeout(30_000),
        })
        expect(response.status, `失效帧 ${route} 必须返回 404`).toBe(404)
        expect(await response.json(), `失效帧 ${route} 必须保留错误类型`).toMatchObject({ value: { error: 'stale element reference' } })
      }
      expect(await browser.$(SHELL_ROOT).isExisting(), '失效帧请求不得破坏顶层上下文').toBe(true)
    }
    finally {
      await browser.switchFrame(null)
      await browser.execute((selector) => {
        document.querySelector(selector)?.remove()
      }, selector)
    }
  })

  it('element properties containing the old error marker remain successful object results', async () => {
    const selector = '[data-testid="dsh-e2e-driver-property"]'
    await browser.execute(() => {
      const element = document.createElement('div')
      element.dataset.testid = 'dsh-e2e-driver-property'
      Object.assign(element, { driverResult: { __wdio_script_error: 'user data' } })
      document.body.append(element)
    })
    try {
      const element = await browser.$(selector)
      const { protocol, hostname, port, path } = browser.options
      const response = await fetch(`${protocol}://${hostname}:${port}${(path ?? '').replace(/\/+$/, '')}/session/${encodeURIComponent(browser.sessionId)}/element/${encodeURIComponent(await element.elementId)}/property/driverResult`, {
        signal: AbortSignal.timeout(30_000),
      })
      expect(response.status, '包含旧异常字段的属性对象必须正常返回').toBe(200)
      expect(await response.json(), '属性值必须完整保留').toEqual({ value: { __wdio_script_error: 'user data' } })
    }
    finally {
      await browser.execute((selector) => {
        document.querySelector(selector)?.remove()
      }, selector)
    }
  })
})
