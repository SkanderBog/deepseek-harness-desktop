use std::path::Path;

use crate::utils::{patch_core_file, patch_dsh, PatchOutcome};

const CLIENT: &str = "node_modules/@deepseek-ai/dsh-client-ui-layout/lib/client.js";
const PATCH_MARKER: &str = "dsh-tauri: mobile sidebar slot props";
const ORIGINAL: &str = r#"const sidebar = (0, react.useMemo)(() => renderSlot("sidebar", {
				collapsed: sidebarCollapsed,
				width: cols.sidebar
			}), [
				renderSlot,
				sidebarCollapsed,
				cols.sidebar
			]);"#;
/// 首版补丁把 `isDshMobileSidebar()` 放在模块级、靠 `viewport` 依赖重算；已经打过首版补丁的运行时
/// 必须整体升级，否则会永远命中 AnchorMissing 而停留在旧行为上。
const LEGACY_HELPER: &str = r#"function isDshMobileSidebar() {
    return typeof document !== "undefined" && document.documentElement.hasAttribute("data-dsh-mobile-ui") && typeof window !== "undefined" && typeof window.matchMedia === "function" && window.innerWidth < 1024 && ["(hover: none)", "(any-pointer: coarse)", "(any-hover: none)"].every(query => window.matchMedia(query).matches);
}
"#;
const LEGACY_TARGET: &str = r#"/* dsh-tauri: mobile sidebar slot props */
			const sidebar = (0, react.useMemo)(() => renderSlot("sidebar", {
				collapsed: isDshMobileSidebar() ? false : sidebarCollapsed,
				width: isDshMobileSidebar() ? Math.min(320, Math.floor(window.innerWidth * .82)) : cols.sidebar
			}), [
				renderSlot,
				sidebarCollapsed,
				cols.sidebar,
				viewport
			]);"#;
/// 上一版把 `(hover: none)`/`(any-hover: none)` 也放进合取：vivo V2353A（Android 14 /
/// Chrome 116）在 360px 视口把这两条报成 false，`every()` 因此恒 false，宽度恒为 0，
/// 强制展开分支从不执行，于是出现「宽容器装折叠图标栏」。下面保留该版原文，
/// 仅用于把已经打过它的运行时升级上来。
const PATCHED_TARGET_V1: &str = r#"/* dsh-tauri: mobile sidebar slot props */
			const mobileSidebarWidth = (0, react.useSyncExternalStore)(subscribeDshMobileSidebar, getDshMobileSidebarWidth, () => 0);
			const sidebar = (0, react.useMemo)(() => renderSlot("sidebar", {
				collapsed: mobileSidebarWidth > 0 ? false : sidebarCollapsed,
				width: mobileSidebarWidth > 0 ? mobileSidebarWidth : cols.sidebar
			}), [
				renderSlot,
				sidebarCollapsed,
				cols.sidebar,
				mobileSidebarWidth
			]);
			function getDshMobileSidebarWidth() {
				return typeof document !== "undefined" && typeof window !== "undefined" && typeof window.matchMedia === "function" && document.documentElement.hasAttribute("data-dsh-mobile-sidebar") && window.innerWidth < 1024 && ["(hover: none)", "(any-pointer: coarse)", "(any-hover: none)"].every(query => window.matchMedia(query).matches) ? Math.min(320, Math.floor(window.innerWidth * .82)) : 0;
			}
			function subscribeDshMobileSidebar(onChange) {
				if (typeof document === "undefined" || typeof window === "undefined") return () => {};
				const observer = new MutationObserver(onChange);
				observer.observe(document.documentElement, { attributes: true, attributeFilter: ["data-dsh-mobile-sidebar"] });
				window.addEventListener("resize", onChange);
				return () => {
					observer.disconnect();
					window.removeEventListener("resize", onChange);
				};
			}"#;
/// 只有插件的 `<html data-dsh-mobile-sidebar>` 所有权标记 + 窄屏触摸媒体查询同时成立才放宽侧栏；
/// 订阅标记/尺寸变化自行重算，**不写布局状态**（写状态会改掉桌面端的侧栏宽度偏好）。
/// 判据只保留 `pointer` 两条，与前端 `MOBILE_POINTER_QUERIES` 保持一致。
const PATCHED_TARGET: &str = r#"/* dsh-tauri: mobile sidebar slot props */
			const mobileSidebarWidth = (0, react.useSyncExternalStore)(subscribeDshMobileSidebar, getDshMobileSidebarWidth, () => 0);
			const sidebar = (0, react.useMemo)(() => renderSlot("sidebar", {
				collapsed: mobileSidebarWidth > 0 ? false : sidebarCollapsed,
				width: mobileSidebarWidth > 0 ? mobileSidebarWidth : cols.sidebar
			}), [
				renderSlot,
				sidebarCollapsed,
				cols.sidebar,
				mobileSidebarWidth
			]);
			function getDshMobileSidebarWidth() {
				return typeof document !== "undefined" && typeof window !== "undefined" && typeof window.matchMedia === "function" && document.documentElement.hasAttribute("data-dsh-mobile-sidebar") && window.innerWidth < 1024 && ["(pointer: coarse)", "(any-pointer: coarse)"].every(query => window.matchMedia(query).matches) ? Math.min(320, Math.floor(window.innerWidth * .82)) : 0;
			}
			function subscribeDshMobileSidebar(onChange) {
				if (typeof document === "undefined" || typeof window === "undefined") return () => {};
				const observer = new MutationObserver(onChange);
				observer.observe(document.documentElement, { attributes: true, attributeFilter: ["data-dsh-mobile-sidebar"] });
				window.addEventListener("resize", onChange);
				return () => {
					observer.disconnect();
					window.removeEventListener("resize", onChange);
				};
			}"#;

fn patch_source(source: &str) -> PatchOutcome {
    if source.contains(PATCHED_TARGET) {
        return PatchOutcome::AlreadyPatched;
    }
    if source.matches(PATCHED_TARGET_V1).count() == 1 {
        return PatchOutcome::Patched(source.replacen(PATCHED_TARGET_V1, PATCHED_TARGET, 1));
    }
    let legacy = format!("{LEGACY_HELPER}{LEGACY_TARGET}");
    if source.matches(&legacy).count() == 1 {
        return PatchOutcome::Patched(source.replacen(&legacy, PATCHED_TARGET, 1));
    }
    if source.contains(PATCH_MARKER)
        || source.contains("isDshMobileSidebar")
        || source.contains("subscribeDshMobileSidebar")
    {
        return PatchOutcome::AnchorMissing;
    }
    if source.matches(ORIGINAL).count() != 1 {
        return PatchOutcome::AnchorMissing;
    }
    PatchOutcome::Patched(source.replacen(ORIGINAL, PATCHED_TARGET, 1))
}

pub fn apply_at(core_dir: &Path) -> Result<(), String> {
    patch_core_file(core_dir, CLIENT, patch_source)
}

pub fn apply(app_handle: &crate::AppHandle) -> Result<(), String> {
    patch_dsh(app_handle, CLIENT, patch_source)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture() -> String {
        ORIGINAL.to_owned()
    }

    #[test]
    fn widens_mobile_sidebar_without_mutating_layout_preferences() {
        let PatchOutcome::Patched(patched) = patch_source(&fixture()) else {
            panic!("expected patched source");
        };
        assert!(patched.contains(PATCH_MARKER));
        assert!(patched.contains(
            "(0, react.useSyncExternalStore)(subscribeDshMobileSidebar, getDshMobileSidebarWidth, () => 0)"
        ));
        assert!(patched.contains("collapsed: mobileSidebarWidth > 0 ? false : sidebarCollapsed"));
        assert!(patched.contains("width: mobileSidebarWidth > 0 ? mobileSidebarWidth : cols.sidebar"));
        assert!(patched.contains(r#"attributeFilter: ["data-dsh-mobile-sidebar"]"#));
        assert!(patched.contains("window.innerWidth < 1024"));
        assert!(patched.contains("window.matchMedia(query).matches"));
        assert!(patched.contains("Math.min(320, Math.floor(window.innerWidth * .82))"));
        assert!(!patched.contains("toggleSidebar"));
        assert!(!patched.contains("isDshMobileSidebar"));
        assert_eq!(patch_source(&patched), PatchOutcome::AlreadyPatched);
    }

    #[test]
    fn upgrades_the_legacy_viewport_dependent_patch() {
        let legacy = format!("{LEGACY_HELPER}{LEGACY_TARGET}");
        let PatchOutcome::Patched(patched) = patch_source(&legacy) else {
            panic!("expected the legacy patch to be upgraded");
        };
        assert!(!patched.contains("isDshMobileSidebar"));
        assert!(patched.contains("(0, react.useSyncExternalStore)"));
        assert!(patched.contains("mobileSidebarWidth > 0 ? false : sidebarCollapsed"));
        assert_eq!(patch_source(&patched), PatchOutcome::AlreadyPatched);
    }

    /// 真机回归：vivo V2353A 上 `(hover: none)`/`(any-hover: none)` 恒 false，上一版判据因此
    /// 永远返回 0，`collapsed` 停在官方值上，只剩 CSS 单独把容器撑宽。新版必须只依赖 pointer。
    #[test]
    fn drops_hover_queries_from_the_runtime_predicate() {
        assert!(!PATCHED_TARGET.contains("(hover: none)"));
        assert!(!PATCHED_TARGET.contains("(any-hover: none)"));
        assert!(PATCHED_TARGET.contains(
            r#"["(pointer: coarse)", "(any-pointer: coarse)"].every(query => window.matchMedia(query).matches)"#
        ));
    }

    #[test]
    fn upgrades_the_previous_hover_dependent_patch() {
        let PatchOutcome::Patched(patched) = patch_source(PATCHED_TARGET_V1) else {
            panic!("expected the previous patch to be upgraded");
        };
        assert!(!patched.contains("(hover: none)"));
        assert!(!patched.contains("(any-hover: none)"));
        assert!(patched.contains(r#"["(pointer: coarse)", "(any-pointer: coarse)"]"#));
        assert_eq!(patch_source(&patched), PatchOutcome::AlreadyPatched);
    }

    #[test]
    fn skips_missing_or_ambiguous_sidebar_anchors() {
        assert_eq!(patch_source(""), PatchOutcome::AnchorMissing);
        assert_eq!(
            patch_source(&format!("{}{}", ORIGINAL, ORIGINAL)),
            PatchOutcome::AnchorMissing
        );
        assert_eq!(
            patch_source(&fixture().replace("width: cols.sidebar", "width: cols.main")),
            PatchOutcome::AnchorMissing
        );
        assert_eq!(patch_source(LEGACY_HELPER), PatchOutcome::AnchorMissing);
        assert_eq!(patch_source(LEGACY_TARGET), PatchOutcome::AnchorMissing);
        assert_eq!(
            patch_source(&format!("{PATCH_MARKER}\n{ORIGINAL}")),
            PatchOutcome::AnchorMissing
        );
        assert_eq!(
            patch_source(&format!("{LEGACY_HELPER}{ORIGINAL}")),
            PatchOutcome::AnchorMissing
        );
        assert_eq!(
            patch_source(&PATCHED_TARGET.replace("const sidebar", "const sidebarX")),
            PatchOutcome::AnchorMissing
        );
    }

    #[test]
    fn ignores_unrelated_mobile_marker_text() {
        let source = format!("// data-dsh-mobile-ui\n{ORIGINAL}");
        assert!(matches!(patch_source(&source), PatchOutcome::Patched(_)));
    }
}
