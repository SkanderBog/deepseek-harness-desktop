import { defineConfig } from 'bumpp'

export default defineConfig({
  release: 'prompt',
  all: true,
  execute: 'node scripts/native-app-version.mjs',
  files: [
    'package.json',
    'src-tauri/Cargo.toml',
    'src-tauri/Cargo.lock',
    'src-tauri/tauri.conf.json',
  ],
})
