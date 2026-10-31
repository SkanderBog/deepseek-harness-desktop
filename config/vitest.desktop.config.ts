import { defineProject } from 'vitest/config'

export default defineProject({
  test: {
    name: 'desktop',
    environment: 'node',
    include: ['tests/e2e/desktop/**/*.e2e.ts'],
    exclude: ['**/node_modules/**', '**/dist/**', '**/target/**'],
    setupFiles: ['./tests/e2e/desktop/setup/shell.ts'],
    fileParallelism: false,
    testTimeout: 180_000,
    hookTimeout: 180_000,
  },
})
