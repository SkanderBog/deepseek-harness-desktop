export const PATCH_PARSE_ERROR_CODE = 'INTERNAL_PLUGIN_PATCH_PARSE_FAILED'
export const PATCH_ENCODING_ERROR_CODE = 'INTERNAL_PLUGIN_PATCH_ENCODING_FAILED'

export function containsPatchLayerParseError(message: string): boolean {
  return message.includes(`${PATCH_PARSE_ERROR_CODE}:`)
}

export function containsPatchLayerEncodingError(message: string): boolean {
  return message.includes(`${PATCH_ENCODING_ERROR_CODE}:`)
}

export function patchLayerErrorDetail(message: string): string {
  for (const code of [PATCH_ENCODING_ERROR_CODE, PATCH_PARSE_ERROR_CODE]) {
    const marker = `${code}:`
    const at = message.indexOf(marker)
    if (at >= 0)
      return message.slice(at + marker.length).trim()
  }
  return ''
}

export const QUARANTINE_FAILED_CODE = 'PATCH_LAYER_QUARANTINE_FAILED'

export function containsQuarantineFailure(message: string): boolean {
  return message.includes(`${QUARANTINE_FAILED_CODE}:`)
}

export function quarantineFailureDetail(message: string): string {
  const marker = `${QUARANTINE_FAILED_CODE}:`
  const at = message.indexOf(marker)
  if (at < 0)
    return ''
  return message.slice(at + marker.length).trim()
}

export const PATCH_ENTRY_UNRESOLVED_CODE = 'PATCH_LAYER_ENTRY_UNRESOLVED'

export interface UnresolvedPatchEntry {
  layer: string
  line: number
  id?: string | null
  name: string
  declared: boolean
}

export function containsPatchEntryUnresolved(message: string): boolean {
  return message.includes(`${PATCH_ENTRY_UNRESOLVED_CODE}:`)
}

export function patchEntryUnresolvedEntries(message: string): UnresolvedPatchEntry[] {
  const marker = `${PATCH_ENTRY_UNRESOLVED_CODE}:`
  const at = message.indexOf(marker)
  if (at < 0)
    return []
  const payload = extractJsonObject(message.slice(at + marker.length))
  if (!payload)
    return []
  try {
    const parsed = JSON.parse(payload) as { entries?: UnresolvedPatchEntry[] }
    return Array.isArray(parsed.entries) ? parsed.entries : []
  }
  catch {
    return []
  }
}

function extractJsonObject(text: string): string {
  const start = text.indexOf('{')
  if (start < 0)
    return ''
  let depth = 0
  let inString = false
  let escaped = false
  for (let index = start; index < text.length; index++) {
    const char = text[index]
    if (inString) {
      if (escaped)
        escaped = false
      else if (char === '\\')
        escaped = true
      else if (char === '"')
        inString = false
      continue
    }
    if (char === '"') {
      inString = true
    }
    else if (char === '{') {
      depth++
    }
    else if (char === '}') {
      depth--
      if (depth === 0)
        return text.slice(start, index + 1)
    }
  }
  return ''
}
