export const APPEARANCE_DEFAULTS = { palette: 'default', terminal: false, transparency: false, opacity: 100, blur: false, sidebarOnly: false } as const
export const APPEARANCE_PALETTES = ['default', 'nord', 'solarized', 'forest', 'amber', 'github', 'github-dimmed', 'github-high-contrast'] as const

export interface Appearance {
  palette: typeof APPEARANCE_PALETTES[number]
  terminal: boolean
  transparency: boolean
  opacity: number
  blur: boolean
  sidebarOnly: boolean
}

function normalizeInteger(value: unknown, minimum: number, maximum: number, fallback: number): number {
  return typeof value === 'number' && Number.isFinite(value)
    ? Math.round(Math.min(maximum, Math.max(minimum, value)))
    : fallback
}

export function normalizeAppearance(value: unknown): Appearance {
  const input = value as (Omit<Partial<Appearance>, 'blur'> & { blur?: unknown }) | null
  const opacity = normalizeInteger(input?.opacity, 20, 100, 100)
  return {
    palette: APPEARANCE_PALETTES.includes(input?.palette as Appearance['palette']) ? input!.palette! : 'default',
    terminal: input?.terminal === true,
    transparency: input?.transparency === undefined ? opacity < 100 : input.transparency === true,
    opacity,
    blur: input?.blur === true || (typeof input?.blur === 'number' && input.blur > 0),
    sidebarOnly: input?.sidebarOnly === true,
  }
}

export function appearanceBackdropFilter(appearance: Appearance): string {
  return appearance.transparency && appearance.opacity < 100 && appearance.blur
    ? 'blur(16px)'
    : 'none'
}

export function appearanceSidebarFill(canvas: string, panel: string, translucent: boolean, percent: number): string {
  return translucent ? `color-mix(in srgb,${canvas} ${percent}%,transparent)` : panel
}

/** 启动页填充色：与 navbar / 侧边栏同源同 alpha，100% 或不透明时回到实心 canvas。 */
export function appearanceStartupFill(appearance: Appearance, canvas: string): string {
  return appearanceSidebarFill(canvas, canvas, appearance.transparency && appearance.opacity < 100, appearance.opacity)
}

/**
 * 官方 web shell boot 页（`[data-dsh-boot]`）的透明投影。
 *
 * 该页由内核在**插件加载之前**绘出，此刻外观插件还没激活，所以只能用与宿主同源的
 * CSS 表达：`html`/`body`/`#root` 全透明，由 boot 节点自身承载与 navbar 一致的 alpha。
 * 只绘制一层 alpha——boot 节点铺底后 `body` 再叠加会得到 0.7×0.7≈0.91 的错误浓度。
 *
 * 背景必须落在与 boot 页同优先级的规则上：官方 `.boot` 的背景表达式是
 * `var(--dsw-alias-bg-base, var(--dsh-boot-bg, Canvas))`，只要 theme 的
 * `--dsw-alias-bg-base` 存在，仅设 `--dsh-boot-bg` 就会被它压掉。选择器保持结构性
 * 且 boot-only，boot 一被 React 交接替换，整套规则自动失效（不污染正常页面，也不需要
 * 给 `[data-dsh-boot]` 注入 inline style，避免 BootHandoff hydration 额外属性告警）。
 *
 * `bodyOnly` 供「自己已经画了 body alpha」的调用方（外观插件）使用：此时只取 boot
 * 节点本体的填充规则，避免与它后写的 `body{background:…}` 争同一层 alpha。
 */
export function appearanceBootCss(appearance: Appearance, bodyOnly = false): string {
  if (!appearance.transparency || appearance.opacity >= 100)
    return ''
  const blocks = (['dark', 'light'] as const).map((scheme) => {
    const { canvas } = appearanceColors(appearance, scheme)
    const selector = scheme === 'dark' ? 'body[data-ds-dark-theme]' : 'body:not([data-ds-dark-theme])'
    const fill = appearanceStartupFill(appearance, canvas)
    return `html:has(${selector} > #root > [data-dsh-boot]),${selector} > #root > [data-dsh-boot]{background:${fill}!important}`
  })
  return [
    ...bodyOnly ? [] : ['html,body,#root{background:transparent!important}'],
    ...blocks,
  ].join('\n')
}

type Palette = readonly [canvas: string, panel: string, surface: string, text: string, muted: string, accent: string, border?: string]

const PALETTES: Record<Appearance['palette'], Record<'dark' | 'light', Palette>> = {
  'default': { dark: ['#151517', '#1b1b1c', '#2c2c2e', '#f9fafb', '#adb2b8', '#7aaaff'], light: ['#ffffff', '#f9fafb', '#f1f3f5', '#0f1115', '#61666b', '#4176e6'] },
  'nord': { dark: ['#2e3440', '#343c4a', '#434c5e', '#eceff4', '#b8c5d6', '#88c0d0'], light: ['#eceff4', '#e5e9f0', '#d8dee9', '#2e3440', '#4c566a', '#375f85'] },
  'solarized': { dark: ['#002b36', '#073642', '#164450', '#eee8d5', '#a4b4b3', '#64b5c5'], light: ['#fdf6e3', '#eee8d5', '#e3dcc8', '#073642', '#4a5d62', '#006c82'] },
  'forest': { dark: ['#15211b', '#1c2d23', '#2b3d31', '#e8f4e9', '#b0c9b6', '#8bcea0'], light: ['#f4f8f0', '#e8efe2', '#d8e4d0', '#203b29', '#49614d', '#276b3d'] },
  'amber': { dark: ['#211c14', '#2b241a', '#3c3223', '#fff1d6', '#cfbd9d', '#f0bd68'], light: ['#fff8eb', '#f3e8d1', '#e8d8b8', '#3c2c13', '#695638', '#875700'] },
  'github': { dark: ['#0d1117', '#010409', '#151b23', '#f0f6fc', '#9198a1', '#4493f8', '#3d444d'], light: ['#ffffff', '#f6f8fa', '#f6f8fa', '#1f2328', '#59636e', '#0969da', '#d1d9e0'] },
  'github-dimmed': { dark: ['#212830', '#151b23', '#262c36', '#d1d7e0', '#9198a1', '#6cb6ff', '#3d444d'], light: ['#ffffff', '#f6f8fa', '#f6f8fa', '#1f2328', '#59636e', '#0969da', '#d1d9e0'] },
  'github-high-contrast': { dark: ['#010409', '#151b23', '#151b23', '#ffffff', '#b7bdc8', '#74b9ff', '#b7bdc8'], light: ['#ffffff', '#eff2f5', '#e6eaef', '#010409', '#454c54', '#023b95', '#454c54'] },
}

export function appearanceColors(appearance: Appearance, scheme: 'dark' | 'light') {
  const [canvas, panel, surface, text, muted, accent, border] = PALETTES[appearance.palette][scheme]
  return { canvas, panel, surface, text, muted, accent, border }
}

export function appearanceTokens(appearance: Appearance): Record<string, { dark: string, light: string }> {
  if (appearance.palette === 'default' && (!appearance.transparency || appearance.opacity === 100))
    return {}
  function modes(scheme: 'dark' | 'light'): Record<string, string> {
    const { canvas, panel, surface, text, muted, accent, border } = appearanceColors(appearance, scheme)
    const tokens: Record<string, string> = {
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
      '--dsw-alias-label-caption': muted,
      '--dsw-alias-state-business-primary': accent,
    }
    if (border) {
      tokens['--dsw-alias-link'] = accent
      for (const level of ['l1', 'l2', 'l2-darkmode-thin', 'l3', 'l4'])
        tokens[`--dsw-alias-border-${level}`] = border
    }
    return tokens
  }
  const dark = modes('dark')
  const light = modes('light')
  return Object.fromEntries(Object.keys(dark).map(key => [key, {
    dark: dark[key],
    light: light[key],
  }]))
}
