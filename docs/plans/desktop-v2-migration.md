# v2 全量 desktop 迁移交接

下一阶段的工作计划与现状。规范冲突时以 `docs/specs/` 为准。

## 目标

把 `source/v1/src` 与 `source/v1/src-tauri/src` 的全量桌面代码迁入 `apps/desktop`，承载 dsh 页面的方式从 iframe 换成 Electron 的 `<webview>` 标签。

## 硬约束

1. 一比一路径迁移：先把 v1 原文件复制到相同相对路径，再在该文件上改写差异。不允许按理解重写整份文件。
2. 不再使用 iframe：`layout/components/iframe.tsx` 改写为 `<webview>` 标签；`hooks/use-iframe-message`、`use-iframe-post`、`use-invoke-iframe`、`use-listen-iframe`、`hooks/use-appearance` 直接删除，不迁、不留空壳。`use-sync-visibility` 改为 `executeJavaScript`。
3. 剔除不适用代码：WebView2、WKWebView、WebKitGTK、wry 专属分支与注释。
4. 内置插件与测试文件不在范围：`packages/builtin-plugin-*` 不动；v1 的 `*.test.*`、`*.spec.*` 不迁。
5. 运行时落位不变：DSH 核心与 pnpm 仍装到 `%APPDATA%\dsh-tauri`（debug 为 `...\dev`），沿用 `config/dependencies.rs` 与 `resources/manifest.jsonc` 的 `managedRoot`。
6. 不再需要 node 与 git：node 用 `ElectronClient::node`，git 连同 Windows MinGit 一起剔除。
7. pkg 分发改用 `.asar.zip`：解压得到单个 `.asar`，由 Electron 挂载后运行内部 dsh bin。
8. 根因归属：运行时/桥接能力缺失一律在 `crates/electron-rs`、`crates/tauri-runtime-electron` 修，然后更新本仓库子模块指针。禁止在 `apps/desktop` 写补丁绕过。

## 已完成

前端 67 个文件（含 6 个测试）按原路径迁入：`components/` 12 个全部、`config/`、`i18n/`（四语种精简表）、`styles/`、`utils/` 11 个全部、`hooks/` 中 15 个与 iframe 无关的、`layout/navbar`（简版）、`store/modules/setting` 全量。

Rust 28 个文件：`logger/`、`config/{i18n,setting,appearance,theme,utils,proxy,constants,window_state}`、`desktop/{builder,window,payload,deep_link,autostart,login_path,startup,single_instance,activation}`、`bridge/{clipboard,system_os,guard,config,mod}`、`utils/`。入口已接 `logger::init`、启动校验、E2E 目录覆盖、`create=false`、单实例、13 个命令、窗口生命周期。

窗口行为：默认关闭到托盘（`prevent_close` + hide，macOS 用应用级 hide 与 Accessory 切换），`close_action=quit` 才退出；托盘、Dock Reopen、单实例唤醒统一走 `utils::show_main_window`。

需补回的简版：`navbar.tsx` 现 110 行（v1 是 822 行），文件/运行/帮助菜单、远程切换器、子窗口标识缺失；`layout/index.tsx` 只挂主题与设置刷新，`dsh-plugins-updated` 写入、更新轮询、下载完成 toast 未接。

## `<webview>` 标签（已就绪）

`electron-rs` `1221322` 按窗口开放 `webviewTag`（默认开启），页面创建的 guest 登记进 webview 注册表，`Policies.attach` 只挂一次，关闭时按普通 webview 清理。`tauri-runtime-electron` `ad07fb3` 在 `webview.attached` 后给 guest 分配 label 并注册 IPC 路由，未安装 guest 专用 handler 时回落到宿主页面的 handler。

因此 guest 里的 `__TAURI_INTERNALS__.invoke` 是原生的，不需要 iframe 中转。边界：guest 没有 Tauri 管理的 webview，ACL 按宿主 label 解析；需要独立身份时用 `RuntimeContext::on_guest_ipc`。

前端改写口径：`iframeRef`/`src`/`onLoad`/`postMessage` 换成 `<webview src partition preload>`，交互用 `executeJavaScript`/`send`/`ipc-message`。`<webview>` 不参与 `WebContentsView` 布局，尺寸由 CSS 决定。

`ElectronClient` 插件尚未在 v2 注册，需 `.plugin(tauri_runtime_electron::init())` 后才能取 `client.node(args)`。

## 待迁清单

`source/v1/src` 共 194 个文件，非测试 145 个，已迁 61 个，剩 85 个。Rust 侧 172 个，非测试 171 个，已迁 28 个，剩 143 个。

- `store/` 26：`desktop-updater`、`harness-updater`、`harness`（含 `patch-layer`/`readiness`/`runtime`/`utils`）、`plugins`、`preinstall`、`recovery`。
- `ui/` 18：`config/` 七个面板与三个 `components/`、`dialog/` 六个、`pet/hint`、`plugin/recovery`。
- `pet/` 15：独立窗口入口。
- `layout/` 9：`connect-dialog`、`full-sidebar-navbar`、`remote-share`、`remote-switcher`、`setup`、`setup-preinstall`、`window-presentation`、`nav-bridge`、`iframe.tsx`。
- `hooks/` 7：`use-backup`、`use-distro-managed`、`use-dsh-shortcuts`、`use-omit-ignore-cursor-events`、`use-plugins-manager`、`use-remote`、`use-window-draggable`。
- `types/` 6、`apis/` 3。

Rust：`service/` 113 个文件 59190 行；`bridge/` 缺 11 个；`desktop/` 缺 16 个；`config/` 缺 4 个。v1 `desktop::handler()` 注册 124 个命令，v2 目前 13 个。

`desktop/compat*`、`tauri_internals`、`frontend_preload` 先查调用方，确认是旧引擎专属才剔除。

运行时文件迁移时同步改造，不要先照抄：`service/download/installable.rs`（删 `Nodejs`/`Git`）、`bridge/lifecycle.rs`（删 node/git 就绪判定）、`config/dependencies.rs`（收敛键集合）、`resources/manifest.jsonc`（删 `engines.node`/`engines.git`、`dsh` 入口改 asar 内路径）、`service/core/*`（改 `client.node`）。

## 顺序

1. 迁 `store/` 六个模块 + `types/` + `apis/`。
2. 迁 `layout/` 全量并完成 iframe→标签改写，`navbar` 补回 v1 结构；此步结束主窗口应能真实加载 dsh 页面。
3. 迁 `ui/` 18 个与 `hooks/` 7 个。
4. 迁 `pet/` 独立窗口（含 `desktop/pet*`、`bridge/pet`）。
5. 按 `bridge/` 批次迁 `service/`，同批完成 node/git 剔除与 asar 改造。

## 验证门禁

```sh
pnpm typecheck
pnpm exec eslint apps/desktop/src
pnpm exec vitest run --project unit apps/desktop --sequence.shuffle
cargo check --manifest-path apps/desktop/src-tauri/Cargo.toml --all-targets
cargo test --manifest-path apps/desktop/src-tauri/Cargo.toml --lib --locked -- --test-threads=1
```

动 `crates/` 时跑该子仓库的 `cargo test --locked` 与 `pnpm run test:checks`，行号变动后跑 `node tests/checks/coverage.mjs --write`，确认提交 CI 通过再更新指针。改 Electron 依赖版本要同步三个锁文件：根 `Cargo.lock`、`tests/assets/transparent/Cargo.lock`、`example/src-tauri/Cargo.lock`。

## 风险

- `service/` 59190 行且与 `bridge/`、前端 store 强耦合，按竖切推进。
- asar：未压缩约 400 MiB，磁盘校验按解压后体积；只有 Electron 打过补丁的 `fs` 能读；`node-pty` 的 `spawn-helper` 必须单独解到磁盘，否则「能启动、一开终端就失败」。
- 删 `use-appearance` 后透明度/模糊/配色投影不再由前端投影；若产品仍要这些设置，改由 CSS 变量或主进程实现，别留死开关。
- `theme.rs` 的 `read_dsh_theme(home, profile)` 参数入口已保留但未接线，迁 `service/profile` 时接回，否则主题只跟随系统。
- Windows 上 `cargo test` 报「failed to remove smoke.exe」时，是上一次测试的 Electron 进程还在；按 PID 清理自己的测试树，不要按镜像名批量杀。
