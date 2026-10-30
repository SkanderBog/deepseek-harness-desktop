use std::path::Path;

use crate::AppHandle;
use crate::utils::{PatchOutcome, patch_core_file, patch_dsh};

const PATCH_MARKER: &str = "dsh-tauri-desktop: Codex thinking-as-text";

const PI_AI_COMPLETIONS_JS: &str =
    "node_modules/@earendil-works/pi-ai/dist/api/openai-completions.js";

const LOOP_ANCHOR: &str = "            for await (const chunk of openaiStream) {";
const LOOP_PATCHED: &str = r#"            /* dsh-tauri-desktop: Codex thinking-as-text */
            const thinkingTagOpen = "<thinking>";
            const thinkingTagClose = "</thinking>";
            let thinkingTagBuffer = "";
            let thinkingTagInside = false;
            let thinkingTagHead = true;
            const emitThinkingAwareContent = (text, thinking) => {
                if (text.length === 0)
                    return;
                const block = thinking ? ensureThinkingBlock() : ensureTextBlock();
                if (thinking)
                    block.thinking += text;
                else
                    block.text += text;
                stream.push({
                    type: thinking ? "thinking_delta" : "text_delta",
                    contentIndex: getContentIndex(block),
                    delta: text,
                    partial: output,
                });
            };
            const flushThinkingAwareContent = () => {
                emitThinkingAwareContent(thinkingTagBuffer, thinkingTagInside);
                thinkingTagBuffer = "";
                thinkingTagHead = false;
            };
            const feedThinkingAwareContent = (chunk) => {
                thinkingTagBuffer += chunk;
                for (;;) {
                    if (thinkingTagInside) {
                        const closeAt = thinkingTagBuffer.indexOf(thinkingTagClose);
                        if (closeAt === -1) {
                            const hold = Math.min(thinkingTagBuffer.length, thinkingTagClose.length - 1);
                            emitThinkingAwareContent(thinkingTagBuffer.slice(0, thinkingTagBuffer.length - hold), true);
                            thinkingTagBuffer = thinkingTagBuffer.slice(thinkingTagBuffer.length - hold);
                            return;
                        }
                        emitThinkingAwareContent(thinkingTagBuffer.slice(0, closeAt), true);
                        thinkingTagBuffer = thinkingTagBuffer.slice(closeAt + thinkingTagClose.length);
                        thinkingTagInside = false;
                        continue;
                    }
                    if (!thinkingTagHead) {
                        emitThinkingAwareContent(thinkingTagBuffer, false);
                        thinkingTagBuffer = "";
                        return;
                    }
                    const head = thinkingTagBuffer.replace(/^\s+/, "");
                    if (head.startsWith(thinkingTagOpen)) {
                        thinkingTagBuffer = head.slice(thinkingTagOpen.length);
                        thinkingTagInside = true;
                        thinkingTagHead = false;
                        continue;
                    }
                    if (head.length < thinkingTagOpen.length && thinkingTagOpen.startsWith(head)) {
                        thinkingTagBuffer = head;
                        return;
                    }
                    thinkingTagHead = false;
                }
            };
            for await (const chunk of openaiStream) {"#;

const CONTENT_ANCHOR: &str = r#"                        const block = ensureTextBlock();
                        block.text += choice.delta.content;
                        stream.push({
                            type: "text_delta",
                            contentIndex: getContentIndex(block),
                            delta: choice.delta.content,
                            partial: output,
                        });"#;
const CONTENT_PATCHED: &str =
    "                        feedThinkingAwareContent(choice.delta.content);";

const FLUSH_ANCHOR: &str =
    "            for (const block of blocks) {\n                finishBlock(block);";
const FLUSH_PATCHED: &str = "            flushThinkingAwareContent();\n            for (const block of blocks) {\n                finishBlock(block);";

fn patch_source(source: &str) -> PatchOutcome {
    if source.contains(PATCH_MARKER) {
        return PatchOutcome::AlreadyPatched;
    }
    if !source.contains(LOOP_ANCHOR)
        || !source.contains(CONTENT_ANCHOR)
        || !source.contains(FLUSH_ANCHOR)
    {
        return PatchOutcome::AnchorMissing;
    }
    let patched = source
        .replacen(LOOP_ANCHOR, LOOP_PATCHED, 1)
        .replacen(CONTENT_ANCHOR, CONTENT_PATCHED, 1)
        .replacen(FLUSH_ANCHOR, FLUSH_PATCHED, 1);
    PatchOutcome::Patched(patched)
}

pub fn apply_at(core_dir: &Path) -> Result<(), String> {
    patch_core_file(core_dir, PI_AI_COMPLETIONS_JS, patch_source)
}
pub fn apply(app_handle: &AppHandle) -> Result<(), String> {
    patch_dsh(app_handle, PI_AI_COMPLETIONS_JS, patch_source)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture() -> String {
        format!("{CONTENT_ANCHOR}\n{LOOP_ANCHOR}\n{FLUSH_ANCHOR}\n            }}\n")
    }

    #[test]
    fn patches_pristine_source_with_tag_scanner() {
        let PatchOutcome::Patched(patched) = patch_source(&fixture()) else {
            panic!("expected patch")
        };
        assert!(patched.contains(PATCH_MARKER));
        assert!(patched.contains(r#"const thinkingTagClose = "</thinking>";"#));
        assert!(
            patched.contains("const block = thinking ? ensureThinkingBlock() : ensureTextBlock();")
        );
        assert!(patched.contains("feedThinkingAwareContent(choice.delta.content);"));
        assert!(patched.contains("            flushThinkingAwareContent();"));
        assert!(!patched.contains(CONTENT_ANCHOR));
        assert_eq!(patched.matches(LOOP_ANCHOR).count(), 1);
    }

    #[test]
    fn patch_is_idempotent() {
        let PatchOutcome::Patched(patched) = patch_source(&fixture()) else {
            panic!("expected patch")
        };
        assert_eq!(patch_source(&patched), PatchOutcome::AlreadyPatched);
    }

    #[test]
    fn missing_loop_anchor_is_skipped() {
        assert_eq!(
            patch_source(&format!("{CONTENT_ANCHOR}\n{FLUSH_ANCHOR}")),
            PatchOutcome::AnchorMissing
        );
    }

    #[test]
    fn missing_content_anchor_is_skipped() {
        assert_eq!(
            patch_source(&format!("{LOOP_ANCHOR}\n{FLUSH_ANCHOR}")),
            PatchOutcome::AnchorMissing
        );
    }

    #[test]
    fn missing_flush_anchor_is_skipped() {
        assert_eq!(
            patch_source(&format!("{LOOP_ANCHOR}\n{CONTENT_ANCHOR}")),
            PatchOutcome::AnchorMissing
        );
    }
}
