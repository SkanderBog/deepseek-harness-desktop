import { defineProject } from 'vitest/config'
import desktopConfig from './vitest.desktop.config'

export default defineProject({
  test: {
    ...desktopConfig.test,
    name: 'webdriver',
    include: ['test/e2e/webdriver/*.e2e.ts'],
  },
})
