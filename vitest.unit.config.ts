import { fileURLToPath } from 'node:url'
import { defineProject } from 'vitest/config'

const alias = {
  '@': fileURLToPath(new URL('./apps/desktop/src', import.meta.url)),
  'dsh-tauri': fileURLToPath(new URL('./packages/builtin-plugin-tauri/src/index.ts', import.meta.url)),
  'dsh-tauri/client': fileURLToPath(new URL('./packages/builtin-plugin-tauri/src/client/index.ts', import.meta.url)),
  'dsh-tauri-ui': fileURLToPath(new URL('./packages/builtin-plugin-ui/src/index.ts', import.meta.url)),
  'dsh-tauri-ui/client': fileURLToPath(new URL('./packages/builtin-plugin-ui/src/client/index.ts', import.meta.url)),
}

export default defineProject({
  resolve: { alias },
  test: {
    name: 'unit',
    environment: 'node',
    include: ['packages/builtin-plugin-*/src/**/*.{test,spec}.{ts,tsx,js,mjs,cjs}', 'tests/**/*.test.{ts,tsx}'],
    exclude: ['**/node_modules/**', '**/dist/**', '**/target/**'],
    testTimeout: 30000,
    hookTimeout: 30000,
  },
})
