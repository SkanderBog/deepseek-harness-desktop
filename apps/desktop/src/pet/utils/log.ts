import { invoke } from '@tauri-apps/api/core'

export function reportPetIssue(scope: string, detail: unknown): void {
  const message = `[pet] ${scope}: ${formatDetail(detail)}`
  console.warn(message)
  void invoke('log_frontend', {
    level: 'error',
    target: 'pet',
    message: message.slice(0, 4000),
  }).catch(() => {})
}

function formatDetail(detail: unknown): string {
  if (detail instanceof Error)
    return `${detail.name}: ${detail.message}`
  if (typeof detail === 'string')
    return detail
  try {
    return JSON.stringify(detail)
  }
  catch {
    return String(detail)
  }
}
