export const APPEARANCE_DEFAULTS = { palette: 'default', terminal: false, opacity: 100 } as const
export const APPEARANCE_PALETTES = ['default', 'nord', 'solarized', 'forest', 'amber'] as const

export interface Appearance {
  palette: typeof APPEARANCE_PALETTES[number]
  terminal: boolean
  opacity: number
}

export function normalizeAppearance(value: unknown): Appearance {
  const input = value as Partial<Appearance> | null
  return {
    palette: APPEARANCE_PALETTES.includes(input?.palette as Appearance['palette']) ? input!.palette! : 'default',
    terminal: input?.terminal === true,
    opacity: typeof input?.opacity === 'number' && Number.isFinite(input.opacity)
      ? Math.round(Math.min(100, Math.max(20, input.opacity)))
      : 100,
  }
}

const PALETTES = {
  default: { dark: ['#151517', '#1b1b1c', '#2c2c2e', '#f9fafb', '#adb2b8', '#7aaaff'], light: ['#ffffff', '#f9fafb', '#f1f3f5', '#0f1115', '#61666b', '#4176e6'] },
  nord: { dark: ['#2e3440', '#343c4a', '#434c5e', '#eceff4', '#b8c5d6', '#88c0d0'], light: ['#eceff4', '#e5e9f0', '#d8dee9', '#2e3440', '#4c566a', '#375f85'] },
  solarized: { dark: ['#002b36', '#073642', '#164450', '#eee8d5', '#a4b4b3', '#64b5c5'], light: ['#fdf6e3', '#eee8d5', '#e3dcc8', '#073642', '#4a5d62', '#006c82'] },
  forest: { dark: ['#15211b', '#1c2d23', '#2b3d31', '#e8f4e9', '#b0c9b6', '#8bcea0'], light: ['#f4f8f0', '#e8efe2', '#d8e4d0', '#203b29', '#49614d', '#276b3d'] },
  amber: { dark: ['#211c14', '#2b241a', '#3c3223', '#fff1d6', '#cfbd9d', '#f0bd68'], light: ['#fff8eb', '#f3e8d1', '#e8d8b8', '#3c2c13', '#695638', '#875700'] },
} as const

export function appearanceColors(appearance: Appearance, scheme: 'dark' | 'light') {
  const [canvas, panel, surface, text, muted, accent] = PALETTES[appearance.palette][scheme]
  return { canvas, panel, surface, text, muted, accent }
}

export function appearanceTokens(appearance: Appearance): Record<string, { dark: string, light: string }> {
  if (appearance.palette === 'default' && appearance.opacity === 100)
    return {}
  const modes = (scheme: 'dark' | 'light') => {
    const { canvas, panel, surface, text, muted, accent } = appearanceColors(appearance, scheme)
    return {
      '--dsw-alias-bg-base': canvas,
      '--dsw-specific-sidebar-fill': panel,
      '--dsw-alias-bg-layer-1': panel,
      '--dsw-alias-bg-layer-2': surface,
      '--dsw-alias-bg-layer-3': surface,
      '--dsw-alias-bg-module-platform': surface,
      '--dsw-alias-bg-overlay': panel,
      '--dsw-specific-menu': panel,
      '--dsw-specific-input-major': panel,
      '--dsw-specific-bubble': surface,
      '--dsw-alias-label-primary': text,
      '--dsw-alias-label-secondary': muted,
      '--dsw-alias-label-tertiary': muted,
      '--dsw-alias-state-business-primary': accent,
    }
  }
  const dark = modes('dark')
  const light = modes('light')
  return Object.fromEntries(Object.keys(dark).map(key => [key, {
    dark: dark[key as keyof typeof dark],
    light: light[key as keyof typeof light],
  }]))
}
