import type { Page } from 'playwright'
import { describe, expect, it } from 'vitest'
import { completePreinstall, installErrorCollector, invoke, pageErrors, shellAppScope } from './setup/shell'

const ASSEMBLY_TIMEOUT_MS = 900_000
const PET_WINDOW_TIMEOUT_MS = 30_000
const PET_SIZE_MIN = 50
const PET_SIZE_MAX = 200
const PET_SIZE_DEFAULT = 100

interface PetStatus {
  enabled?: boolean
  visible?: boolean
  active_pet?: string
  pet_size?: number | null
}

const shell = shellAppScope()
const page = (): Page => shell.get().window.page

async function status(): Promise<PetStatus> {
  return invoke<PetStatus>(page(), 'get_pet_status')
}

async function setEnabled(enabled: boolean): Promise<void> {
  await invoke(page(), 'set_pet_enabled', { enabled })
  await expect.poll(async () => (await status()).enabled, { message: `把 enabled 写成 ${enabled} 后状态未收敛`, timeout: PET_WINDOW_TIMEOUT_MS }).toBe(enabled)
}

async function expectSizeRejected(outOfRange: number): Promise<void> {
  await expect(invoke(page(), 'set_pet_size', { size: outOfRange })).rejects.toThrow(/PET_SIZE_OUT_OF_RANGE/)
}

describe('桌面端桌宠独立窗口', () => {
  it('TC-PET-L3-02-004 桌宠尺寸边界：范围内接受、越界拒绝且不改状态', async () => {
    await installErrorCollector(page())
    await completePreinstall(page(), ASSEMBLY_TIMEOUT_MS)
    await setEnabled(false)

    await setEnabled(true)

    for (const inRange of [PET_SIZE_MIN, PET_SIZE_MAX]) {
      await invoke(page(), 'set_pet_size', { size: inRange })
      expect((await status()).pet_size, `范围内提交 ${inRange} 必须被接受并落盘`).toBe(inRange)
    }

    const settled = (await status()).pet_size
    for (const outOfRange of [0, PET_SIZE_MIN - 1, PET_SIZE_MAX + 1, 999]) {
      await expectSizeRejected(outOfRange)
      expect(
        (await status()).pet_size,
        `越界提交 ${outOfRange} 不得改动已落盘的尺寸`,
      ).toBe(settled)
    }

    await invoke(page(), 'set_pet_size', { size: PET_SIZE_DEFAULT })
    expect((await status()).pet_size, '收尾必须恢复默认尺寸').toBe(PET_SIZE_DEFAULT)
    await setEnabled(false)

    expect(await pageErrors(page()), 'TC-PET-L3-02-004 尺寸边界：壳层出现报错').toEqual([])
  }, ASSEMBLY_TIMEOUT_MS)
})
