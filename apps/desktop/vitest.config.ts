import { fileURLToPath } from 'node:url'
import { defineProject } from 'vitest/config'

export default defineProject({
  test: {
    name: 'desktop',
    environment: 'node',
    root: fileURLToPath(new URL('.', import.meta.url)),
    include: ['tests/e2e/**/*.e2e.ts'],
    exclude: ['**/node_modules/**', '**/dist/**', '**/target/**'],
    setupFiles: ['./tests/e2e/setup/shell.ts'],
    fileParallelism: false,
    testTimeout: 180000,
    hookTimeout: 180000,
  },
})
