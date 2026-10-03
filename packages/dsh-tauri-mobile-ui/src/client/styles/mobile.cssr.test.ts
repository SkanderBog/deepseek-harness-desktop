import postcss from 'postcss'
import { describe, expect, it, vi } from 'vitest'
import { cssr } from '../../../../dsh-tauri-ui/src/client/utils/cssr'
import { MOBILE_MEDIA_QUERIES } from '../../../../dsh-tauri/src/client/utils/device'
import mobileStyle from './mobile.cssr'

vi.mock('dsh-tauri/client', () => ({ MOBILE_MEDIA_QUERIES }))
vi.mock('dsh-tauri-ui/client', () => ({ cssr }))

describe('mobile conversation layout', () => {
  const root = postcss.parse(mobileStyle.render())

  it('keeps current mobile layout overrides scoped to mobile devices', () => {
    const media = root.nodes.find(node => node.type === 'atrule' && node.name === 'media' && node.params === '(hover: none) and (any-pointer: coarse) and (any-hover: none)')
    expect(media?.type).toBe('atrule')
    if (media?.type !== 'atrule')
      throw new Error('Missing mobile media query')
    expect(media.params).toBe('(hover: none) and (any-pointer: coarse) and (any-hover: none)')
    const rules: Record<string, unknown> = {}
    media.walkRules((rule) => {
      rules[rule.selector] = rule.nodes.map(node => node.type === 'decl' ? [node.prop, node.value, node.important] : [])
    })
    expect(rules).toEqual({
      '[data-dsh-mobile-preferences]': [['display', 'flex', undefined], ['flex', '1', undefined], ['min-width', '0', undefined], ['align-items', 'center', undefined], ['gap', '8px', undefined], ['padding', '8px 0', undefined]],
      '[data-dsh-mobile-preferences] button': [['margin-left', 'auto', undefined], ['flex-shrink', '0', undefined]],
      '[data-slot="conversation.composer.bar"] [class$="_dock"]': [['display', 'none', true]],
      '[class$="_composerStack"] > [data-slot="conversation.input.dock"]': [['display', 'none', true]],
      '[class$="_turnErrorCode"]': [['display', 'none', true]],
      '[data-slot="conversation.header"] [class$="_header"]': [['display', 'none', true]],
      '[data-slot="main"] header[class*="_pageHead"]': [['padding-left', '0', true], ['padding-top', '24px', true]],
      'header[class*="_pageHead"] [class*="_toolbar"]': [['display', 'none', true]],
      '[data-slot="conversation.view"] [class$="_scroll"]': [['padding', '16px', true]],
      '[class*="_userStack"]': [['max-width', '100%', true]],
      '[data-slot="main"] [data-conversation-scroll]': [['padding-bottom', '0', true]],
    })
  })

  it('overrides conversation scroll bottom padding to zero inside the mobile media query', () => {
    const declarations: unknown[] = []
    root.walkRules('[data-slot="main"] [data-conversation-scroll]', (rule) => {
      const parent = rule.parent
      expect(parent?.type).toBe('atrule')
      if (parent?.type === 'atrule')
        expect(parent.params).toBe('(hover: none) and (any-pointer: coarse) and (any-hover: none)')
      rule.walkDecls((decl) => {
        declarations.push([decl.prop, decl.value, decl.important])
      })
    })
    expect(declarations).toEqual([['padding-bottom', '0', true]])
  })
})
