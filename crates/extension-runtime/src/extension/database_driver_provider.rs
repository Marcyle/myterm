use std::path::Path;

use anyhow::{Result, anyhow};

use crate::extension::{ExtensionKind, ExtensionProvider, ExtensionSummary};

pub struct DatabaseDriverExtensionProvider;

impl ExtensionProvider for DatabaseDriverExtensionProvider {
    fn kind(&self) -> ExtensionKind {
        ExtensionKind::DatabaseDriver
    }

    fn list_installed(&self, _root: &Path) -> Result<Vec<ExtensionSummary>> {
        // Database support removed
        Ok(Vec::new())
    }

    fn install_from_dir(&self, _dir: &Path) -> Result<ExtensionSummary> {
        Err(anyhow!("database support removed"))
    }

    fn uninstall(&self, _dir: &Path) -> Result<String> {
        Err(anyhow!("database support removed"))
    }
}
