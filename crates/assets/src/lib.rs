use anyhow::anyhow;
use gpui::{AssetSource, Result, SharedString};
use rust_embed::RustEmbed;
use std::borrow::Cow;

/// Embed application assets for GPUI Component.
///
/// This assets provides icons svg files for [IconName](https://docs.rs/gpui-component/latest/gpui_component/enum.IconName.html).
///
/// ```
/// use gpui::*;
/// use gpui_component_assets::Assets;
///
/// let app = Application::new().with_assets(Assets);
/// ```
#[derive(RustEmbed)]
#[folder = "assets"]
#[include = "icons/**/*.svg"]
#[include = "fonts/**/*.ttf"]
pub struct Assets;

impl AssetSource for Assets {
    fn load(&self, path: &str) -> Result<Option<Cow<'static, [u8]>>> {
        if path.is_empty() {
            return Ok(None);
        }

        Self::get(path)
            .map(|f| Some(f.data))
            .ok_or_else(|| anyhow!("could not find asset at path \"{path}\""))
    }

    fn list(&self, path: &str) -> Result<Vec<SharedString>> {
        Ok(Self::iter()
            .filter_map(|p| p.starts_with(path).then(|| p.into()))
            .collect())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 回归测试:agent 图标必须能从 AssetSource 加载。
    /// 背景:rust-embed 不声明 rerun-if-changed,新增资源文件曾导致
    /// release 构建嵌入过期快照、图标全部落空(build.rs 已修复)。
    #[test]
    fn agent_icons_are_loadable() {
        use gpui::AssetSource as _;
        let agents = [
            "icons/agents/claude.svg",
            "icons/agents/codex.svg",
            "icons/agents/gemini.svg",
            "icons/agents/grok.svg",
            "icons/agents/omp.svg",
            "icons/agents/opencode.svg",
            "icons/agents/pi.svg",
            "icons/agents/default.svg",
        ];
        for name in agents {
            let data = Assets
                .load(name)
                .unwrap_or_else(|e| panic!("{name}: load error {e}"))
                .unwrap_or_else(|| panic!("{name}: not embedded"));
            assert!(!data.is_empty(), "{name}: empty payload");
        }
        let listed = Assets.list("icons/agents").expect("list icons/agents");
        assert!(listed.len() >= 8, "expected >= 8 agent icons, got {:?}", listed);
    }
}
