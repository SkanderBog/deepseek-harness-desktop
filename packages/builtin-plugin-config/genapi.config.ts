import { defineConfig } from '@genapi/core'
import pipeline from '@genapi/pipeline'
import { ofetch } from '@genapi/presets'
import { original } from 'dsh-h3/genapi'

const root = '../../'
const plugins = ['extension', 'scheduler', 'rightclick', 'archive', 'model', 'ui', 'notification', 'worktree', 'remote']

export default defineConfig({
  preset: pipeline(original, ofetch.ts.parser, ofetch.ts.compiler, ofetch.ts.generate, ofetch.ts.dest),
  meta: { import: { http: 'dsh-tauri/client' } },
  transform: { operation: name => name.replace(/ApiTauri(?:Extension|Scheduler|Rightclick|Archive|Model|Ui|Notification|Worktree|Bridge|Remote)/, '') },
  patch: { operations: { post: 'postWorktree', delete: 'deleteWorktree' } },
  servers: plugins.map(plugin => ({
    input: root + 'packages/builtin-plugin-' + plugin + '/src/host/server/index.ts',
    output: {
      main: root + 'packages/builtin-plugin-' + plugin + '/src/client/apis/index.ts',
      type: root + 'packages/builtin-plugin-' + plugin + '/src/client/apis/index.type.ts',
    },
  })),
})
