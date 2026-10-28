export default {
  content: ['./index.html', './src/**/*.{js,ts,jsx,tsx}'],
  darkMode: 'class',
  theme: {
    extend: {
      colors: { canvas: 'var(--color-canvas)', panel: 'var(--color-panel)', ink: 'var(--color-ink)', muted: 'var(--muted)', accent: 'var(--accent)', danger: 'var(--danger)' },
      fontFamily: { sans: ['-apple-system', 'BlinkMacSystemFont', 'Segoe UI', 'sans-serif'], mono: ['SF Mono', 'JetBrains Mono', 'Consolas', 'monospace'] },
    },
  },
}
