# Linux、Wayland 与 GTK 互操作

状态：未验证。

Electron 窗口不提供旧 WebView2、WKWebView 或 GTK WebView 的进程内原生对象；不能沿用依赖嵌入式 WebView/GTK 对象的互操作假设。Linux、Wayland 下的窗口行为尚未完成发行版验收。

补齐条件：明确支持的发行版与会话类型，并在其打包环境验证窗口、输入法、文件对话框及原生扩展互操作。