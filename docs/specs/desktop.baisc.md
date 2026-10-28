# Electron Runtime 桌面端基础规范

## 当前范围

桌面宿主基于 Rust/Tauri API 与 `tauri-runtime-electron`。本阶段只迁移配置、构建和基础设施，不迁移完整桌面业务实现。桌面源码仍在 `apps/desktop/src/`，迁移阶段保持原样。

## 目录

- `apps/desktop/src/`：React/Vite renderer。
- `apps/desktop/src-tauri/`：Rust shell 与 Electron Runtime 入口。
- `packages/builtin-plugin-*/`：插件源码，按 workspace package 管理。
- `apps/native/`：独立 Expo/RN 项目，不与桌面 Vite 配置混用。

## Runtime 边界

使用 `tauri::Builder::<tauri_runtime_electron::ElectronRuntime>::new()`。Tauri 配置只保留 Electron Runtime 可用的命令、窗口和构建设置；不得添加 Wry/WebView 权限、远程 WebView origin/capability 或依赖 WebView2 的配置。Electron 主进程/预加载的完整安全策略由后续 desktop 实现补齐，本阶段不实现桥接层。

## 基础命令

- `pnpm dev`：启动桌面 Vite renderer。
- `pnpm build`：构建当前 renderer。
- `pnpm tauri dev`：运行 Electron Runtime shell；需先确认外部 sibling `tauri-runtime-electron` checkout 可用。
- `pnpm test:unit`：运行 Vitest unit project。
- `pnpm typecheck`：检查 workspace 当前纳入的 TypeScript。

当前 scaffold 不等同于完整应用可交付；桌面 IPC、窗口、安全 preload、打包和更新仍属于后续实现。
