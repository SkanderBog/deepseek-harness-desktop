import { defineConfig } from '@genapi/core'
import { pluginPipeline } from './genapi.pipeline'

// 本列表的判据是「有需要生成的宿主 HTTP 路由」，不是「有宿主半区」：
// `dsh-tauri`（3 条路由）与 `dsh-tauri-pet`（3 条）都自行手写调用层，没有列出。
// `dsh-tauri-model` 是官方 `ui-settings-models` 的原样 fork，本仓库自有的模型
// 配置宿主路由（打开配置文件 / 端点探测 / 预设表）随该能力一并落在它名下；
// `dsh-tauri-ui` 只留通用层自己的路由（会话恢复 / 未分组目录）。
const plugins = [
  'dsh-tauri-extension',
  'dsh-tauri-scheduler',
  'dsh-tauri-rightclick',
  'dsh-tauri-archive',
  'dsh-tauri-experimental',
  'dsh-tauri-model',
  'dsh-tauri-ui',
  'dsh-tauri-notification',
  'dsh-tauri-worktree',
  'dsh-tauri-ssh',
]

// 壳层走 @tauri-apps/plugin-http（Rust 侧发请求），复用 ssh 路由再生成一份调用层。
const SHELL_SERVERS = [
  {
    input: 'packages/dsh-tauri-ssh/src/host/routes',
    output: { main: 'src/apis/remote.ts', type: 'src/apis/remote.types.ts' },
    meta: { baseURL: JSON.stringify('/api/desktop/dsh-tauri-ssh'), import: { http: './http' } },
  },
]

export default defineConfig({
  preset: pluginPipeline,
  meta: { import: { http: 'dsh-tauri/client' } },
  // worktree 的根级 routes/post.ts、routes/delete.ts 生成名是 `post` / 保留字 `delete`
  patch: { operations: { delete: 'deleteWorktree', post: 'postWorktree' } },
  servers: [
    ...plugins.map(plugin => ({
      input: `packages/${plugin}/src/host/routes`,
      output: { main: `packages/${plugin}/src/client/apis/index.ts` },
      meta: { baseURL: JSON.stringify(`/api/desktop/${plugin}`) },
    })),
    ...SHELL_SERVERS,
  ],
})
