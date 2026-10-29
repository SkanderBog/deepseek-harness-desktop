import { usePreferredDark, useWatch } from '@reause/core'

export function resolveTheme(
  preference: 'dark' | 'light' | 'system' | null | undefined,
  systemDark: boolean,
): 'dark' | 'light' {
  return preference === 'light' || preference === 'dark'
    ? preference
    : (systemDark ? 'dark' : 'light')
}

export function useThemeAdaptive() {
  const systemDark = usePreferredDark()
  const theme = resolveTheme('system', systemDark)
  useWatch(
    theme,
    value => document.documentElement.dataset.theme = value,
    { immediate: true },
  )
}
