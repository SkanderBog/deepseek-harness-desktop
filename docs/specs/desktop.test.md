# Electron Runtime 桌面端测试协议

当前迁移阶段不迁移旧 Tauri/WebView2 的 WebdriverIO L3 测试。Vitest 根配置只声明 `unit`、`plugin`、`desktop` project；native 测试独立运行，不共享桌面项目。

桌面 E2E 在 Electron 主进程、隔离用户数据目录和真实 renderer 运行方式确定后再建立。不得把 renderer DOM 测试伪装成原生窗口/IPC 测试，也不得读取或修改真实用户目录。

目前可运行的基础检查：

- `pnpm test:unit`
- `pnpm test:native`
- `pnpm typecheck:native`
- `pnpm --filter dsh-desktop build`（只构建现有 renderer；不代表 Electron shell 已打包）
