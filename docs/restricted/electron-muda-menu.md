# Electron Runtime：muda 菜单能力

状态：受限。

Electron Runtime 当前不保证 muda 菜单的原生窗口挂载、快捷键和回调能力。桌面实现暂不依赖这些能力；需要菜单功能时优先使用 Electron Menu / webContents 事件，待运行时补齐后再接入。
