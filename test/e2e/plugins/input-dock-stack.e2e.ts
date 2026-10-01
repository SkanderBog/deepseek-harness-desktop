import type { Browser, Page } from 'playwright'
import { chromium } from 'playwright'
import { afterAll, beforeAll, describe, expect, it } from 'vitest'
import globalStyle from '../../../packages/dsh-tauri-ui/src/client/styles/global.cssr'

let browser: Browser
let page: Page

beforeAll(async () => {
  browser = await chromium.launch()
  page = await browser.newPage({ viewport: { width: 1000, height: 800 } })
})

afterAll(async () => {
  await browser?.close()
})

async function render(count = 3, anchor = false): Promise<void> {
  await page.mouse.move(0, 0)
  await page.setContent(`<style>
    body { margin:0; --dsh-composer-card-max-width:600px; --dsh-composer-side-clearance:24px }
    .seat { position:absolute; bottom:50px; left:0; width:1000px }
    [data-slot="conversation.input.dock"] { display:flex; flex-direction:column; align-items:center }
    [data-card] { width:100%; box-sizing:border-box; background:#eee; border:1px solid #aaa }
    [data-card] button { height:62px; display:block }
    .composer { width:600px; height:100px; margin:8px auto 0 }
    ${globalStyle.render()}
  </style><div class="seat"><div data-slot="conversation.input.dock" style="display:contents">${Array.from({ length: count }, (_, index) => `${anchor && index === 1 ? '<div data-dsh-tauri-worktree-mode-anchor></div>' : ''}<div data-card="${index}"><button>card</button></div>`).join('')}</div><div class="composer"></div></div>`)
}

async function heights(): Promise<number[]> {
  return page.locator('[data-card]').evaluateAll(elements => elements.map(element => Math.round(element.getBoundingClientRect().height * 100) / 100))
}

describe('input dock hover geometry', () => {
  it('keeps both side gutters collapsed', async () => {
    await render()
    const last = await page.locator('[data-card="2"]').boundingBox()
    expect(last, '末项必须存在').not.toBeNull()
    for (const x of [50, 950]) {
      await page.mouse.move(x, last!.y + 10)
      expect(await heights(), '两侧空白不得展开卡片').toEqual([11.52, 11.76, 64])
    }
  })

  it('animates real layout height and stays open on the uppermost card', async () => {
    await render()
    const dock = page.locator('[data-slot="conversation.input.dock"]')
    expect(await dock.evaluate(element => element.getBoundingClientRect().height), '收起时不得保留完整卡片占位').toBe(88)
    await page.locator('[data-card="2"]').hover()
    await expect.poll(() => dock.evaluate(element => element.getBoundingClientRect().height), { message: '展开高度必须完成过渡' }).toBe(192)
    await page.locator('[data-card="0"] button').hover()
    expect(await heights(), '移动到最上层不能回缩').toEqual([64, 64, 64])
    expect(await page.locator('[data-card="0"]').evaluate(element => getComputedStyle(element).transitionDuration), '必须有过渡动画').toBe('0.22s, 0.22s')
    await page.mouse.move(0, 0)
    await expect.poll(() => dock.evaluate(element => element.getBoundingClientRect().height), { message: '离开后布局必须重新收紧' }).toBe(88)
  })

  it('keeps the dock expanded while a card has keyboard focus', async () => {
    await render()
    await page.locator('[data-card="0"] button').focus()
    await expect.poll(() => heights(), { message: '键盘焦点所在卡片必须完整展开' }).toEqual([64, 64, 64])
  })

  it('does not stack two cards even when an anchor is present', async () => {
    await render(2, true)
    expect(await heights(), 'anchor 不计入堆叠门槛').toEqual([64, 64])
  })

  it('honors reduced motion without disabling expansion', async () => {
    await page.emulateMedia({ reducedMotion: 'reduce' })
    try {
      await render()
      expect(await page.locator('[data-card="0"]').evaluate(element => getComputedStyle(element).transitionDuration), '减少动态效果时必须关闭动画').toBe('0s')
      await page.locator('[data-card="2"]').hover()
      expect(await heights(), '关闭动效不能影响展开').toEqual([64, 64, 64])
    }
    finally {
      await page.emulateMedia({ reducedMotion: 'no-preference' })
    }
  })
})
