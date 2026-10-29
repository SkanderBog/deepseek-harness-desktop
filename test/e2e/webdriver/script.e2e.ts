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

  it('connected non-frame elements return no such frame while matching user exceptions remain JavaScript errors', async () => {
    const selector = '[data-testid="dsh-e2e-driver-non-frame"]'
    await browser.execute(() => {
      const element = document.createElement('div')
      element.dataset.testid = 'dsh-e2e-driver-non-frame'
      document.body.append(element)
    })
    try {
      const element = await browser.$(selector)
      const { protocol, hostname, port, path } = browser.options
      const base = `${protocol}://${hostname}:${port}${(path ?? '').replace(/\/+$/, '')}/session/${encodeURIComponent(browser.sessionId)}`
      const response = await fetch(`${base}/frame`, {
        method: 'POST',
        headers: { 'Content-Type': 'application/json' },
        body: JSON.stringify({ id: { 'element-6066-11e4-a52e-4f735466cecf': await element.elementId } }),
        signal: AbortSignal.timeout(30_000),
      })
      expect(response.status, '连接中的普通元素不是帧，必须返回 404').toBe(404)
      expect(await response.json(), '普通元素必须返回 no such frame').toMatchObject({ value: { error: 'no such frame' } })
      const exception = await fetch(`${base}/execute/sync`, {
        method: 'POST',
        headers: { 'Content-Type': 'application/json' },
        body: JSON.stringify({ script: 'throw new Error("element is not a frame")', args: [] }),
        signal: AbortSignal.timeout(30_000),
      })
      expect(exception.status, '用户脚本同文异常必须返回 500').toBe(500)
      expect(await exception.json(), '用户脚本同文异常不能改成帧错误').toMatchObject({ value: { error: 'javascript error', message: 'element is not a frame' } })
      expect(await browser.$(SHELL_ROOT).isExisting(), '错误帧请求不得改变顶层上下文').toBe(true)
    }
    finally {
      await browser.switchFrame(null)
      await browser.execute((selector) => {
        document.querySelector(selector)?.remove()
      }, selector)
    }
  })

  it('async argument failures preserve stale errors and allow the next callback to complete', async () => {
    const { protocol, hostname, port, path } = browser.options
    const base = `${protocol}://${hostname}:${port}${(path ?? '').replace(/\/+$/, '')}/session/${encodeURIComponent(browser.sessionId)}/execute/async`
    const failed = await fetch(base, {
      method: 'POST',
      headers: { 'Content-Type': 'application/json' },
      body: JSON.stringify({ script: 'arguments[arguments.length - 1]("unexpected")', args: [{ 'element-6066-11e4-a52e-4f735466cecf': 'missing-async-element' }] }),
      signal: AbortSignal.timeout(30_000),
    })
    expect(failed.status, '异步参数引用缺失必须立即返回 404').toBe(404)
    expect(await failed.json(), '异步参数引用缺失必须保留 stale 类型').toMatchObject({ value: { error: 'stale element reference' } })
    const completed = await fetch(base, {
      method: 'POST',
      headers: { 'Content-Type': 'application/json' },
      body: JSON.stringify({ script: 'arguments[arguments.length - 1]("completed")', args: [] }),
      signal: AbortSignal.timeout(30_000),
    })
    expect(completed.status, '参数失败后异步回调仍必须完成').toBe(200)
    expect(await completed.json(), '异步成功返回值必须保留').toEqual({ value: 'completed' })
  })

  it('replaced native frames execute scripts in the replacement document', async () => {
    const selector = '[data-testid="dsh-e2e-driver-replacement"]'
    try {
      for (const text of ['first frame', 'replacement frame']) {
        await browser.switchFrame(null)
        await browser.execute((selector, text) => {
          document.querySelector(selector)?.remove()
          const frame = document.createElement('iframe')
          frame.dataset.testid = 'dsh-e2e-driver-replacement'
          frame.srcdoc = `<p>${text}</p>`
          document.body.append(frame)
        }, selector, text)
        await browser.switchFrame(await browser.$(selector))
        await browser.waitUntil(() => browser.execute(expected => document.body?.textContent?.trim() === expected, text))
        expect(await browser.execute(() => document.body.textContent?.trim()), '原生脚本必须在当前帧文档执行').toBe(text)
      }
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
