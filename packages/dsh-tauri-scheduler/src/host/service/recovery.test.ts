import { beforeEach, expect, it, vi } from 'vitest'
import { recovery } from './recovery'
import { runs } from './runs'

vi.mock('./runs', () => ({ runs: { list: vi.fn(), save: vi.fn() } }))

beforeEach(() => vi.clearAllMocks())

it('interrupts only running records and keeps their existing errors', async () => {
  const records = ['running', 'failed', 'queued', 'running'].map((status, index) => ({ id: String(index), status, error: index === 3 ? 'existing' : undefined }))
  vi.mocked(runs.list).mockResolvedValue(records as never)
  await recovery.recover()
  expect(runs.save).toHaveBeenCalledTimes(2)
  expect(runs.save).toHaveBeenNthCalledWith(1, { ...records[0], status: 'interrupted', error: 'host_interrupted', finishedAt: expect.any(String) })
  expect(runs.save).toHaveBeenNthCalledWith(2, { ...records[3], status: 'interrupted', finishedAt: expect.any(String) })
  expect(records[0].status).toBe('running')
})
