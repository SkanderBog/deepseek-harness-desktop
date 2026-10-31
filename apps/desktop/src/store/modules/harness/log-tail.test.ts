import { invoke } from '@tauri-apps/api/core'
import { afterEach, expect, it, vi } from 'vitest'
import { readServiceLogTail } from './utils'

vi.mock('@tauri-apps/api/core', () => ({ invoke: vi.fn() }))
afterEach(() => vi.resetAllMocks())

it('服务日志尾部剥离 ANSI 颜色、裁剪空白并丢弃空行', async () => {
  vi.mocked(invoke).mockResolvedValue('  \u001B[31mERROR 服务启动失败\u001B[0m  \r\n\u001B[0m\n  plain <text> & detail \n')
  expect(await readServiceLogTail()).toEqual(['ERROR 服务启动失败', 'plain <text> & detail'])
  expect(invoke).toHaveBeenCalledWith('read_service_logs', { maxBytes: 16 * 1024 })
})
