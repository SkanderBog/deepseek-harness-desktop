import { defineService } from 'dsh-tauri'
import { runs } from './runs'

const SCHEDULER_INTERRUPTED_ERROR = 'host_interrupted'

export const recovery = defineService({
  async recover(): Promise<void> {
    for (const run of (await runs.list()).filter(run => run.status === 'running')) {
      await runs.save({
        ...run,
        status: 'interrupted',
        finishedAt: new Date().toISOString(),
        error: run.error || SCHEDULER_INTERRUPTED_ERROR,
      })
    }
  },
})
