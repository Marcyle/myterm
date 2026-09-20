use std::path::PathBuf;

const APP_DIR_NAME: &str = "myterm";
const LEGACY_APP_DIR_NAME: &str = "one-hub";

pub fn config_dir() -> anyhow::Result<PathBuf> {
    let base = if cfg!(target_os = "windows") {
        dirs::config_dir().ok_or_else(|| anyhow::anyhow!("Could not find config directory"))?
    } else {
        dirs::home_dir()
            .ok_or_else(|| anyhow::anyhow!("Could not find home directory"))?
            .join(".config")
    };

    Ok(base.join(APP_DIR_NAME))
}

pub fn legacy_config_dir() -> anyhow::Result<PathBuf> {
    let base = if cfg!(target_os = "windows") {
        dirs::config_dir().ok_or_else(|| anyhow::anyhow!("Could not find config directory"))?
    } else {
        dirs::home_dir()
            .ok_or_else(|| anyhow::anyhow!("Could not find home directory"))?
            .join(".config")
    };

    Ok(base.join(LEGACY_APP_DIR_NAME))
}

pub fn data_dir() -> Option<PathBuf> {
    dirs::data_dir().map(|p| p.join(APP_DIR_NAME))
}

pub fn legacy_data_dir() -> Option<PathBuf> {
    dirs::data_dir().map(|p| p.join(LEGACY_APP_DIR_NAME))
}

pub fn existing_data_file(filename: &str) -> Option<PathBuf> {
    let path = data_dir()?.join(filename);
    if path.exists() {
        return Some(path);
    }

    legacy_data_dir()
        .map(|p| p.join(filename))
        .filter(|p| p.exists())
        .or(Some(path))
}

pub fn migrate_legacy_data_file(filename: &str, overwrite_existing: bool) -> anyhow::Result<()> {
    let Some(data_dir) = data_dir() else {
        return Ok(());
    };
    let Some(legacy_data_dir) = legacy_data_dir() else {
        return Ok(());
    };

    let path = data_dir.join(filename);
    let legacy_path = legacy_data_dir.join(filename);
    if !legacy_path.exists() || path.exists() && !overwrite_existing {
        return Ok(());
    }

    std::fs::create_dir_all(&data_dir)?;
    std::fs::copy(&legacy_path, &path)?;
    tracing::info!(
        legacy_path = %legacy_path.display(),
        new_path = %path.display(),
        "Migrated legacy one-hub data file to myterm"
    );
    Ok(())
}
