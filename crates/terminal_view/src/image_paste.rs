//! Clipboard image paste support.
//!
//! Terminal paste is text-only, so an image copied from a screenshot tool is
//! first written to a temporary PNG and then pasted as its path. Agent CLIs
//! (Claude Code, Codex, OpenCode, pi) attach or open image paths pasted into
//! their prompt.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize as AtomicCounter, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

use anyhow::{Result, anyhow};
use gpui::{ClipboardEntry, ClipboardItem, ImageFormat};
use image::ExtendedColorType;
use image::ImageEncoder;
use image::codecs::png::PngEncoder;

/// Remote directory used for images pasted into SSH terminals.
pub const REMOTE_DIR: &str = "/tmp/myterm-paste";

/// Maximum image dimension kept after decoding. Screenshot tools rarely
/// exceed this; larger images are downscaled before writing the PNG.
const MAX_IMAGE_DIMENSION: u32 = 8192;

static PASTE_COUNTER: AtomicCounter = AtomicCounter::new(0);

#[derive(Debug)]
pub struct ClipboardImage {
    pub png: Vec<u8>,
    pub file_name: String,
}

/// Extract an image from a GPUI clipboard item.
///
/// Returns `None` when the clipboard holds no image. Decoding/encoding errors
/// are returned so the caller can surface them instead of pasting nothing.
pub fn from_clipboard(item: &ClipboardItem) -> Option<Result<ClipboardImage>> {
    let image = item.entries().iter().find_map(|entry| match entry {
        ClipboardEntry::Image(image) => Some(image),
        _ => None,
    })?;

    Some(encode_clipboard_image(image))
}

fn encode_clipboard_image(image: &gpui::Image) -> Result<ClipboardImage> {
    let format = match image.format {
        ImageFormat::Png => image::ImageFormat::Png,
        ImageFormat::Jpeg => image::ImageFormat::Jpeg,
        ImageFormat::Webp => image::ImageFormat::WebP,
        ImageFormat::Gif => image::ImageFormat::Gif,
        ImageFormat::Bmp => image::ImageFormat::Bmp,
        ImageFormat::Tiff => image::ImageFormat::Tiff,
        ImageFormat::Ico => image::ImageFormat::Ico,
        ImageFormat::Svg => return Err(anyhow!("SVG clipboard images are not supported")),
    };

    let decoded = image::load_from_memory_with_format(&image.bytes, format)
        .map_err(|error| anyhow!("failed to decode clipboard image: {error}"))?
        .to_rgba8();

    encode_rgba(decoded.width(), decoded.height(), decoded.into_raw())
}

fn encode_rgba(width: u32, height: u32, bytes: Vec<u8>) -> Result<ClipboardImage> {
    if width == 0 || height == 0 {
        return Err(anyhow!("clipboard image is empty"));
    }

    let expected = (width as usize)
        .checked_mul(height as usize)
        .and_then(|pixels| pixels.checked_mul(4))
        .ok_or_else(|| anyhow!("image dimensions overflow"))?;
    if bytes.len() != expected {
        return Err(anyhow!(
            "clipboard image has {} bytes, expected {expected}",
            bytes.len()
        ));
    }

    let rgba = image::RgbaImage::from_raw(width, height, bytes)
        .ok_or_else(|| anyhow!("invalid clipboard image buffer"))?;

    let rgba = if width > MAX_IMAGE_DIMENSION || height > MAX_IMAGE_DIMENSION {
        let scale = (MAX_IMAGE_DIMENSION as f32 / width.max(height) as f32).min(1.0);
        let new_width = ((width as f32 * scale).round() as u32).max(1);
        let new_height = ((height as f32 * scale).round() as u32).max(1);
        image::imageops::resize(
            &rgba,
            new_width,
            new_height,
            image::imageops::FilterType::Lanczos3,
        )
    } else {
        rgba
    };

    let mut png = Vec::new();
    PngEncoder::new(&mut png).write_image(
        rgba.as_raw(),
        rgba.width(),
        rgba.height(),
        ExtendedColorType::Rgba8,
    )?;

    Ok(ClipboardImage {
        png,
        file_name: next_file_name(),
    })
}

fn next_file_name() -> String {
    let millis = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_millis())
        .unwrap_or_default();
    let counter = PASTE_COUNTER.fetch_add(1, Ordering::Relaxed);
    format!("paste-{millis}-{}-{counter}.png", std::process::id())
}

/// Write the PNG under the OS temp directory and return its path.
pub fn save_local(image: &ClipboardImage) -> Result<PathBuf> {
    let dir = std::env::temp_dir().join("myterm-paste");
    std::fs::create_dir_all(&dir)?;
    let path = dir.join(&image.file_name);
    std::fs::write(&path, &image.png)?;
    Ok(path)
}

/// Render a path for pasting into a prompt.
///
/// Agent CLIs accept quoted paths; quoting also keeps a path containing spaces
/// usable when it is pasted into a shell prompt.
pub fn format_path(path: &Path) -> String {
    let text = path.to_string_lossy();
    if text.chars().any(char::is_whitespace) {
        format!("\"{text}\"")
    } else {
        text.into_owned()
    }
}

pub fn format_remote_path(path: &str) -> String {
    if path.chars().any(char::is_whitespace) {
        format!("\"{path}\"")
    } else {
        path.to_owned()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn path_without_spaces_is_pasted_verbatim() {
        assert_eq!(
            format_path(Path::new("/tmp/myterm-paste/a.png")),
            "/tmp/myterm-paste/a.png"
        );
    }

    #[test]
    fn path_with_spaces_is_quoted() {
        assert_eq!(
            format_path(Path::new(r"C:\Users\Test User\AppData\Local\Temp\a.png")),
            r#""C:\Users\Test User\AppData\Local\Temp\a.png""#
        );
    }

    #[test]
    fn encodes_rgba_as_png() {
        let encoded = encode_rgba(1, 1, vec![255, 0, 0, 255]).expect("encode");
        assert!(encoded.png.starts_with(b"\x89PNG\r\n\x1a\n"));
        assert!(encoded.file_name.ends_with(".png"));
    }

    #[test]
    fn converts_png_clipboard_entry() {
        let encoded = encode_rgba(2, 1, vec![255, 0, 0, 255, 0, 255, 0, 255]).expect("encode");
        let item = ClipboardItem::new_image(&gpui::Image::from_bytes(
            ImageFormat::Png,
            encoded.png.clone(),
        ));

        let image = from_clipboard(&item)
            .expect("image entry")
            .expect("convert");
        assert!(image.png.starts_with(b"\x89PNG\r\n\x1a\n"));
    }

    #[test]
    fn converts_bmp_clipboard_entry() {
        let rgba =
            image::RgbaImage::from_raw(2, 1, vec![255, 0, 0, 255, 0, 255, 0, 255]).expect("image");
        let mut bmp = std::io::Cursor::new(Vec::new());
        image::DynamicImage::ImageRgba8(rgba)
            .write_to(&mut bmp, image::ImageFormat::Bmp)
            .expect("encode bmp");
        let item =
            ClipboardItem::new_image(&gpui::Image::from_bytes(ImageFormat::Bmp, bmp.into_inner()));

        let image = from_clipboard(&item)
            .expect("image entry")
            .expect("convert");
        assert!(image.png.starts_with(b"\x89PNG\r\n\x1a\n"));
    }

    #[test]
    fn saves_png_under_temp_dir() {
        let image = ClipboardImage {
            png: b"\x89PNG\r\n\x1a\n".to_vec(),
            file_name: format!("test-{}.png", std::process::id()),
        };
        let path = save_local(&image).expect("save");
        assert!(path.exists());
        assert_eq!(std::fs::read(&path).expect("read"), image.png);
        let _ = std::fs::remove_file(path);
    }
}
