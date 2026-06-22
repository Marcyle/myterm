mod composite_provider;
mod database_driver_provider;
mod kind;
mod language_provider;
pub mod manifest;
mod provider;
mod summary;

pub use composite_provider::CompositeExtensionProvider;
pub use database_driver_provider::DatabaseDriverExtensionProvider;
pub use kind::ExtensionKind;
pub use language_provider::LanguageExtensionProvider;
pub use provider::{ExtensionProvider, ExtensionRegistry, init_global};
pub use summary::ExtensionSummary;

use std::{path::PathBuf, sync::Arc};

use gpui::{App, BorrowAppContext};
use gpui_component::highlighter::{LanguageRegistry, LoadReport, load_extensions_dir};

pub fn init(cx: &mut App) {
    let Some(root) = extensions_root() else {
        tracing::warn!("无法解析扩展根目录,跳过 ExtensionRegistry 初始化");
        return;
    };
    let registry = builtin_registry(root.clone());
    init_global(registry);
    load_language_extensions(&root);
    crate::refresh_global_runtime_catalog(cx);
    crate::extension_action_handler::register_db_tree_extension_action_handler(cx);
}

pub fn builtin_registry(extensions_root: PathBuf) -> ExtensionRegistry {
    let mut registry = ExtensionRegistry::new(extensions_root);
    registry.register_provider(Arc::new(LanguageExtensionProvider));
    registry.register_provider(Arc::new(DatabaseDriverExtensionProvider));
    registry.register_provider(Arc::new(CompositeExtensionProvider));
    registry
}

pub fn extensions_root() -> Option<PathBuf> {
    let base = one_core::storage::manager::get_config_dir().ok()?;
    Some(base.join("extensions"))
}

pub fn load_language_extensions_from_root(root: &std::path::Path) -> anyhow::Result<LoadReport> {
    load_extensions_dir(
        &root.join(ExtensionKind::Language.dir_name()),
        LanguageRegistry::singleton(),
    )
}

fn load_language_extensions(root: &std::path::Path) {
    match load_language_extensions_from_root(root) {
        Ok(report) => {
            if !report.loaded.is_empty() {
                tracing::info!(
                    "已加载 {} 个语言扩展: {:?}",
                    report.loaded.len(),
                    report.loaded
                );
            }
            if !report.failed.is_empty() {
                tracing::warn!(
                    "有 {} 个语言扩展加载失败: {:?}",
                    report.failed.len(),
                    report.failed
                );
            }
        }
        Err(err) => {
            tracing::warn!("加载语言扩展失败: {err:?}");
        }
    }
}

pub fn refresh_runtime_contributions(_cx: &mut impl BorrowAppContext) {
    // Database tree extension menu registry removed
}

#[cfg(test)]
mod composite_provider_tests;
#[cfg(test)]
mod provider_tests;
