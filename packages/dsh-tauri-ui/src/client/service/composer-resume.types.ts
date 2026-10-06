export interface ComposerRecoverySessions {
  fork: (options: { sessionId: string, atSeq: number, increaseTitle: boolean }) => Promise<string>
}

export interface ComposerRecoveryNavigation {
  open: (sessionId: string) => { status: 'opened', value: unknown } | { status: 'unavailable', reason: string }
}

export interface ComposerRecoveryInput {
  atSeq: number
  sessions: ComposerRecoverySessions
  navigation: ComposerRecoveryNavigation
}
