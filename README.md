<p align="center">
  <a href="https://github.com/dsh-tauri/deepseek-harness-desktop">
    <img src="public/favicon.svg" width="96" alt="DeepSeek Harness Desktop" />
  </a>
</p>

<h1 align="center">DeepSeek Harness 桌面版</h1>

<p align="center">
  在桌面上运行 <a href="https://github.com/deepseek-ai/deepseek-harness">DeepSeek Harness</a> ——<br />
  无需手动安装 Node.js、pnpm 或 Docker；普通安装首次启动可能需要联网初始化。
</p>

<p align="center">
  <a href="https://github.com/dsh-tauri/deepseek-harness-desktop/releases">
    <img src="https://img.shields.io/github/v/release/dsh-tauri/deepseek-harness-desktop?style=flat-square&label=release&color=4D6BFE" alt="Release" />
  </a>
  <img src="https://img.shields.io/github/downloads/dsh-tauri/deepseek-harness-desktop/total?style=flat-square&label=downloads&color=4D6BFE" alt="Downloads" />
  <img src="https://img.shields.io/github/stars/dsh-tauri/deepseek-harness-desktop?style=flat-square&label=stars&color=4D6BFE" alt="Stars" />
  <img src="https://img.shields.io/github/license/dsh-tauri/deepseek-harness-desktop?style=flat-square&label=license&color=4D6BFE" alt="MIT License" />
  <img src="https://img.shields.io/badge/Windows%20%7C%20macOS%20%7C%20Linux-black?style=flat-square" alt="Windows | macOS | Linux" />
  <img src="https://img.shields.io/badge/dsh-0.2.0--rc.2-4D6BFE?style=flat-square" alt="dsh 0.2.0-rc.2" />
</p>

<p align="center">
  <samp><a href="./README.en.md">English</a> · <a href="./README.es.md">Español</a> · <a href="https://dshtauri.mintlify.site">文档</a> · <strong>中文</strong></samp>
</p>

<p align="center">
  <a href="https://trendshift.io/repositories/151676?utm_source=trendshift-badge&amp;utm_medium=badge&amp;utm_campaign=badge-trendshift-151676" target="_blank" rel="noopener noreferrer"><img src="https://trendshift.io/api/badge/trendshift/repositories/151676/daily?language=Rust" alt="dsh-tauri%2Fdeepseek-harness-desktop | Trendshift" width="250" height="55"/></a>
</p>

<p align="center">
  <a href="docs/PREVIEW.md">
    <img src="./docs/images/hero-zh.png" width="100%" alt="DSH Desktop 中文宣传横幅" />
  </a>
</p>

## 功能

- 🪶 **原生桌面** — Tauri 2 + React 19，内嵌本地 Harness Web 界面。
- 🔄 **运行时管理** — 安装依赖、选择内核版本，并提供桌面端与内核更新入口。
- 🧩 **插件管理** — 12 个内置插件；支持本地路径安装、内置插件停用、社区插件安装、升级、卸载与错误查看。
- 🗂️ **档案配置** — 分别管理插件与设置；档案不是操作系统安全沙箱。
- 💽 **数据目录** — 安装时（Windows 安装包）可选择数据存放位置；Windows / macOS / Linux 都能在设置里整体迁移与回滚。
- ⌨️ **命令行集成** — 通过托管 shim 提供 `dsh` / `pnpm`，不是全局 npm 内核安装。
- 🐾 **桌宠** — Pets / Codex 资源管理、资源包导入与会话状态展示；预设素材来自远端。
- 🎨 **个性化** — 8 种配色、终端模式与原生透明，可一键恢复默认。

## 内置插件

随桌面资源分发的 12 个第一方插件：

| 插件 | 包标识 | 用途 |
| --- | --- | --- |
| [DSH Tauri](https://github.com/dsh-tauri/deepseek-harness-desktop/tree/main/packages/dsh-tauri) | `dsh-tauri` | 桌面壳与 Harness 通信 |
| [DSH Tauri UI](https://github.com/dsh-tauri/deepseek-harness-desktop/tree/main/packages/dsh-tauri-ui) | `dsh-tauri-ui` | 桌面设置界面 |
| [DSH Tauri Mobile UI](https://github.com/dsh-tauri/deepseek-harness-desktop/tree/main/packages/dsh-tauri-mobile-ui) | `dsh-tauri-mobile-ui` | 触屏布局与移动端偏好设置 |
| [DSH Tauri Worktree](https://github.com/dsh-tauri/deepseek-harness-desktop/tree/main/packages/dsh-tauri-worktree) | `dsh-tauri-worktree` | 会话 Git 工作树与检出 |
| [DSH Tauri Extension](https://github.com/dsh-tauri/deepseek-harness-desktop/tree/main/packages/dsh-tauri-extension) | `dsh-tauri-extension` | 官方插件管理、可选市场、Skills 与 MCP |
| [DSH Tauri Scheduler](https://github.com/dsh-tauri/deepseek-harness-desktop/tree/main/packages/dsh-tauri-scheduler) | `dsh-tauri-scheduler` | 定时任务与执行记录 |
| [DSH Tauri Archive](https://github.com/dsh-tauri/deepseek-harness-desktop/tree/main/packages/dsh-tauri-archive) | `dsh-tauri-archive` | 聊天归档与恢复 |
| [DSH Tauri Pet](https://github.com/dsh-tauri/deepseek-harness-desktop/tree/main/packages/dsh-tauri-pet) | `dsh-tauri-pet` | 桌宠与活动状态 |
| [DSH Tauri Rightclick Menu](https://github.com/dsh-tauri/deepseek-harness-desktop/tree/main/packages/dsh-tauri-rightclick) | `dsh-tauri-rightclick` | 会话、工作区与正文右键菜单 |
| [DSH Tauri Model](https://github.com/dsh-tauri/deepseek-harness-desktop/tree/main/packages/dsh-tauri-model) | `dsh-tauri-model` | 模型选择与参数 |
| [DSH Tauri SSH](https://github.com/dsh-tauri/deepseek-harness-desktop/tree/main/packages/dsh-tauri-ssh) | `dsh-tauri-ssh` | SSH 远端 Harness 连接与同步 |
| [DSH Tauri Notification](https://github.com/dsh-tauri/deepseek-harness-desktop/tree/main/packages/dsh-tauri-notification) | `dsh-tauri-notification` | 会话通知与交互操作 |

`dsh-tauri-experimental` 为可选实验包，默认关闭，不计入上述 12 个内置插件。

## 可选预设

启动引导提供以下 8 个社区插件，按需安装；前 5 个标记为推荐，最后 3 个需主动选择。

| 插件 | 包标识 | 用途 |
| --- | --- | --- |
| [DSH Market](https://github.com/dsh-market/dsh-market) | `dshmarket` | 社区插件市场 |
| [DSH Better Sidebar](https://github.com/omdsh-dev/DSH-better-sidebar) | `dsh-better-sidebar` | 按会话隔离的编辑器侧栏 |
| [DSH Rewind](https://github.com/SiriLee/dsh-rewind) | `dsh-rewind-plugin` | 对话回退与工作区备份 |
| [DSH Bridge](https://github.com/wenbin-wb/dsh-bridge) | `@wenbin_wb/dsh-bridge` | 远程访问、隧道与机器人接入 |
| [DSH IM](https://github.com/xmanrui/dsh-im) | `@xmanrui/dsh-im` | IM 渠道与机器人管理 |
| [Billion Context](https://github.com/ranxianglei/billion-context) | `billion-context` | 上下文压缩与历史恢复 |
| [DSH PDF Reader](https://github.com/SkanderBog/dsh-pdf-reader) | `dsh-pdf-reader` | 按页 PDF 阅读、本地 OCR、引用、搜索与视觉裁剪 |
| [DSH Context Manager](https://github.com/SkanderBog/dsh-context-manager) | `dsh-context-manager` | 审阅并选择性压缩较早的完整历史组 |

新增或更新预设请提交 [Issue](https://github.com/dsh-tauri/deepseek-harness-desktop/issues)；可用版本由清单中的内核兼容规则决定。

## 快速开始

从 [Releases](https://github.com/dsh-tauri/deepseek-harness-desktop/releases) 下载对应平台与架构的安装包：

| 平台 | 要求 | 安装包 |
| --- | --- | --- |
| Windows | Windows 10+、WebView2 | x64 `.exe` / `.msi` |
| macOS | macOS 12+；Web 界面需 Safari 17.4+ 能力 | Intel / Apple Silicon `.dmg` |
| Linux | WebKit2GTK 4.1 运行库 | x64 `.AppImage` / `.deb` |

macOS 也可通过 Homebrew 安装：

```bash
brew install dsh-tauri/desktop/deepseek-harness
```

- 普通安装包首次启动需联网下载缺失的运行时与内核；Git 功能需要可用的 Git。
- 仅在对应 Release 实际发布含运行时与内核资源的 `Bundle` 时，才可使用该捆绑包；正式版 `0.22.1` 提供 Bundle 资产（`_Bundle_*.exe`、`_Bundle_*.deb`、两种 `_Bundle_*.dmg`）。
- 本地运行不等于完全离线：模型服务、插件安装、更新与桌宠预设素材仍可能联网。
- Linux 显示、Wayland、AppImage 与权限问题的处理见[安装与故障排查文档](https://dshtauri.mintlify.site)。

### 桌面代理

- 在**配置 → 应用 → 代理地址**设置 HTTP、HTTPS、SOCKS5 或 SOCKS5H 代理。
- 仅用于桌面端依赖/内核下载、更新检查及插件元数据，不修改模型请求或插件子进程的网络配置。
- 留空沿用系统/环境代理；保存后对新请求生效，可重试失败的下载。
- SOCKS5H 通过代理解析目标域名；本机回环连接始终直连。
- 代理 URL 含账号密码，保存在本机配置。远程认证优先使用 HTTPS 代理；HTTP 代理不会加密认证信息，即使目标网站使用 HTTPS。

## 运行方式

Windows 启动失败时会通过原生对话框显示具体错误。若出现 `STARTUP_LOW_INTEGRITY`：

- 进程低于 Medium 完整性级别，无法写入正常用户数据；更新后的程序可能继承安装目录的 Low 标签。
- 用 `icacls` 检查目录与程序；将可信安装恢复为 Medium，或重新安装到正常目录。
- 以管理员身份运行不会移除 Low 标签限制；无需删除会话或修改 `DSH_HOME`。

| 当前基线 | 版本 |
| --- | --- |
| 桌面端 | `0.22.1` |
| 推荐 Harness 内核 | `0.2.0-rc.2` |
| 声明的内核最低版本 | `0.1.5-rc.1`，不保证所有插件兼容 |
| Node.js 运行时 | `22.22.0` |
| pnpm | `11.7.0` |

- Rust 后端管理依赖与 Harness 进程，React WebView 内嵌其界面；正式版默认地址为 `http://127.0.0.1:3080`，端口占用时可能调整。
- 内核与预设选择遵循[资源清单](<./src-tauri/resources/manifest.jsonc>)及插件声明的版本范围；不保证任意最新上游版本都兼容。
- 正式版默认启用 CLI 集成：Windows 更新用户 PATH，macOS / Linux 按需在 Bash / Zsh 配置中加入托管 PATH 块。需重新打开终端，其他 shell 可能需要手动配置。

## 交流

- [加入 Discord 社区](https://discord.gg/RT9As6Cj8B)

<table>
  <tr>
    <td align="center"><strong>QQ 群</strong><br /><img src="./docs/images/community/qq-qrcode.jpg" width="360" alt="QQ 群二维码" /></td>
    <td align="center"><strong>微信群(已满,请先加我微信) -> </strong><br /><img src="./docs/images/community/wx-qrcode.png" width="360" alt="微信群二维码" /></td>
    <td align="center"><strong>个人微信</strong><br /><img src="https://github.com/user-attachments/assets/c1d6e493-b608-4a6d-b387-dfcaa37ccfdc" width="360" alt="微信群二维码" /></td>
  </tr>
</table>

## 开发

参见[中文开发指南](<./docs/DEVELOPMENT.zh.md>)或[英文开发指南](<./docs/DEVELOPMENT.md>)。功能详情见[在线文档](https://dshtauri.mintlify.site)。

## 说明

> [!WARNING]
> **开发预览** — 上游 `dsh` 仍在快速迭代，可能存在破坏性变更；升级前请确认兼容性。

> [!NOTE]
> **安全声明** — `dsh` 具备本地代码执行能力。仅供学习 / 研究 / 测试，请在可信、隔离的环境中使用。

## 相关项目

- [deepseek-harness](https://github.com/deepseek-ai/deepseek-harness) — 上游 `dsh` agent 平台
- [deepseek-harness-pkg](https://github.com/dsh-tauri/deepseek-harness-pkg) — 预打包 Harness 发行版与下载源
- [dsh-pet](https://github.com/PC2005-cloud/dsh-pet) · [dsh-pet-mov](https://github.com/dsh-tauri/dsh-pet-mov) · [dsh-pet-component](https://github.com/hairyf/dsh-pet-component) — 桌宠素材与渲染组件
- [dsh-plugin-codex-pets](https://github.com/Skylarking/dsh-plugin-codex-pets) · [BongoCat](https://github.com/ayangweb/BongoCat) · [dsh-dafeiyu](https://github.com/QCYTSN/dsh-dafeiyu) · [codex-to-dsh-pet](https://github.com/Signalight/codex-to-dsh-pet) — 桌宠参考项目

## License

[MIT](<./LICENSE>)，附加[非商用条款](<./LICENSE.details>) © deepseek-harness-desktop contributors
