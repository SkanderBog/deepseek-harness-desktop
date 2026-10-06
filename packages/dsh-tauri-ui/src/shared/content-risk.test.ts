import { describe, expect, it } from 'vitest'
import { isContentRiskFailure } from './content-risk'

describe('isContentRiskFailure', () => {
  it('recognizes the dedicated code and the current DeepSeek compatibility signature', () => {
    expect(isContentRiskFailure({ code: 'CONTENT_REJECTED', message: 'provider-specific copy' })).toBe(true)
    expect(isContentRiskFailure({ code: 'CONTENT_REJECTED', message: 'provider-specific copy', status: 403 })).toBe(true)
    expect(isContentRiskFailure({ code: 'INVALID_REQUEST', message: ' Content Exists Risk ', status: 400 })).toBe(true)
  })

  it('does not classify unrelated invalid requests or non-400 failures as content rejection', () => {
    expect(isContentRiskFailure({ code: 'INVALID_REQUEST', message: 'invalid model', status: 400 })).toBe(false)
    expect(isContentRiskFailure({ code: 'INVALID_REQUEST', message: 'Content Exists Risk', status: 500 })).toBe(false)
    expect(isContentRiskFailure({ code: 'UNKNOWN', message: 'Content Exists Risk', status: 400 })).toBe(false)
    expect(isContentRiskFailure(null)).toBe(false)
  })
})
