import { fileURLToPath } from 'node:url'

const root = fileURLToPath(new URL('../..', import.meta.url)).replaceAll('\\', '/')
const source = root + '/packages/builtin-plugin-*/src/**/*.{js,ts,jsx,tsx}'
const excluded = '!' + root + '/packages/builtin-plugin-ui/src/client/styles/index.ts'

export default {
  darkMode: 'class',
  content: [source, excluded],
  theme: {
    extend: {
      colors: { primary: 'var(--dsw-alias-label-primary)', secondary: 'var(--dsw-alias-label-secondary)', tertiary: 'var(--dsw-alias-label-tertiary)', dimmed: 'var(--dsw-alias-label-dimmed)', 'border-l2': 'var(--dsw-alias-border-l2)', 'border-l3': 'var(--dsw-alias-border-l3)', 'border-l4': 'var(--dsw-alias-border-l4)', brand: 'var(--dsw-alias-brand-primary)', business: 'var(--dsw-alias-state-business-primary)', 'layer-1': 'var(--dsw-alias-bg-layer-1)', 'layer-3': 'var(--dsw-alias-bg-layer-3)', hover: 'var(--dsw-alias-interactive-bg-hover)', active: 'var(--dsw-alias-interactive-bg-active)', error: 'var(--dsw-alias-state-error-primary)', success: 'var(--dsw-alias-state-success-primary)', warn: 'var(--dsw-alias-state-warn-primary)' },
      fontFamily: { sans: ['var(--dsw-font-family)'], mono: ['var(--ds-font-family-code)'] },
      borderRadius: { xs: 'var(--dsw-radius-xs)', sm: 'var(--dsw-radius-sm)', md: 'var(--dsw-radius-md)', lg: 'var(--dsw-radius-lg)', xl: 'var(--dsw-radius-xl)' },
      boxShadow: { 'focus-ring': '0 0 0 2px var(--dsw-alias-border-l3)' },
    },
  },
}
