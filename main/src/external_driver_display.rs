use gpui_component::{Icon, IconName, Size, Sizable};
use one_core::storage::DbConnectionConfig;
use std::path::Path;

pub(crate) fn external_driver_icon_for_config(
    _config: &DbConnectionConfig,
    size: impl Into<Size>,
) -> Option<Icon> {
    // Database support removed - return default icon
    Some(IconName::Database.color().with_size(size))
}

#[allow(dead_code)]
pub(crate) fn external_driver_icon_from_path(_path: &str, size: impl Into<Size>) -> Icon {
    // Database support removed - return default icon
    IconName::Database.color().with_size(size)
}

#[allow(dead_code)]
pub(crate) fn external_driver_icon_from_file_path(_path: &Path, size: impl Into<Size>) -> Icon {
    // Database support removed - return default icon
    IconName::Database.color().with_size(size)
}
