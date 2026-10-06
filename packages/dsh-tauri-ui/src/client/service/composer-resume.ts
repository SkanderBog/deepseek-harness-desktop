import type { ComposerRecoveryInput } from './composer-resume.types'
import { postSessionResume } from '../apis'

export async function resumeComposer(input: { sessionId: string, recovery?: ComposerRecoveryInput }): Promise<{ ok: boolean, error?: string }> {
  try {
    if (input.recovery === undefined) {
      await postSessionResume({ sessionId: input.sessionId })
      return { ok: true }
    }
    const childId = await input.recovery.sessions.fork({
      sessionId: input.sessionId,
      atSeq: input.recovery.atSeq,
      increaseTitle: true,
    })
    const opened = input.recovery.navigation.open(childId)
    if (opened.status !== 'opened')
      throw new Error(opened.reason)
    await Promise.resolve(opened.value)
    await postSessionResume({ sessionId: childId, recoverFromSessionId: input.sessionId })
    return { ok: true }
  }
  catch (error) {
    return { ok: false, error: error instanceof Error ? error.message : String(error) }
  }
}
