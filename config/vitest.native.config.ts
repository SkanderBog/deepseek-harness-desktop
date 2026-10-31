import { fileURLToPath } from 'node:url'
import { defineConfig } from 'vitest/config'

export default defineConfig({
  root: fileURLToPath(new URL('../apps/native', import.meta.url)),
  resolve: { alias: { '@': fileURLToPath(new URL('../apps/native/src', import.meta.url)) } },
  test: {
    name: 'native',
    environment: 'node',
    include: ['src/**/*.test.{ts,tsx}'],
    testTimeout: 60000,
  },
})
