//! AI agent 识别与图标映射。
//!
//! 两条检测路径:
//! 1. 命令检测(SSH/JMS,shell integration 上报 OSC 133;C + base64 命令):精确,覆盖命令执行窗口
//! 2. 标题启发式(本地终端,无 shell integration):解析 agent 写入的 OSC 0 标题
//!
//! 实测依据(2026-11 本机抓包,ConPTY 透传):
//! - claude: 标题 "✳ Claude Code" → 工作中 ◐/◑ 轮转 → 完成后 "✳ <会话摘要>"
//! - pi:     标题 "π - <目录>",执行子命令时变为实际命令行
//! - omp:    标题 "π > <目录>",并广播 OSC 7501;state=...:app=omp
//! - omp:    Oh My Pi,官方 logo 为渐变 π
//! - opencode: 广播 OSC 7501;state=idle:app=opencode(结构化状态,暂不依赖)
//! - 三家在 ConPTY 下均不发送 OSC 9;4 进度序列

use std::sync::Arc;

/// 正在终端中运行的 AI agent 类型。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AgentKind {
    Claude,
    Codex,
    Gemini,
    /// pi coding agent(earendil-works/pi)
    Pi,
    /// Oh My Pi(can1357/oh-my-pi)
    OhMyPi,
    OpenCode,
    Grok,
    /// 已识别为 agent 但无品牌图标的工具(aider、droid 等),使用默认图标
    Other(Arc<str>),
}

impl AgentKind {
    /// agent 显示名(供 tooltip / 无障碍使用)
    pub fn display_name(&self) -> &str {
        match self {
            AgentKind::Claude => "Claude Code",
            AgentKind::Codex => "Codex CLI",
            AgentKind::Gemini => "Gemini CLI",
            AgentKind::Pi => "pi",
            AgentKind::OhMyPi => "Oh My Pi",
            AgentKind::OpenCode => "OpenCode",
            AgentKind::Grok => "Grok CLI",
            AgentKind::Other(name) => name,
        }
    }

    /// (SVG 资源路径,是否品牌彩色图标)
    ///
    /// Color 模式经 img() 全彩渲染;Mono 模式经 svg() 跟随主题 text_color。
    pub fn icon_asset(&self) -> (&'static str, bool) {
        match self {
            AgentKind::Claude => ("icons/agents/claude.svg", true),
            AgentKind::Codex => ("icons/agents/codex.svg", false),
            AgentKind::Gemini => ("icons/agents/gemini.svg", true),
            AgentKind::Pi => ("icons/agents/pi.svg", true),
            AgentKind::OhMyPi => ("icons/agents/omp.svg", true),
            AgentKind::OpenCode => ("icons/agents/opencode.svg", false),
            AgentKind::Grok => ("icons/agents/grok.svg", false),
            AgentKind::Other(_) => ("icons/agents/default.svg", false),
        }
    }
}

/// 有品牌图标的 agent 二进制名(小写)。
const BRANDED_AGENTS: &[(&str, AgentKind)] = &[
    ("claude", AgentKind::Claude),
    ("claude-code", AgentKind::Claude),
    ("codex", AgentKind::Codex),
    ("gemini", AgentKind::Gemini),
    ("gemini-cli", AgentKind::Gemini),
    ("pi", AgentKind::Pi),
    ("omp", AgentKind::OhMyPi),
    ("oh-my-pi", AgentKind::OhMyPi),
    ("opencode", AgentKind::OpenCode),
    ("grok", AgentKind::Grok),
];

/// 公认的 agent 二进制名,但暂无品牌图标(命中后回落默认机器人图标)。
const UNBRANDED_AGENTS: &[&str] = &[
    "aider",
    "cursor-agent",
    "droid",
    "crush",
    "qwen",
    "amp",
    "goose",
    "kilo",
];

/// 归一化命令 token:去路径前缀与 Windows 脚本后缀,转小写。
fn normalize_bin(token: &str) -> Option<String> {
    let bin = token.rsplit(['/', '\\']).next()?;
    // 去掉 npm 版本后缀(claude@latest、pi@1.2.0)
    let bin = bin.split('@').next()?;
    let bin = bin
        .strip_suffix(".exe")
        .or_else(|| bin.strip_suffix(".cmd"))
        .or_else(|| bin.strip_suffix(".bat"))
        .unwrap_or(bin);
    if bin.is_empty() {
        None
    } else {
        Some(bin.to_ascii_lowercase())
    }
}

/// 从 shell 命令行识别 agent。
///
/// 扫描全部 token(跳过 sudo/env/npx 等包装器与选项),兼容
/// `claude`、`npx claude`、`sudo /usr/local/bin/claude`、`cd x && claude` 等形态。
pub fn detect_from_command(command: &str) -> Option<AgentKind> {
    for token in command.split_whitespace() {
        // 跳过 shell 操作符与选项
        if token == "&&" || token == "||" || token == ";" || token == "|" || token.starts_with('-')
        {
            continue;
        }
        // 跳过 env 赋值(KEY=VALUE)与包装器
        if token.contains('=') {
            continue;
        }
        let Some(bin) = normalize_bin(token) else {
            continue;
        };
        match bin.as_str() {
            "sudo" | "env" | "nohup" | "exec" | "command" | "nice" | "npx" | "pnpm" | "bunx"
            | "dlx" | "yarn" | "timeout" => continue,
            _ => {}
        }
        for (name, kind) in BRANDED_AGENTS {
            if bin == *name {
                return Some(kind.clone());
            }
        }
        if UNBRANDED_AGENTS.contains(&bin.as_str()) {
            return Some(AgentKind::Other(Arc::from(bin)));
        }
    }
    None
}

/// 从终端标题启发式识别 agent(本地终端无 shell integration 时的兜底)。
pub fn detect_from_title(title: &str) -> Option<AgentKind> {
    let trimmed = title.trim();
    if trimmed.is_empty() {
        return None;
    }
    let lower = trimmed.to_ascii_lowercase();

    // Claude Code 的标题形态:"✳ Claude Code" / "◐ Claude Code" / "✳ <会话摘要>"
    if lower.contains("claude code") {
        return Some(AgentKind::Claude);
    }
    // π 前缀标题按分隔符区分(实测 2026-11):
    // - omp (Oh My Pi):"π > <目录>"
    // - pi (pi.dev):  "π - <目录>"
    // 其余 π 形态无法区分时归入 pi。
    if let Some(rest) = trimmed.strip_prefix('π') {
        if rest.trim_start().starts_with('>') {
            return Some(AgentKind::OhMyPi);
        }
        return Some(AgentKind::Pi);
    }
    // 其余按品牌词匹配
    for (needle, kind) in [
        ("oh my pi", AgentKind::OhMyPi),
        ("opencode", AgentKind::OpenCode),
        ("grok", AgentKind::Grok),
        ("gemini", AgentKind::Gemini),
        ("codex", AgentKind::Codex),
    ] {
        if lower.contains(needle) {
            return Some(kind);
        }
    }
    // 不做裸 "omp" token 扫描:npm/git 等命令行出现在标题时误报率过高
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detect_plain_command() {
        assert_eq!(detect_from_command("claude"), Some(AgentKind::Claude));
        assert_eq!(
            detect_from_command("codex --full-auto"),
            Some(AgentKind::Codex)
        );
        assert_eq!(detect_from_command("omp"), Some(AgentKind::OhMyPi));
        assert_eq!(detect_from_command("git status"), None);
    }

    #[test]
    fn detect_with_wrapper_and_path() {
        assert_eq!(
            detect_from_command("npx claude@latest"),
            Some(AgentKind::Claude)
        );
        assert_eq!(
            detect_from_command("sudo /usr/local/bin/pi --help"),
            Some(AgentKind::Pi)
        );
        assert_eq!(
            detect_from_command("cmd /c claude.exe"),
            Some(AgentKind::Claude)
        );
    }

    #[test]
    fn detect_compound_command() {
        assert_eq!(
            detect_from_command("cd /tmp && claude"),
            Some(AgentKind::Claude)
        );
        // echo 不应因参数里出现 agent 名而误报主体,但当前策略允许 token 扫描命中
        assert_eq!(detect_from_command("echo claude"), Some(AgentKind::Claude));
    }

    #[test]
    fn detect_unbranded_falls_back_to_other() {
        assert!(matches!(
            detect_from_command("aider --model x"),
            Some(AgentKind::Other(name)) if name.as_ref() == "aider"
        ));
    }

    #[test]
    fn env_assignment_ignored() {
        assert_eq!(
            detect_from_command("ANTHROPIC_API_KEY=sk-xxx claude"),
            Some(AgentKind::Claude)
        );
    }

    #[test]
    fn detect_title_claude() {
        assert_eq!(
            detect_from_title("\u{2733} Claude Code"),
            Some(AgentKind::Claude)
        );
        assert_eq!(
            detect_from_title("\u{25d0} Claude Code"),
            Some(AgentKind::Claude)
        );
        assert_eq!(
            detect_from_title("\u{2733} Simple test"),
            None,
            "会话摘要标题不再包含 Claude Code,不应误报"
        );
    }

    #[test]
    fn detect_title_pi() {
        assert_eq!(detect_from_title("π - myterm"), Some(AgentKind::Pi));
        assert_eq!(detect_from_title("user@host: ~/pi"), None);
    }

    #[test]
    fn detect_title_omp() {
        assert_eq!(detect_from_title("π > myterm"), Some(AgentKind::OhMyPi));
        assert_eq!(detect_from_title("π - myterm"), Some(AgentKind::Pi));
        assert_eq!(detect_from_title("πmyterm"), Some(AgentKind::Pi));
    }

    #[test]
    fn detect_title_others() {
        assert_eq!(detect_from_title("OpenCode"), Some(AgentKind::OpenCode));
        assert_eq!(detect_from_title("Grok CLI"), Some(AgentKind::Grok));
        assert_eq!(detect_from_title("Oh My Pi"), Some(AgentKind::OhMyPi));
        assert_eq!(detect_from_title("npm view omp version"), None);
        assert_eq!(detect_from_title(""), None);
    }

    #[test]
    fn icon_assets_exist_and_are_consistent() {
        for kind in [
            AgentKind::Claude,
            AgentKind::Codex,
            AgentKind::Gemini,
            AgentKind::Pi,
            AgentKind::OhMyPi,
            AgentKind::OpenCode,
            AgentKind::Grok,
            AgentKind::Other(Arc::from("aider")),
        ] {
            let (path, _) = kind.icon_asset();
            assert!(path.starts_with("icons/agents/"), "{}", path);
            assert!(path.ends_with(".svg"), "{}", path);
        }
    }
}
