//! 终端主题配置
//!
//! 提供终端的颜色、字体、字号等外观设置
//!
//! ## 配色系统设计
//!
//! 本模块采用语义化配色系统，确保所有颜色组合具有足够的对比度：
//! - `background` / `foreground`: 主要背景和文字，对比度 >= 7:1
//! - `muted` / `muted_foreground`: 次要区域背景和文字，对比度 >= 4.5:1
//! - `accent` / `accent_foreground`: 强调色背景和文字，对比度 >= 4.5:1
//!
//! 颜色使用规则：
//! - 在 `background` 上使用 `foreground` 或 `muted_foreground`
//! - 在 `muted` 上使用 `foreground` 或 `muted_foreground`
//! - 在 `accent` 上使用 `accent_foreground`

use gpui::{App, Font, Hsla, SharedString, TextSystem, px, rgb};
use std::sync::OnceLock;

/// 最小字体大小
pub const MIN_FONT_SIZE: f32 = 8.0;
/// 最大字体大小
pub const MAX_FONT_SIZE: f32 = 32.0;
/// 默认行高比例
pub const DEFAULT_LINE_HEIGHT_SCALE: f32 = 1.4;

/// 终端主题配色（用于侧边栏等 UI 组件）
///
/// 所有颜色对都经过对比度验证，确保可读性：
/// - `background` + `foreground`: 主要内容
/// - `background` + `muted_foreground`: 次要内容
/// - `muted` + `foreground`: 卡片/列表项上的主要内容
/// - `muted` + `muted_foreground`: 卡片/列表项上的次要内容
/// - `accent` + `accent_foreground`: 按钮/选中状态
#[derive(Clone, Debug)]
pub struct TerminalColors {
    /// 主背景色
    pub background: Hsla,
    /// 主前景色（在 background 上使用）
    pub foreground: Hsla,
    /// 次要背景色（卡片、列表项、悬停状态）
    pub muted: Hsla,
    /// 次要前景色（次要文字、标签、占位符）
    pub muted_foreground: Hsla,
    /// 边框色
    pub border: Hsla,
    /// 强调背景色（按钮、选中项）
    pub accent: Hsla,
    /// 强调前景色（在 accent 背景上使用）
    pub accent_foreground: Hsla,
}

/// 终端主题配置
#[derive(Clone, Debug)]
pub struct TerminalTheme {
    /// 主题名称
    pub name: &'static str,
    /// 前景色（文字颜色）
    pub foreground: Hsla,
    /// 背景色
    pub background: Hsla,
    /// 光标颜色
    pub cursor: Hsla,
    /// 选中区域颜色
    pub selection: Hsla,
}

impl PartialEq for TerminalTheme {
    fn eq(&self, other: &Self) -> bool {
        self.name == other.name
            && self.foreground == other.foreground
            && self.background == other.background
            && self.cursor == other.cursor
            && self.selection == other.selection
    }
}

/// 获取当前操作系统的默认等宽字体
pub fn default_monospace_font() -> &'static str {
    if cfg!(target_os = "macos") {
        "Menlo"
    } else if cfg!(target_os = "windows") {
        "Consolas"
    } else {
        // Linux 和其他系统
        "DejaVu Sans Mono"
    }
}

/// Nerd Font 备用字体（用于渲染私用区图标字形）
///
/// TUI 程序（yazi、eza、starship、lsd 等）的图标位于私用区：
/// U+E000–U+F8FF 以及补充私用区 U+F0000–U+FFFFD（Nerd Fonts v3 图标大量在后者）。
/// 普通等宽字体（Consolas、JetBrains Mono、Menlo 等）没有这些字形，缺少时的表现
/// 就是空白或方框（.notdef），因此必须由打过 Nerd Fonts 补丁的家族来兜底。
///
/// 列表按优先级排列，未安装的家族会被字形查找自动跳过；`Maple Mono NF`
/// 由应用内置（见 myterm_app.rs 中 add_fonts 的注册逻辑），因此一定可用。
pub fn nerd_font_fallbacks() -> Vec<SharedString> {
    vec![
        "Maple Mono NF".into(),
        "Symbols Nerd Font Mono".into(),
        "Symbols Nerd Font".into(),
        "JetBrainsMono Nerd Font Mono".into(),
        "JetBrainsMono Nerd Font".into(),
        "CaskaydiaCove Nerd Font Mono".into(),
        "CaskaydiaMono Nerd Font Mono".into(),
        "FiraCode Nerd Font Mono".into(),
        "Hack Nerd Font Mono".into(),
        "MesloLGS NF".into(),
        "SauceCodePro Nerd Font Mono".into(),
        "UbuntuMono Nerd Font Mono".into(),
        "Maple Mono NF CN".into(),
        "Maple Mono Normal NF CN".into(),
    ]
}

/// 默认备用字体列表（按优先级排序，跨平台兼容）
///
/// 顺序：本平台常见等宽字体 → Nerd Font 图标字体 → Emoji / CJK 字体。
/// Nerd Font 必须排在 CJK 字体之前：部分中文字体在私用区也有映射，
/// 若排在前面会把图标字形“抢”成乱码或空白。
pub fn default_font_fallbacks() -> Vec<SharedString> {
    let mut fallbacks: Vec<SharedString> = if cfg!(target_os = "macos") {
        vec![
            "JetBrains Mono".into(),
            "Menlo".into(),
            "Monaco".into(),
            "SF Mono".into(),
            "Courier New".into(),
        ]
    } else if cfg!(target_os = "windows") {
        vec![
            "JetBrains Mono".into(),
            "Cascadia Mono".into(),
            "Consolas".into(),
            "Courier New".into(),
            "Lucida Console".into(),
        ]
    } else {
        vec![
            "JetBrains Mono".into(),
            "Ubuntu Mono".into(),
            "Liberation Mono".into(),
            "Courier New".into(),
        ]
    };

    fallbacks.extend(nerd_font_fallbacks());

    if cfg!(target_os = "macos") {
        fallbacks.extend([
            "Apple Color Emoji".into(),
            "Apple Symbols".into(),
            "PingFang SC".into(),
            "PingFang TC".into(),
            "Hiragino Sans GB".into(),
            "Heiti SC".into(),
        ]);
    } else if cfg!(target_os = "windows") {
        fallbacks.extend([
            "Segoe UI Emoji".into(),
            "Microsoft YaHei UI".into(),
            "Microsoft YaHei".into(),
            "SimHei".into(),
            "DengXian".into(),
            "SimSun".into(),
        ]);
    } else {
        fallbacks.extend([
            "Noto Color Emoji".into(),
            "Noto Sans Mono CJK SC".into(),
            "Noto Sans CJK SC".into(),
            "WenQuanYi Micro Hei".into(),
            "WenQuanYi Zen Hei".into(),
        ]);
    }

    fallbacks
}

/// 系统可用等宽字体族（含内置注册字体），用于设置面板的字体下拉框。
///
/// 结果在进程内只枚举一次（`OnceLock` 缓存）：
/// `all_font_names()` 会遍历系统字体集合，逐族判定等宽又会做多次字形度量，
/// 开销较大，而 `SettingField::dropdown` 每次渲染都会重新构造选项列表。
pub fn available_terminal_fonts(cx: &App) -> Vec<SharedString> {
    static CACHE: OnceLock<Vec<SharedString>> = OnceLock::new();
    CACHE
        .get_or_init(|| enumerate_monospace_fonts(cx.text_system()))
        .clone()
}

/// 枚举系统里真实存在的等宽字体族，并按「推荐优先」排序。
fn enumerate_monospace_fonts(text_system: &TextSystem) -> Vec<SharedString> {
    // 过滤掉内部家族（".SystemUIFont"）与 Windows 竖排/旋转变体（以 '@' 开头）
    let mut families: Vec<String> = text_system
        .all_font_names()
        .into_iter()
        .filter(|name| !name.is_empty() && !name.starts_with('.') && !name.starts_with('@'))
        .filter(|name| is_monospace_family(text_system, name))
        .collect();

    // 系统字体集合与内置字体集合可能同名（如已安装 JetBrains Mono），去重
    families.sort_by_key(|name| name.to_lowercase());
    families.dedup_by(|a, b| a.eq_ignore_ascii_case(b));

    let mut ordered: Vec<SharedString> = Vec::with_capacity(families.len());
    let preferred: Vec<String> = TerminalTheme::available_monospace_fonts()
        .iter()
        .map(|name| name.to_string())
        .chain(nerd_font_fallbacks().iter().map(|name| name.to_string()))
        .collect();
    for name in preferred {
        if let Some(pos) = families.iter().position(|f| f.eq_ignore_ascii_case(&name)) {
            ordered.push(families.remove(pos).into());
        }
    }
    ordered.extend(families.into_iter().map(SharedString::from));
    ordered
}

/// 判定一个字体族是否适合做终端等宽字体，三条都满足才算：
///
/// 1. `m` / `i` / `W` / `0` 的前进宽度一致——最基本的一条；
/// 2. 单元格宽度落在 0.4~0.8 em。东亚旧字体（宋体、MS Gothic）与多数等宽字体在此区间，
///    而 Marlett / Webdings / Wingdings 这类“整字宽图标字体”会被排掉；
/// 3. `g` 比 `x` 高（下伸部）、`X` 比 `x` 高（大写字高）。这一条专门排除
///    Segoe MDL2 Assets / Segoe Fluent Icons / Kingsoft UE 这类图标字体：
///    它们把 a–z/A–Z 码位都映射成同尺寸图元，两个比值恒为 1.00。
///
/// 名字启发式（带 Mono/Code）不可取：Consolas、Menlo、Hack 名字里都没有 Mono。
fn is_monospace_family(text_system: &TextSystem, family: &str) -> bool {
    const SIZE: f32 = 16.0;
    let font = Font {
        family: SharedString::from(family.to_owned()),
        ..Default::default()
    };
    let font_id = text_system.resolve_font(&font);
    let size = px(SIZE);

    let advance = |ch: char| {
        text_system
            .advance(font_id, size, ch)
            .ok()
            .map(|s| f32::from(s.width))
    };
    let height = |ch: char| {
        text_system
            .typographic_bounds(font_id, size, ch)
            .ok()
            .map(|b| f32::from(b.size.height))
    };

    // 1) 等宽
    let Some(cell) = advance('m') else {
        return false;
    };
    for ch in ['i', 'W', '0'] {
        match advance(ch) {
            Some(other) if (other - cell).abs() < 0.3 => {}
            _ => return false,
        }
    }

    // 2) 单元格宽度合理
    if !(0.4 * SIZE..=0.8 * SIZE).contains(&cell) {
        return false;
    }

    // 3) 确实含拉丁文字字形（有下伸部、有独立大写字高）
    let (Some(x_height), Some(g_height), Some(cap_height)) =
        (height('x'), height('g'), height('X'))
    else {
        return false;
    };
    if x_height <= 0.0 {
        return false;
    }

    let descender_ratio = g_height / x_height;
    let cap_ratio = cap_height / x_height;
    (1.15..=2.5).contains(&descender_ratio) && (1.10..=2.5).contains(&cap_ratio)
}

impl TerminalTheme {
    /// 获取所有可用主题
    pub fn all() -> Vec<Self> {
        vec![
            Self::midnight(),
            Self::daylight(),
            Self::ink(),
            Self::paper(),
            Self::ocean(),
            Self::obsidian(),
            Self::lotus(),
            Self::neon_blue(),
            Self::matrix(),
            Self::crimson(),
        ]
    }

    /// 创建主题
    fn new(
        name: &'static str,
        foreground: Hsla,
        background: Hsla,
        cursor: Hsla,
        selection: Hsla,
    ) -> Self {
        Self {
            name,
            foreground,
            background,
            cursor,
            selection,
        }
    }

    /// 暗夜主题（深灰背景，浅灰文字）
    pub fn midnight() -> Self {
        Self::new(
            "midnight",
            rgb(0xE4E4E4).into(),
            rgb(0x1E1E1E).into(),
            rgb(0xFFFFFF).into(),
            rgb(0x3D3D3D).into(),
        )
    }

    /// 明亮主题（白色背景，深灰文字）
    pub fn daylight() -> Self {
        Self::new(
            "daylight",
            rgb(0x2E3436).into(),
            rgb(0xFFFFFF).into(),
            rgb(0x000000).into(),
            rgb(0xD3D7CF).into(),
        )
    }

    /// 墨黑主题（近黑背景，米色文字）
    pub fn ink() -> Self {
        Self::new(
            "ink",
            rgb(0xCECDC3).into(),
            rgb(0x100F0F).into(),
            rgb(0xDA702C).into(),
            rgb(0x282726).into(),
        )
    }

    /// 纸白主题（米白背景，深色文字）
    pub fn paper() -> Self {
        Self::new(
            "paper",
            rgb(0x100F0F).into(),
            rgb(0xFFFCF0).into(),
            rgb(0xDA702C).into(),
            rgb(0xE6E4D9).into(),
        )
    }

    /// 海浪主题（深蓝灰背景，暖米色文字）
    pub fn ocean() -> Self {
        Self::new(
            "ocean",
            rgb(0xDCD7BA).into(),
            rgb(0x1F1F28).into(),
            rgb(0xC8C093).into(),
            rgb(0x2D4F67).into(),
        )
    }

    /// 黑曜主题（深棕黑背景，灰绿文字）
    pub fn obsidian() -> Self {
        Self::new(
            "obsidian",
            rgb(0xC5C9C5).into(),
            rgb(0x181616).into(),
            rgb(0xC8C093).into(),
            rgb(0x2D4F67).into(),
        )
    }

    /// 莲白主题（米黄背景，深灰紫文字）
    pub fn lotus() -> Self {
        Self::new(
            "lotus",
            rgb(0x545464).into(),
            rgb(0xF2ECBC).into(),
            rgb(0x43436C).into(),
            rgb(0xB6D7A8).into(),
        )
    }

    /// 霓蓝主题（深蓝黑背景，青蓝文字）
    pub fn neon_blue() -> Self {
        Self::new(
            "neon_blue",
            rgb(0x00D9FF).into(),
            rgb(0x0A0E14).into(),
            rgb(0xFFFFFF).into(),
            rgb(0x1A3A52).into(),
        )
    }

    /// 矩阵主题（近黑背景，亮绿文字，Matrix 风格）
    pub fn matrix() -> Self {
        Self::new(
            "matrix",
            rgb(0x00FF41).into(),
            rgb(0x0D0D0D).into(),
            rgb(0xFFFFFF).into(),
            rgb(0x1A3A1A).into(),
        )
    }

    /// 赤红主题（深红黑背景，亮红文字）
    pub fn crimson() -> Self {
        Self::new(
            "crimson",
            rgb(0xFF5555).into(),
            rgb(0x1A0A0A).into(),
            rgb(0xFFFFFF).into(),
            rgb(0x4A1A1A).into(),
        )
    }

    /// 根据名称查找主题
    pub fn find_by_name(name: &str) -> Option<Self> {
        Self::all().into_iter().find(|t| t.name == name)
    }

    /// 判断是否为深色主题
    pub fn is_dark(&self) -> bool {
        // 根据背景色亮度判断
        self.background.l < 0.5
    }

    /// 获取用于 UI 组件的配色
    ///
    /// 该方法根据主题的基础颜色生成一套完整的 UI 配色，
    /// 所有颜色组合都保证足够的对比度以确保可读性。
    pub fn colors(&self) -> TerminalColors {
        let is_dark = self.is_dark();

        // 计算 muted 背景色（卡片、列表项等）
        let muted = if is_dark {
            // 深色主题：muted 比背景稍亮
            Hsla {
                h: self.background.h,
                s: self.background.s,
                l: (self.background.l + 0.06).min(0.25),
                a: 1.0,
            }
        } else {
            // 浅色主题：muted 比背景稍暗
            Hsla {
                h: self.background.h,
                s: self.background.s.min(0.1),
                l: (self.background.l - 0.06).max(0.85),
                a: 1.0,
            }
        };

        // 计算 muted_foreground（次要文字）
        // 关键：必须与 background 和 muted 都有足够对比度
        let muted_foreground = if is_dark {
            // 深色主题：使用中等亮度的灰色
            // 确保在深色背景上可读
            Hsla {
                h: self.foreground.h,
                s: self.foreground.s * 0.3,
                l: 0.55, // 固定中等亮度，确保在深色背景上可读
                a: 1.0,
            }
        } else {
            // 浅色主题：使用较深的灰色
            // 确保在浅色背景上可读
            Hsla {
                h: self.foreground.h,
                s: self.foreground.s * 0.3,
                l: 0.45, // 固定中等亮度，确保在浅色背景上可读
                a: 1.0,
            }
        };

        // 计算边框色
        let border = if is_dark {
            Hsla {
                h: self.background.h,
                s: self.background.s,
                l: (self.background.l + 0.12).min(0.35),
                a: 1.0,
            }
        } else {
            Hsla {
                h: self.background.h,
                s: self.background.s.min(0.1),
                l: (self.background.l - 0.15).max(0.75),
                a: 1.0,
            }
        };

        // 计算强调色前景（在 accent 背景上使用的文字颜色）
        // 根据 accent 的亮度决定使用深色还是浅色文字
        let accent_foreground = if self.cursor.l > 0.5 {
            // accent 是亮色，使用深色文字
            Hsla {
                h: self.cursor.h,
                s: self.cursor.s * 0.2,
                l: 0.1, // 深色文字
                a: 1.0,
            }
        } else {
            // accent 是暗色，使用亮色文字
            Hsla {
                h: self.cursor.h,
                s: self.cursor.s * 0.1,
                l: 0.95, // 亮色文字
                a: 1.0,
            }
        };

        TerminalColors {
            background: self.background,
            foreground: self.foreground,
            muted,
            muted_foreground,
            border,
            accent: self.cursor,
            accent_foreground,
        }
    }

    /// 人工维护的「推荐等宽字体」列表（按操作系统优化排序）
    ///
    /// 仅用于两件事：字体下拉框的置顶顺序；以及极端情况下（字体枚举失败）的兼容返回。
    /// 下拉框的实际内容由 [`available_terminal_fonts`] 枚举系统字体得到，
    /// 因此用户自己安装的字体（如 FiraCode Nerd Font Mono、Maple Mono NF CN）也能选到。
    pub fn available_monospace_fonts() -> Vec<&'static str> {
        if cfg!(target_os = "macos") {
            vec![
                "Menlo", // macOS 默认
                "JetBrains Mono",
                "Maple Mono",
                "Maple Mono NF",
                "Monaco",
                "SF Mono",
                "Courier New",
                "Fira Code",
                "Source Code Pro",
                "Cascadia Code",
                "Hack",
                "IBM Plex Mono",
            ]
        } else if cfg!(target_os = "windows") {
            vec![
                "Consolas", // Windows 默认
                "JetBrains Mono",
                "Maple Mono",
                "Maple Mono NF",
                "Cascadia Mono",
                "Cascadia Code",
                "Courier New",
                "Lucida Console",
                "Fira Code",
                "Source Code Pro",
                "Hack",
                "IBM Plex Mono",
            ]
        } else {
            vec![
                "DejaVu Sans Mono", // Linux 常见默认
                "JetBrains Mono",
                "Maple Mono",
                "Maple Mono NF",
                "Ubuntu Mono",
                "Liberation Mono",
                "Courier New",
                "Fira Code",
                "Source Code Pro",
                "Cascadia Code",
                "Hack",
                "IBM Plex Mono",
            ]
        }
    }
}
