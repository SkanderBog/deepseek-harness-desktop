import { describe, expect, it } from 'vitest'
import { APPEARANCE_DEFAULTS, APPEARANCE_PALETTES, appearanceColors, appearanceTokens, normalizeAppearance } from './appearance'

describe('appearance preferences', () => {
  it('migrates old transparency settings but respects an explicit disabled switch', () => {
    expect(normalizeAppearance({ opacity: 70 })).toMatchObject({ transparency: true, sidebarOnly: false })
    expect(normalizeAppearance({ opacity: 70, transparency: false })).toMatchObject({ transparency: false, opacity: 70 })
    expect(normalizeAppearance({ opacity: 100, transparency: true, sidebarOnly: true })).toMatchObject({ transparency: true, sidebarOnly: true })
  })

  it.each([undefined, null, {}, 1, 'nord', { palette: 'missing', terminal: 'true', opacity: Number.NaN }])('keeps original defaults for invalid or missing preferences: %j', (value) => {
    expect(normalizeAppearance(value)).toEqual(APPEARANCE_DEFAULTS)
  })

  it.each([[0, 20], [255, 100], [77.4, 77], [80, 80]])('normalizes opacity %s to %s', (input, opacity) => {
    expect(normalizeAppearance({ palette: 'nord', terminal: true, opacity: input })).toEqual({ palette: 'nord', terminal: true, transparency: opacity < 100, opacity, sidebarOnly: false })
  })

  it('does not override core theme tokens under default settings', () => {
    expect(appearanceTokens(APPEARANCE_DEFAULTS)).toEqual({})
  })

  it.each(APPEARANCE_PALETTES)('keeps %s text readable on all opaque palette surfaces in both modes', (palette) => {
    for (const scheme of ['light', 'dark'] as const) {
      const colors = appearanceColors({ ...APPEARANCE_DEFAULTS, palette }, scheme)
      for (const background of [colors.canvas, colors.panel, colors.surface]) {
        for (const foreground of [colors.text, colors.muted]) {
          const [dark, light] = [luminance(background), luminance(foreground)].sort((a, b) => a - b)
          expect((light + 0.05) / (dark + 0.05), `${palette}/${scheme}: ${foreground} on ${background}`).toBeGreaterThanOrEqual(4.5)
        }
      }
    }
  })
})

function luminance(hex: string): number {
  const channels = [1, 3, 5].map((offset) => {
    const value = Number.parseInt(hex.slice(offset, offset + 2), 16) / 255
    return value <= 0.04045 ? value / 12.92 : ((value + 0.055) / 1.055) ** 2.4
  })
  return channels[0] * 0.2126 + channels[1] * 0.7152 + channels[2] * 0.0722
}
