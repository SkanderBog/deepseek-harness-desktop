import { afterEach, beforeEach, describe, expect, it } from 'vitest'
import { collectPageErrors, installErrorCollector, invoke, pageErrors, shellAppScope, WINDOW_TIMEOUT_MS } from '../setup/shell'

interface PetStatus {
  enabled: boolean
  pet_size: number | null
}

const app = shellAppScope()

beforeEach(async () => {
  const shell = app.get()
  await shell.browser.contexts()[0].addInitScript(collectPageErrors)
  await installErrorCollector(shell.window.page)
  await invoke(shell.window.page, 'set_pet_enabled', { enabled: false })
  await expect.poll(async () => (await invoke<PetStatus>(shell.window.page, 'get_pet_status')).enabled, { timeout: WINDOW_TIMEOUT_MS }).toBe(false)
  await expect.poll(async () => (await shell.windows()).some(window => window.label === 'pet'), { timeout: WINDOW_TIMEOUT_MS }).toBe(false)
  await invoke(shell.window.page, 'set_pet_size', { size: 100 })
  await expect.poll(async () => (await invoke<PetStatus>(shell.window.page, 'get_pet_status')).pet_size, { timeout: WINDOW_TIMEOUT_MS }).toBe(100)
})

afterEach(async () => {
  const shell = app.get()
  await invoke(shell.window.page, 'set_pet_size', { size: 100 })
  await invoke(shell.window.page, 'set_pet_enabled', { enabled: false })
  await expect.poll(async () => (await shell.windows()).some(window => window.label === 'pet'), { timeout: WINDOW_TIMEOUT_MS }).toBe(false)
})

describe('desktop pet IPC', () => {
  it('accepts inclusive size limits and rejects out-of-range values without changing persisted size', async () => {
    const page = app.get().window.page
    for (const size of [50, 200]) {
      const updated = await invoke<PetStatus>(page, 'set_pet_size', { size })
      expect(updated.pet_size, '范围内提交必须返回指定尺寸').toBe(size)
      expect((await invoke<PetStatus>(page, 'get_pet_status')).pet_size, '范围内提交必须持久化尺寸').toBe(size)
    }

    const previous = (await invoke<PetStatus>(page, 'get_pet_status')).pet_size
    for (const size of [0, 49, 201, 999]) {
      await expect(invoke(page, 'set_pet_size', { size })).rejects.toThrow(/PET_SIZE_OUT_OF_RANGE/)
      expect((await invoke<PetStatus>(page, 'get_pet_status')).pet_size, '越界提交不得修改已保存尺寸').toBe(previous)
    }

    expect(await pageErrors(page), '尺寸 IPC 不应产生页面异常').toEqual([])
  })

  it('creates and destroys the native pet window when its enabled state changes', async () => {
    const shell = app.get()
    const page = shell.window.page
    const windowsBefore = await shell.windows()
    const nativeBefore = await invoke<string[]>(page, 'plugin:window|get_all_windows')
    expect(windowsBefore.map(window => window.label), '隔离实例初始不应有宠物窗口').not.toContain('pet')

    await invoke(page, 'set_pet_enabled', { enabled: true })
    await expect.poll(async () => (await shell.windows()).some(window => window.label === 'pet'), { timeout: WINDOW_TIMEOUT_MS, message: '启用桌宠后应创建真实窗口' }).toBe(true)
    const opened = (await shell.windows()).find(window => window.label === 'pet')!
    expect(opened.page, '宠物必须使用独立 CDP 页面').not.toBe(page)
    await expect.poll(async () => (await invoke<string[]>(page, 'plugin:window|get_all_windows')).sort(), { timeout: WINDOW_TIMEOUT_MS, message: '原生窗口管理器应注册宠物窗口' }).toEqual([...nativeBefore, 'pet'].sort())
    expect((await invoke<PetStatus>(page, 'get_pet_status')).enabled, '启用命令必须保存 enabled=true').toBe(true)
    expect(await pageErrors(page), '启用桌宠不应造成壳层页面异常').toEqual([])
    expect(await pageErrors(opened.page), '宠物窗口加载不应产生页面异常').toEqual([])

    await invoke(page, 'set_pet_enabled', { enabled: false })
    await expect.poll(() => opened.page.isClosed(), { timeout: WINDOW_TIMEOUT_MS, message: '禁用桌宠必须销毁真实页面' }).toBe(true)
    await expect.poll(async () => (await shell.windows()).map(window => window.label), { timeout: WINDOW_TIMEOUT_MS, message: '宠物关闭后壳层窗口集合应恢复' }).toEqual(windowsBefore.map(window => window.label))
    await expect.poll(async () => (await invoke<string[]>(page, 'plugin:window|get_all_windows')).sort(), { timeout: WINDOW_TIMEOUT_MS, message: '原生窗口管理器应注销宠物窗口' }).toEqual(nativeBefore.sort())
    expect((await invoke<PetStatus>(page, 'get_pet_status')).enabled, '禁用命令必须保存 enabled=false').toBe(false)
    expect(await pageErrors(page), '禁用桌宠不应产生壳层页面异常').toEqual([])
  })
})
