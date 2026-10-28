import path from 'node:path'
import { fileURLToPath } from 'node:url'
import { defineConfig } from 'vitest/config'

const nativeRoot = path.dirname(fileURLToPath(import.meta.url))

export default defineConfig({
  root: nativeRoot,
  resolve: { alias: { '@': path.join(nativeRoot, 'src') } },
  test: {
    name: 'native',
    environment: 'node',
    include: ['src/**/*.test.{ts,tsx}'],
    testTimeout: 10000,
  },
})
