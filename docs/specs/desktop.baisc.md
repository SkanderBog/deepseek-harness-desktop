# Electron Runtime 桌面端基础规范

## 当前范围

桌面宿主基于 Rust/Tauri API 与 `tauri-runtime-electron`。本阶段按 `source/v1` 相同相对路径迁移桌面基础框架，保留源代码结构和实现，仅调整业务剥离、Electron API 与 Rust 2024 必需语法。范围包括布局、公共组件与钩子、设置与日志、图标资源和原生系统设施；不迁移 Harness 安装运行、插件管理、远程连接等业务。

## 目录

- `apps/desktop/src/`：React/Vite renderer。
- `apps/desktop/src-tauri/`：Rust shell 与 Electron Runtime 入口。
- `packages/builtin-plugin-*/`：插件源码，按 workspace package 管理。
- `apps/native/`：独立 Expo/RN 项目，不与桌面 Vite 配置混用。

## Runtime 边界

使用 `tauri::Builder::<tauri_runtime_electron::ElectronRuntime>::new()`。Tauri 配置只保留 Electron Runtime 可用的命令、窗口和构建设置；不得添加 Wry 平台实现、远程 WebView origin/capability 或依赖 WebView2 的配置。通过现有 Tauri API 使用 Electron 提供的窗口与缩放能力；原生弹窗策略使用 `tauri_runtime_electron::on_new_window`，不引入旧 WebView 桥。删除 WebView2、WKWebView、WebKitGTK 专属兼容逻辑，保留来源、命令和路径校验。

## 基础命令

- `pnpm dev`：启动桌面 Vite renderer。
- `pnpm build`：构建当前 renderer。
- `pnpm tauri dev`：运行 Electron Runtime shell。
- `pnpm test:unit`：运行 Vitest unit project。
- `pnpm typecheck`：检查 workspace 当前纳入的 TypeScript。
