//! 内嵌 dsh 页面的「启动期外观」引导：在 document-start 注册一个只做外观收发的
//! 轻量接收器，向宿主请求 boot 页 CSS 并挂到独立 head style 上。
//!
//! 需求来自官方 boot 页（HARNESS + Loading plugins…）：它由内核在插件加载**之前**
//! 绘出，外观插件此时还没激活，所以插件侧的 boot 透明规则来不及生效；而 boot 页的
//! 背景表达式是 `var(--dsw-alias-bg-base, var(--dsh-boot-bg, Canvas))`，只要 theme
//! 的 `--dsw-alias-bg-base` 存在，仅设 `--dsh-boot-bg` 就会被压掉。因此透明必须由宿主
//! 在文档创建时就下发给帧内，且只能画一层 alpha（body 与 boot 同时上 alpha 会得到
//! 0.7×0.7≈0.91 的错误浓度，见 `shared/appearance.ts::appearanceBootCss`）。
//!
//! 与 `plugin_boot.js.inc` 的分工：那个脚本负责健康探活（splash stalled/ready/failed）
//! 与帧身份申报，生命周期随应用挂载而永久停止；本模块只管外观，且**永不停止**——
//! 重新加载、bfcache 还原、运行期改设置都需要重新握手。两者互不依赖。
//!
//! 安全边界：脚本只接受 `event.source === window.parent` 且 `source === 'dsh-desktop'`
//! 的宿主消息，只写一个独立 `<style>`（不动 boot 节点的 DOM，避免 BootHandoff
//! hydration 不匹配），不读用户配置也不做任何持久化；回发地址用帧内自带的同源
//! origin（宿主与内嵌服务同源），不引入任何宿主侧配置往返。

/// iframe 内：启动期外观请求/应用/确认（document-start 注入，幂等）。
pub(crate) const APPEARANCE_BOOTSTRAP_JS: &str = include_str!("appearance.js.inc");

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn appearance_bootstrap_keeps_the_boot_handshake_idempotent_and_document_only() {
        assert!(APPEARANCE_BOOTSTRAP_JS.contains("window.__dsh_appearance_bootstrap__"));
        assert!(APPEARANCE_BOOTSTRAP_JS.contains("if (window.parent !== window.top) return"));
        assert!(APPEARANCE_BOOTSTRAP_JS.contains("dsh://appearance:request"));
        assert!(APPEARANCE_BOOTSTRAP_JS.contains("dsh://appearance:applied"));
        assert!(APPEARANCE_BOOTSTRAP_JS.contains("dsh-tauri:boot-appearance"));
        assert!(APPEARANCE_BOOTSTRAP_JS.contains("document.head || document.documentElement"));
        assert!(APPEARANCE_BOOTSTRAP_JS.contains("event.source !== window.parent"));
        assert!(APPEARANCE_BOOTSTRAP_JS.contains("data.source !== 'dsh-desktop'"));
        assert!(APPEARANCE_BOOTSTRAP_JS.contains("pageshow"));
        // 宿主是 tauri://localhost，帧内是 http://127.0.0.1:<port>，跨源回发只能用 '*'；
        // 用 location.origin 当 targetOrigin 会让消息静默丢失（外观插件的既有教训）。
        assert!(APPEARANCE_BOOTSTRAP_JS.contains("postMessage({ source: 'dsh-desktop', type: 'dsh://appearance:request' }, '*')"));
        assert!(APPEARANCE_BOOTSTRAP_JS.contains("postMessage({ source: 'dsh-desktop', type: 'dsh://appearance:applied' }, '*')"));
        assert!(!APPEARANCE_BOOTSTRAP_JS.contains("window.location.origin"));
        // boot 节点只由官方 React 树负责，注入脚本不许碰它的 DOM / 内联样式。
        assert!(!APPEARANCE_BOOTSTRAP_JS.contains("data-dsh-boot"));
        assert!(!APPEARANCE_BOOTSTRAP_JS.contains("style.background"));
        assert!(!APPEARANCE_BOOTSTRAP_JS.contains("location.reload"));
    }
}
