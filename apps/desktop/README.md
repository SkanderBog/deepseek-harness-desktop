# Desktop 基础框架

桌面端以 Rust/Tauri API 和 `tauri-runtime-electron` 为宿主，前端为 React/Vite。迁移来源为 `source/v1`：原 `src/`、`src-tauri/`、资源和图标脚本保留相同相对路径，除业务剥离、Electron 适配和 Rust 2024 语法外保留源实现。

已迁移布局、HeroUI/Overlastic/Query providers、公共组件、主题与样式、语言检测、提示队列、日志、设置持久化、公共钩子和工具、品牌图标、托盘、窗口几何、剪贴板文本、外链与下载、系统登录启动、单实例和启动校验。开发、正式版与测试 store 文件隔离，renderer 的文件名由 Rust 注入。

Harness 安装与运行、插件管理、远程连接、桌宠和更新等业务不在本次范围内。主题文件解析等通用函数保留，业务数据读取接点随业务迁移恢复；当前布局内容区为空。旧 WebView2、WKWebView 和 WebKitGTK 兼容实现未迁移。

```sh
pnpm dev:desktop
pnpm --filter dsh-desktop build
pnpm typecheck
pnpm exec vitest run --project unit apps/desktop --sequence.shuffle
cargo test --manifest-path apps/desktop/src-tauri/Cargo.toml --lib --locked -- --test-threads=1
```

最小内容尺寸为 860×620。Electron 原生页面支持右键编辑菜单与 F12 开关开发者工具；禁用开发者工具的页面保留禁用策略。修改 `crates/electron-rs` 后须生成其 runtime assets，并重新构建、重启桌面进程。

主窗口默认关闭到托盘：点击 X 完全隐藏窗口但保留进程，托盘左键或“打开面板”恢复窗口并聚焦；托盘“退出”结束应用。显式设置 `close_action=quit` 时仍关窗退出。主窗口最小化与任务栏还原保留原有大小。平台限制参见 `docs/restricted/`，前端构建成功不代表安装包或各平台均已验收。
