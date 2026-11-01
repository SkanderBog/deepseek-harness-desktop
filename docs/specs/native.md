# DeepSeek Harness 移动端规范

## 范围

移动端是独立的 Expo / React Native 工程。

- 源码根：`apps/native/src/`
- Expo 项目根：`apps/native/`
- 桌面工程：`apps/desktop/`

## 技术栈与命令

Expo SDK 57、React Native 0.86.3、React 19、HeroUI Native、Uniwind、Expo Router。

| 用途 | 命令 |
| --- | --- |
| 开发 | `pnpm dev:native` |
| Android | `pnpm android` |
| iOS | `pnpm ios` |
| 单测 | `pnpm test:native` |
| 类型检查 | `pnpm typecheck:native` |
| Android JS 导出 | `pnpm build:native` |
| Android 原生工程生成 | `pnpm prebuild:native` |

本地导出和 prebuild 不等于 APK 构建或真机验收。

## 配置

- `apps/native/app.json`：Expo 标识、平台权限、插件、资源及路由根 `src/app`。
- `apps/native/app.config.js`：注入根目录 `THIRD_PARTY_NOTICES.md`，并维护 Android 大屏/折叠屏方向补丁。
- `apps/native/metro.config.js`：Uniwind CSS、Expo Metro 与 `@/` 别名解析。
- `apps/native/tsconfig.json`：Expo TypeScript 工程，`@/*` 指向 `src/*`。
- `config/vitest.native.config.ts`：单独的 native Vitest 工程，测试范围为 `src/**/*.test.{ts,tsx}`。
- `apps/native/package.json`：独立依赖与 Expo 脚本，入口 `./src/entry.ts`。
- `apps/native/assets/native/`：移动端图标、启动图及通知图标。

## 目录

`apps/native/src/` 按 Expo Router、apis、components、config、hooks、i18n、services、store、styles、test、ui、utils 分层。路由测试放在 `ui/`，避免 Expo Router 将测试识别为页面。WebView 容器与桥接协议留在 native 工程内。

## 测试边界

Native 测试使用独立的 Vitest 配置，不进入根 `unit`、`plugin`、`desktop` project。状态模块是进程级单例；涉及 store 或 i18n 的测试应按源码现有约定重置状态，并 mock Node 环境不兼容的 Expo 原生模块。既有权限、网络探测、认证与桥接行为以迁移后的代码和本地检查为准，不继承迁移前的测试通过声明。
