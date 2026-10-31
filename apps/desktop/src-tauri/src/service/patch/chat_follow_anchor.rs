//! 会话滚动跟随的 `overflow-anchor` 补丁（issue #1187）。
//!
//! WKWebView 里只要锚定规则带 `:has()`，滚动跟随期间每次节点移除都要对整棵会话子树
//! 重做选择器匹配（采样中占比近四成，表现为滚动和指针移动都慢半拍）。跟随意图本来
//! 就由 JS 持有，这里把它镜像到外层滚动容器，让 CSS 改用普通属性选择器；Chrome 不受
//! 影响是因为 Blink 的 `:has()` 失效范围更窄。
//!
//! 镜像值要按滚动容器聚合，而 `owners` 是 WeakMap 不可遍历，故另用 `followers` 记录
//! 每个滚动容器上的跟随者集合；已发布的旧版补丁直接遍历 WeakMap，必须被就地修复。

use std::path::Path;

use crate::utils::{patch_core_file, patch_dsh, PatchOutcome};

const CLIENT: &str = "node_modules/@deepseek-ai/dsh-client-ui-chat/lib/client.js";

const ORIGINAL_CSS: &str =
    r#"[data-conversation-scroll]:has(.EvIC1a_root[data-chat-following-tail])"#;
const PATCHED_CSS: &str = r#"[data-conversation-scroll][data-chat-following-tail]"#;

const ORIGINAL_STATIC: &str = r#"			static owners = /* @__PURE__ */ new WeakMap();"#;
const PATCHED_STATIC: &str = r#"			static owners = /* @__PURE__ */ new WeakMap();
			static followers = /* @__PURE__ */ new WeakMap();"#;

const ORIGINAL_BIND: &str = r#"			bind(element) {
				ScrollFollow.owners.set(element, this);
				return () => {
					if (ScrollFollow.owners.get(element) === this) ScrollFollow.owners.delete(element);
				};
			}"#;
const PATCHED_BIND: &str = r#"			bind(element) {
				this.element = element;
				this.scrollport = element.closest("[data-conversation-scroll]");
				ScrollFollow.owners.set(element, this);
				const scrollport = this.scrollport;
				if (scrollport !== void 0 && scrollport !== null) {
					let followers = ScrollFollow.followers.get(scrollport);
					if (followers === void 0) ScrollFollow.followers.set(scrollport, followers = /* @__PURE__ */ new Set());
					followers.add(this);
				}
				this.followAnchor();
				return () => {
					const followers = scrollport === void 0 || scrollport === null ? void 0 : ScrollFollow.followers.get(scrollport);
					if (followers !== void 0) followers.delete(this);
					const owned = ScrollFollow.owners.get(element) === this;
					if (owned) ScrollFollow.owners.delete(element);
					this.followAnchor();
					if (owned) {
						this.element = void 0;
						this.scrollport = void 0;
					}
				};
			}"#;
const LEGACY_BIND: &str = r#"			bind(element) {
				this.element = element;
				this.scrollport = element.closest("[data-conversation-scroll]");
				ScrollFollow.owners.set(element, this);
				this.followAnchor();
				return () => {
					if (ScrollFollow.owners.get(element) === this) {
						ScrollFollow.owners.delete(element);
						this.followAnchor();
						this.element = void 0;
						this.scrollport = void 0;
					}
				};
			}"#;

const ORIGINAL_FOLLOW: &str = r#"			setFollowing(active) {
				this.following = active;
				if (!active) this.target = null;
			}"#;
const PATCHED_FOLLOW: &str = r#"			setFollowing(active) {
				this.following = active;
				if (!active) this.target = null;
				this.followAnchor();
			}
			followAnchor() {
				const scrollport = this.scrollport;
				if (scrollport === void 0 || scrollport === null) return;
				let following = false;
				const followers = ScrollFollow.followers.get(scrollport);
				if (followers !== void 0) followers.forEach((follower) => {
					if (follower.following) following = true;
				});
				if (scrollport.hasAttribute("data-chat-following-tail") === following) return;
				if (following) scrollport.setAttribute("data-chat-following-tail", "");
				else scrollport.removeAttribute("data-chat-following-tail");
			}"#;
const LEGACY_FOLLOW: &str = r#"			setFollowing(active) {
				this.following = active;
				if (!active) this.target = null;
				this.followAnchor();
			}
			followAnchor() {
				const scrollport = this.scrollport;
				if (scrollport === void 0 || scrollport === null) return;
				let following = false;
				ScrollFollow.owners.forEach((owner) => {
					if (owner.following && owner.scrollport === scrollport) following = true;
				});
				if (scrollport.hasAttribute("data-chat-following-tail") === following) return;
				if (following) scrollport.setAttribute("data-chat-following-tail", "");
				else scrollport.removeAttribute("data-chat-following-tail");
			}"#;

fn patch_source(source: &str) -> PatchOutcome {
    let anchors: [(&[&str], &str); 4] = [
        (&[ORIGINAL_CSS], PATCHED_CSS),
        (&[ORIGINAL_STATIC], PATCHED_STATIC),
        (&[ORIGINAL_BIND, LEGACY_BIND], PATCHED_BIND),
        (&[ORIGINAL_FOLLOW, LEGACY_FOLLOW], PATCHED_FOLLOW),
    ];
    let mut patched = source.to_string();
    let mut changed = false;
    for (candidates, replacement) in anchors {
        match patched.matches(replacement).count() {
            1 => continue,
            0 => {}
            _ => return PatchOutcome::AnchorMissing,
        }
        let mut candidate = None;
        for start in candidates {
            match patched.matches(*start).count() {
                0 => {}
                1 if candidate.is_none() => candidate = Some(*start),
                _ => return PatchOutcome::AnchorMissing,
            }
        }
        let Some(candidate) = candidate else {
            return PatchOutcome::AnchorMissing;
        };
        patched = patched.replacen(candidate, replacement, 1);
        changed = true;
    }
    if changed {
        PatchOutcome::Patched(patched)
    } else {
        PatchOutcome::AlreadyPatched
    }
}

pub fn apply_at(core_dir: &Path) -> Result<(), String> {
    patch_core_file(core_dir, CLIENT, patch_source)
}

pub fn apply(app_handle: &tauri::AppHandle) -> Result<(), String> {
    patch_dsh(app_handle, CLIENT, patch_source)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::process::{Command, Stdio};
    use std::time::{Duration, Instant};

    const FIXTURE: &str = include_str!("../../../../test/fixtures/chat-follow-anchor.txt");

    fn released() -> String {
        format!(
            "<script>{}{}{}{}</script>",
            ORIGINAL_CSS, ORIGINAL_STATIC, ORIGINAL_BIND, ORIGINAL_FOLLOW
        )
    }

    fn shipped() -> String {
        format!(
            "<script>{}{}{}{}</script>",
            PATCHED_CSS, ORIGINAL_STATIC, LEGACY_BIND, LEGACY_FOLLOW
        )
    }

    #[test]
    fn patches_released_source_once() {
        let PatchOutcome::Patched(patched) = patch_source(&released()) else {
            panic!("released source should patch");
        };
        assert!(patched.contains(PATCHED_CSS));
        assert!(patched.contains(PATCHED_STATIC));
        assert!(patched.contains(PATCHED_BIND));
        assert!(patched.contains(PATCHED_FOLLOW));
        assert!(!patched.contains(":has(.EvIC1a_root"));
        assert!(!patched.contains(ORIGINAL_BIND));
        assert!(!patched.contains(ORIGINAL_FOLLOW));
        assert!(matches!(
            patch_source(&patched),
            PatchOutcome::AlreadyPatched
        ));
    }

    #[test]
    fn repairs_the_shipped_patch_that_crashes_on_weakmap() {
        let shipped = shipped();
        assert!(shipped.contains("ScrollFollow.owners.forEach"));
        let PatchOutcome::Patched(repaired) = patch_source(&shipped) else {
            panic!("shipped patch should be repaired");
        };
        assert!(!repaired.contains("ScrollFollow.owners.forEach"));
        assert!(!repaired.contains(LEGACY_BIND));
        assert!(!repaired.contains(LEGACY_FOLLOW));
        assert!(repaired.contains(PATCHED_STATIC));
        assert!(matches!(
            patch_source(&repaired),
            PatchOutcome::AlreadyPatched
        ));
    }

    #[test]
    fn changed_css_anchor_is_skipped() {
        let source = released().replace(ORIGINAL_CSS, "");
        assert!(matches!(patch_source(&source), PatchOutcome::AnchorMissing));
    }

    #[test]
    fn changed_follow_method_is_skipped() {
        let source = released().replace("this.target = null;", "this.target = void 0;");
        assert!(matches!(patch_source(&source), PatchOutcome::AnchorMissing));
    }

    #[test]
    fn changed_owner_static_is_skipped() {
        let source = released().replace(ORIGINAL_STATIC, "");
        assert!(matches!(patch_source(&source), PatchOutcome::AnchorMissing));
    }

    #[test]
    fn mixed_original_and_shipped_states_are_skipped() {
        let source = format!("{}{}", released(), shipped());
        assert!(matches!(patch_source(&source), PatchOutcome::AnchorMissing));
    }

    #[test]
    fn repaired_core_passes_the_javascript_contract() {
        let core = tempfile::tempdir_in(env!("CARGO_MANIFEST_DIR")).unwrap();
        let target = core.path().join(CLIENT);
        std::fs::create_dir_all(target.parent().unwrap()).unwrap();
        let shipped = format!(
            "const css = \"{}\";{}",
            PATCHED_CSS,
            FIXTURE.replacen(ORIGINAL_BIND, LEGACY_BIND, 1).replacen(
                ORIGINAL_FOLLOW,
                LEGACY_FOLLOW,
                1
            )
        );
        let PatchOutcome::Patched(repaired) = patch_source(&shipped) else {
            panic!("shipped fixture should be repaired");
        };
        assert!(matches!(
            patch_source(&repaired),
            PatchOutcome::AlreadyPatched
        ));
        std::fs::write(&target, &repaired).unwrap();

        let script = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../test/fixtures/chat-follow-anchor-contract.mjs");
        let mut child = Command::new("node")
            .arg(script)
            .arg(&target)
            .stdin(Stdio::null())
            .spawn()
            .expect("node is required for the repaired JavaScript contract");
        let deadline = Instant::now() + Duration::from_secs(20);
        loop {
            if let Some(status) = child.try_wait().unwrap() {
                assert!(
                    status.success(),
                    "repaired JavaScript contract failed: {status}"
                );
                break;
            }
            if Instant::now() >= deadline {
                child.kill().unwrap();
                child.wait().unwrap();
                panic!("repaired JavaScript contract timed out");
            }
            std::thread::sleep(Duration::from_millis(20));
        }
    }
}
