//! Provider brand icons, vendored from CodexBar and embedded in the binary.
//!
//! The SVGs under `data/icons/providers/` come from
//! `Sources/CodexBar/Resources/ProviderIcon-<slug>.svg` in
//! <https://github.com/steipete/CodexBar> (MIT, Copyright (c) 2026 Peter
//! Steinberger - see `THIRD_PARTY_LICENSES.md`). Each is a single monochrome
//! `<path fill="white">` in a `100x100` viewBox, meant to be tinted by the
//! consuming app, which is why they are rendered with the theme's icon colour
//! rather than a per-provider accent.
//!
//! Embedding avoids an install step and a runtime file lookup for ~270KB of
//! assets that never change. The table is generated from the directory listing;
//! the slug is the CodexBar provider id, which is also the id the `codexbar`
//! CLI reports in `ProviderPayload::provider`.

/// Every vendored icon, keyed by provider id. Sorted, so lookup can bisect.
const PROVIDER_ICONS: &[(&str, &[u8])] = &[
    ("abacus", include_bytes!("../data/icons/providers/abacus.svg")),
    ("aiand", include_bytes!("../data/icons/providers/aiand.svg")),
    ("alibaba", include_bytes!("../data/icons/providers/alibaba.svg")),
    ("amp", include_bytes!("../data/icons/providers/amp.svg")),
    ("antigravity", include_bytes!("../data/icons/providers/antigravity.svg")),
    ("augment", include_bytes!("../data/icons/providers/augment.svg")),
    ("bedrock", include_bytes!("../data/icons/providers/bedrock.svg")),
    ("chutes", include_bytes!("../data/icons/providers/chutes.svg")),
    ("claude", include_bytes!("../data/icons/providers/claude.svg")),
    ("clawrouter", include_bytes!("../data/icons/providers/clawrouter.svg")),
    ("clinepass", include_bytes!("../data/icons/providers/clinepass.svg")),
    ("codebuff", include_bytes!("../data/icons/providers/codebuff.svg")),
    ("codex", include_bytes!("../data/icons/providers/codex.svg")),
    ("commandcode", include_bytes!("../data/icons/providers/commandcode.svg")),
    ("copilot", include_bytes!("../data/icons/providers/copilot.svg")),
    ("crof", include_bytes!("../data/icons/providers/crof.svg")),
    ("cursor", include_bytes!("../data/icons/providers/cursor.svg")),
    ("deepgram", include_bytes!("../data/icons/providers/deepgram.svg")),
    ("deepinfra", include_bytes!("../data/icons/providers/deepinfra.svg")),
    ("deepseek", include_bytes!("../data/icons/providers/deepseek.svg")),
    ("devin", include_bytes!("../data/icons/providers/devin.svg")),
    ("doubao", include_bytes!("../data/icons/providers/doubao.svg")),
    ("elevenlabs", include_bytes!("../data/icons/providers/elevenlabs.svg")),
    ("factory", include_bytes!("../data/icons/providers/factory.svg")),
    ("gemini", include_bytes!("../data/icons/providers/gemini.svg")),
    ("grok", include_bytes!("../data/icons/providers/grok.svg")),
    ("groq", include_bytes!("../data/icons/providers/groq.svg")),
    ("jetbrains", include_bytes!("../data/icons/providers/jetbrains.svg")),
    ("kilo", include_bytes!("../data/icons/providers/kilo.svg")),
    ("kimi", include_bytes!("../data/icons/providers/kimi.svg")),
    ("kiro", include_bytes!("../data/icons/providers/kiro.svg")),
    ("litellm", include_bytes!("../data/icons/providers/litellm.svg")),
    ("llmproxy", include_bytes!("../data/icons/providers/llmproxy.svg")),
    ("longcat", include_bytes!("../data/icons/providers/longcat.svg")),
    ("manus", include_bytes!("../data/icons/providers/manus.svg")),
    ("mimo", include_bytes!("../data/icons/providers/mimo.svg")),
    ("minimax", include_bytes!("../data/icons/providers/minimax.svg")),
    ("mistral", include_bytes!("../data/icons/providers/mistral.svg")),
    ("neuralwatt", include_bytes!("../data/icons/providers/neuralwatt.svg")),
    ("notion", include_bytes!("../data/icons/providers/notion.svg")),
    ("ollama", include_bytes!("../data/icons/providers/ollama.svg")),
    ("opencode", include_bytes!("../data/icons/providers/opencode.svg")),
    ("opencodego", include_bytes!("../data/icons/providers/opencodego.svg")),
    ("openrouter", include_bytes!("../data/icons/providers/openrouter.svg")),
    ("perplexity", include_bytes!("../data/icons/providers/perplexity.svg")),
    ("poe", include_bytes!("../data/icons/providers/poe.svg")),
    ("qoder", include_bytes!("../data/icons/providers/qoder.svg")),
    ("qwencloud", include_bytes!("../data/icons/providers/qwencloud.svg")),
    ("sakana", include_bytes!("../data/icons/providers/sakana.svg")),
    ("stepfun", include_bytes!("../data/icons/providers/stepfun.svg")),
    ("sub2api", include_bytes!("../data/icons/providers/sub2api.svg")),
    ("synthetic", include_bytes!("../data/icons/providers/synthetic.svg")),
    ("t3chat", include_bytes!("../data/icons/providers/t3chat.svg")),
    ("venice", include_bytes!("../data/icons/providers/venice.svg")),
    ("vertexai", include_bytes!("../data/icons/providers/vertexai.svg")),
    ("warp", include_bytes!("../data/icons/providers/warp.svg")),
    ("wayfinder", include_bytes!("../data/icons/providers/wayfinder.svg")),
    ("windsurf", include_bytes!("../data/icons/providers/windsurf.svg")),
    ("xai", include_bytes!("../data/icons/providers/xai.svg")),
    ("zai", include_bytes!("../data/icons/providers/zai.svg")),
    ("zed", include_bytes!("../data/icons/providers/zed.svg")),
    ("zenmux", include_bytes!("../data/icons/providers/zenmux.svg")),
    ("zoommate", include_bytes!("../data/icons/providers/zoommate.svg")),
];

/// The icon for a CLI provider id, or `None` when nothing is vendored for it.
///
/// CodexBar names its icon files after the same provider ids the CLI reports,
/// so this is a direct lookup; callers fall back to a text-only label rather
/// than drawing a placeholder.
pub fn provider_icon(provider: &str) -> Option<&'static [u8]> {
    PROVIDER_ICONS
        .binary_search_by_key(&provider, |(slug, _)| *slug)
        .ok()
        .map(|index| PROVIDER_ICONS[index].1)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn table_is_sorted_for_binary_search() {
        assert!(PROVIDER_ICONS.windows(2).all(|w| w[0].0 < w[1].0));
    }

    #[test]
    fn finds_the_providers_the_cli_reports() {
        // Every id seen in this repo's real-world CLI fixtures.
        for provider in ["codex", "claude", "antigravity"] {
            assert!(provider_icon(provider).is_some(), "missing icon for {provider}");
        }
        assert!(provider_icon("gemini").is_some());
        assert!(provider_icon("copilot").is_some());
    }

    #[test]
    fn unknown_provider_has_no_icon() {
        assert!(provider_icon("definitely-not-a-provider").is_none());
    }

    #[test]
    fn every_icon_is_a_non_empty_svg() {
        for (slug, bytes) in PROVIDER_ICONS {
            assert!(!bytes.is_empty(), "{slug} is empty");
            // Some carry an XML declaration or doctype ahead of the root tag.
            assert!(
                bytes.windows(4).any(|w| w == b"<svg"),
                "{slug} is not an SVG"
            );
        }
    }
}
