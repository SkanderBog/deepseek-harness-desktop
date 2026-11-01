import { defineConfig } from 'bumpp'

export default defineConfig({
  release: 'prompt',
  files: [
    'package.json',
    'apps/desktop/package.json',
    'apps/native/package.json',
    'apps/desktop/src-tauri/Cargo.toml',
    'apps/desktop/src-tauri/Cargo.lock',
    'apps/desktop/src-tauri/tauri.conf.json',
    'apps/native/app.json',
  ],
})
