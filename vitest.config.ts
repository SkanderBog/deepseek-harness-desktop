import { defineConfig } from 'vitest/config'

export default defineConfig({
  test: {
    projects: ['./vitest.unit.config.ts', './packages/builtin-plugin-config/vitest.config.ts', './apps/desktop/vitest.config.ts'],
    coverage: { provider: 'v8', exclude: ['**/node_modules/**', '**/dist/**', '**/target/**', '**/*.config.*', 'apps/desktop/src-tauri/**', 'apps/native/android/**', 'apps/native/ios/**'] },
  },
})
