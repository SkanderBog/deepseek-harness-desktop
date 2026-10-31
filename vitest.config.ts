import { defineConfig } from 'vitest/config'

export default defineConfig({
  test: {
    projects: [
      './config/vitest.unit.config.ts',
      './config/vitest.desktop.config.ts',
    ],
    coverage: { provider: 'v8', exclude: ['**/node_modules/**', '**/dist/**', '**/target/**', '**/*.config.*', 'apps/desktop/src-tauri/**', 'apps/native/android/**', 'apps/native/ios/**'] },
  },
})
