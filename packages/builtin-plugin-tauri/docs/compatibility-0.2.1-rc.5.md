# Harness 0.2.1-rc.5 兼容记录

推荐核心和开发依赖为 `0.2.1-rc.5`，最低支持为 `0.2.1-rc.1`。源码对照固定到 `dsh-v0.2.1-rc.5`（`b830ec950faba0a46dda564be3e2d854afed94f1`），并逐项比较 rc.1；历史派生来源保留。已核实打包发行 `dsh-0.2.1-rc.5-41234040595`，提供 Windows、Linux、macOS 的 x64/arm64 zip 和 asar.zip。

rc.4 到 rc.5 的源码差异为 368 个文件、+529/-399，主要实现变化是 reasoning 展示过滤前导 `<signature>`；它不把正文 `<thinking>` 转为思考通道，也未改变 ScrollFollow。

删除旧草稿恢复补丁及三个装配入口：rc.1 起 `ui-conversation/src/client/input/facade.ts` 已原生处理语义草稿、引用与命令恢复。删除鉴权的旧 `requestRejection` 回退，保留官方 `admit` 路径；只对嵌入环境的未认证活跃 peer 放行，保留跨源 403、disposed 和中止信号检查。删除不存在于支持版本的原生 `sessions.provideInfo` 探测，以及被 current 投影覆盖的重复 list 投影。清单删除 sidebar、rewind、im 的旧 0.1.7 核心 lane，12 个运行插件同步最低版本。

保留当前会话和输入动作桥：官方会话列表没有 current，输入动作仍从 `sessions.binding` 和 `uiSession.adapter.bindingSource` 读取。工作树切换依赖这些投影避免覆盖用户选择。

保留 SlotOutlet 导出、未分组 composer、SessionStore.remove、工作树关联、旧持久化视图容错、权限保存错误、减少动画、移动输入及侧栏、MCP 启动/OAuth、pi-ai 请求头和 thinking 补丁：rc.1 与 rc.5 仍有对应缺口。保留 Electron browser 注入与 CLI carrier、Node/ABI 和旧注入清理；上游 preload 不会自动运行于本壳。

本地验证：源码 typecheck；清单 12 项、最低门槛 5 项、插件安装 18 项、本地更新 3 项 Rust 测试通过；鉴权 7 项、元数据 2 项、核心更新 UI 16 项通过。会话桥 1 项回归及修改源码 lint 通过。未执行插件 Build；未验证真实模型请求、发布包压缩锚点及完整桌面浏览器交互，不能将源码对照等同于这些运行验证。
