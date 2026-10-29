import type { SidebarBusyAction } from '@/store/modules/harness'

export interface RuntimeExitAcceptance {
  serviceHealthy: boolean
  serviceRunning: boolean
  readinessCommitPending: boolean
  busyAction: SidebarBusyAction
  observedToken: number
  currentToken: number
  notOwned: boolean
}

export function shouldAcceptRuntimeExit({
  serviceHealthy,
  serviceRunning,
  readinessCommitPending,
  busyAction,
  observedToken,
  currentToken,
  notOwned,
}: RuntimeExitAcceptance): boolean {
  if (!serviceHealthy && !readinessCommitPending)
    return false
  if (!serviceRunning)
    return false
  if (busyAction === 'shutdown')
    return false
  if (observedToken !== currentToken)
    return false
  return notOwned
}

export function runtimeExitMessageKey(exitCode: number | null | undefined): string {
  if (exitCode == null)
    return 'errors.process_exited_without_code'
  if (isNativeCrashCode(exitCode))
    return 'errors.process_exited_native_crash'
  return 'errors.process_exited_with_code'
}

const NATIVE_CRASH_CODES = new Set([
  0x80000003,
  0xC0000005,
  0xC0000409,
  0xC0000374,
  0xC000001D,
  0xC00000FD,
])

export function isNativeCrashCode(exitCode: number | null | undefined): boolean {
  return exitCode != null && NATIVE_CRASH_CODES.has(exitCode >>> 0)
}
