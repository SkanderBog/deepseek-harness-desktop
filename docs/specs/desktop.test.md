# Electron Runtime 桌面端测试协议

当前迁移阶段不迁移旧 Tauri/WebView2 的 WebdriverIO L3 测试。Vitest 根配置只声明 `unit`、`plugin`、`desktop` project；native 测试独立运行，不共享桌面项目。

桌面 E2E 在 Electron 主进程、隔离用户数据目录和真实 renderer 运行方式确定后再建立。不得把 renderer DOM 测试伪装成原生窗口/IPC 测试，也不得读取或修改真实用户目录。

目前可运行的基础检查：

- `pnpm test:unit`
- `pnpm test:native`
- `pnpm typecheck:native`
- `pnpm --filter dsh-desktop build`（构建 renderer；不代表 Electron shell 已打包）
- `cargo check --manifest-path apps/desktop/src-tauri/Cargo.toml --all-targets`
- `cargo test --manifest-path apps/desktop/src-tauri/Cargo.toml --lib --locked -- --test-threads=1`
- `pnpm --dir crates/electron-rs run test:checks`（生产 runtime CJS + 原生替身，不等同真实桌面验收）

真实 Electron 验证须使用独立 userData/sessionData，仅关闭测试自身窗口；报告中区分原生 API 还原、输入注入、菜单事件触发与人工任务栏点击。高 DPI、多显示器、macOS/Linux 和托盘生命周期分别验收，不以单平台通过替代。Windows 测试目标通过原版 `windows-app-manifest.xml` 嵌入 Common Controls v6，禁止仅为测试绕过原生依赖或削弱断言。
