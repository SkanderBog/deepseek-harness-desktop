import { defineProject } from 'vitest/config'
import { SHARED_ALIAS } from './tooling.config'

export default defineProject({
  resolve: {
    alias: SHARED_ALIAS,
  },
  test: {
    name: 'unit',
    include: [
      'packages/**/*.{test,spec}.{ts,tsx,js,mjs,cjs}',
      'test/**/*.test.ts',
      'src/**/*.test.ts',
    ],
    exclude: [
      '**/node_modules/**',
      'test/archive/**',
      'archive/**',
      // 渲染官方 UI primitives 的 ssh 组件套件需要包内 vitest.config 的
      // CSS 内联与 react 单副本别名（pnpm 下 primitives 嵌套 react@18），
      // 本 project 无此处理会挂载失败；这些套件由
      // `pnpm --filter dsh-tauri-ssh test` 全量执行。
      'packages/dsh-tauri-ssh/src/client/components/**',
      'packages/dsh-tauri-ssh/src/client/index.test.ts',
    ],
    // 壳层模块在导入期就访问 Tauri API，node 下需要最小运行时垫片，见该文件说明。
    setupFiles: ['./test/setup/tauri-runtime.ts'],
    maxWorkers: 4,
    testTimeout: 30_000,
    hookTimeout: 30_000,
  },
})
