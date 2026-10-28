import path from 'node:path'
import { fileURLToPath } from 'node:url'
import { defineProject } from 'vitest/config'

const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '../..')

export default defineProject({
  root,
  test: {
    name: 'plugin',
    environment: 'node',
    include: ['tests/e2e/plugins/**/*.e2e.ts'],
    exclude: ['**/node_modules/**', '**/dist/**', '**/target/**'],
    fileParallelism: false,
    testTimeout: 120000,
    hookTimeout: 120000,
  },
})
