<div align="center">
  <p>
    <img src="logo.svg" alt="MyTerm" width="120" />
  </p>

  <h1>MyTerm</h1>

  <p><strong>Native desktop workspace for SSH, SFTP, local terminals, JumpServer bastion access, and port forwarding.</strong></p>

  <p>
    Built with <a href="https://gpui.rs">GPUI</a> · Rust native desktop · GPU-accelerated rendering
  </p>

  <p>
    <a href="https://github.com/feigeCode/myterm/releases"><img src="https://img.shields.io/github/downloads/feigeCode/myterm/total?style=for-the-badge&color=blue" alt="Downloads" /></a>
    <a href="https://github.com/feigeCode/myterm/actions/workflows/ci.yml"><img src="https://img.shields.io/github/actions/workflow/status/feigeCode/myterm/ci.yml?branch=main&style=for-the-badge" alt="CI" /></a>
    <a href="LICENSE-APACHE"><img src="https://img.shields.io/badge/license-Apache--2.0-blue?style=for-the-badge" alt="License" /></a>
    <a href="https://qm.qq.com/cgi-bin/qm/qr?k=&group_code=860670605"><img src="https://img.shields.io/badge/QQ%20Group-860670605-EB1923?style=for-the-badge&logo=tencentqq&logoColor=white" alt="QQ Group 860670605" /></a>
  </p>

  <p>
    <img src="https://img.shields.io/badge/SSH-111827?logo=gnubash&logoColor=white" alt="SSH" />
    <img src="https://img.shields.io/badge/SFTP-2563EB?logo=filezilla&logoColor=white" alt="SFTP" />
    <img src="https://img.shields.io/badge/Terminal-0F172A?logo=gnometerminal&logoColor=white" alt="Terminal" />
    <img src="https://img.shields.io/badge/JumpServer-1F2937" alt="JumpServer" />
    <img src="https://img.shields.io/badge/Port%20Forwarding-0F766E" alt="Port Forwarding" />
    <img src="https://img.shields.io/badge/Rust-000000?logo=rust&logoColor=white" alt="Rust" />
    <img src="https://img.shields.io/badge/GPUI-DEA584" alt="GPUI" />
  </p>

  <p>
    <a href="README_CN.md">中文</a> ·
    <a href="#install">Install</a> ·
    <a href="https://github.com/feigeCode/myterm/releases/latest">Latest Release</a> ·
    <a href="#features">Features</a> ·
    <a href="CONTRIBUTING.md">Contributing</a>
  </p>
</div>

## Overview

MyTerm is a native, cross-platform desktop client that brings remote-access tooling into a single tabbed workspace. It is written in Rust on top of [GPUI](https://gpui.rs), the GPU-accelerated UI framework from Zed, so the interface stays responsive without running inside a browser shell.

Connections are organized into **workspaces** on a home page. Each connection — SSH/SFTP, port forwarding, or JumpServer — is a card you can open, edit, duplicate, or delete. Open connections become tabs, with keyboard shortcuts for fast tab switching.

## Why MyTerm?

<table>
  <tr>
    <td width="50%">
      <h3>Native desktop, not a browser shell</h3>
      <p>Built with Rust and GPUI for GPU-accelerated rendering and a true native desktop feel on macOS, Windows, and Linux.</p>
    </td>
    <td width="50%">
      <h3>One workspace for remote access</h3>
      <p>SSH terminals, SFTP file transfer, local terminals, JumpServer bastion access, and port forwarding all live in the same tabbed window.</p>
    </td>
  </tr>
  <tr>
    <td>
      <h3>Bastion access that just works</h3>
      <p>Connect to JumpServer-managed assets over the Koko WebSocket terminal, including the full web login flow with captcha and MFA.</p>
    </td>
    <td>
      <h3>Encrypted connections</h3>
      <p>Credentials are encrypted at rest with a master key (AES-GCM), with an optional repository password to lock saved connections.</p>
    </td>
  </tr>
</table>

## Features

### SSH & Local Terminal

A full terminal experience powered by `alacritty_terminal`, with multi-tab sessions for both local shells and remote SSH hosts. The terminal layer includes:

- Search within the buffer (forward/backward), text selection, and copy/paste with configurable behavior (auto-copy, middle-click paste, multi-line paste confirmation).
- Command autocomplete and a path-sync option that keeps the working directory aligned across panes.
- A high-risk command confirmation guard, vi-style navigation mode, and adjustable font size with zoom shortcuts.
- A quick-command panel for saving, pinning, and reusing frequently run commands.

### SFTP File Management

Browse and transfer files on remote hosts through an SFTP sidebar docked next to the terminal:

- Drag-and-drop upload, directory navigation, and file operations.
- Path favorites and quick jumps to frequently used directories.
- Built on `russh` / `russh-sftp` for a pure-Rust SSH stack.

### JumpServer (JMS) Bastion

Connect to assets managed by a JumpServer bastion without leaving MyTerm. The integration uses the same web-session path your browser takes, so it works on instances that enforce image captcha and MFA:

- **Full login flow:** RSA + AES password encryption, image captcha, and MFA.
- **Koko WebSocket terminal:** interactive sessions tunneled through JumpServer's Koko component, with input, output, and resize.
- **Asset tree sidebar:** the asset tree is docked next to the terminal; expand nodes (lazy-loaded) and pick an account inline.
- **Server-side asset search:** search across all permitted assets, not just the loaded part of the tree.
- **Tab-per-asset:** the first asset connects in the current tab; subsequent assets open new tabs, each with its own asset tree.
- **Saved connections:** store URL, username, and encrypted password as a reusable connection card with credentials pre-filled.

### Port Forwarding

Create reusable SSH port forwarding connections from existing SSH/SFTP servers:

- **Local forwarding** for reaching databases or internal HTTP endpoints through a remote host.
- **Dynamic SOCKS tunnels** for routing tools through a remote host, implemented over SSH `direct-tcpip`.

### Remote File Editing

Edit remote files directly inside MyTerm with syntax highlighting and search/replace, without switching to a separate editor.

### Workspaces & Connection Management

- Group connections into workspaces and filter the home page by workspace or connection type.
- Quick-open dialog and search across connection name, host, username, and port.
- Duplicate, edit, and delete connections inline from the connection card.

### Security, Theming & i18n

- Connection credentials are encrypted at rest with a master key (AES-GCM); an optional repository password unlocks saved connections.
- Light and dark themes with a token-based design system, plus a configurable global HTTP proxy.
- Localized in English, Simplified Chinese, and Traditional Chinese.

## Install

Download the latest build from the [Releases](https://github.com/feigeCode/myterm/releases/latest) page.

| Platform | Architecture | Artifact |
|----------|--------------|----------|
| macOS | Apple Silicon, Intel | `.dmg`, `.tar.gz` |
| Linux | x86_64 | `.tar.gz` |
| Windows | x86_64 | `.zip` |

Checksums are published as `sha256sums.txt` in each release.

### macOS Gatekeeper

If macOS blocks the app after installing the DMG with "Apple cannot check it for malicious software", run:

```bash
sudo xattr -rd com.apple.quarantine /Applications/MyTerm.app
```

## Getting Started

1. Open MyTerm and create your first connection from the home page.
2. Add an SSH host and open a remote terminal, or start a local terminal.
3. Open the SFTP sidebar to browse remote directories or drag files in to upload.
4. Connect to a JumpServer bastion, log in with captcha/MFA, and pick an asset from the sidebar asset tree.
5. Create a port forwarding connection from an SSH host when you need a local tunnel or SOCKS proxy.

## Build From Source

### Prerequisites

- Rust (2024 edition)
- Platform-specific system dependencies

### System Dependencies

**macOS / Linux:**

```bash
./script/bootstrap
```

**Windows (PowerShell):**

```powershell
.\script\install-window.ps1
```

### Run

```bash
cargo run -p main
```

### Development Checks

```bash
# Build
cargo build

# Test
cargo test --all

# Lint
cargo clippy -- --deny warnings

# Format check
cargo fmt --check
```

See [CONTRIBUTING.md](CONTRIBUTING.md) for the full development guide.

## Architecture

MyTerm is a Cargo workspace (Rust 2024 edition). Key crates:

| Layer | Crate | Responsibility |
|-------|-------|----------------|
| Application | `main` | Entry point, home page, connection windows, settings, tab orchestration |
| Core | `crates/core` | Connection storage, encryption, configuration, tab container |
| UI library | `crates/ui` (gpui-component) | Reusable component library (60+ components) and theming |
| App UI | `crates/one_ui` | Application-specific components (cards, tables, editors) |
| Terminal | `crates/terminal`, `crates/terminal_view` | Terminal engine and view, sidebars (SFTP, quick commands) |
| SSH / SFTP | `crates/ssh`, `crates/sftp`, `crates/sftp_view` | SSH transport and SFTP file operations |
| Port forwarding | `crates/port_forwarding`, `crates/port_forwarding_view` | Local and dynamic SSH tunnels |
| JumpServer | `crates/jms` | JumpServer web login + Koko WebSocket terminal client |
| Remote editing | `crates/remote_file_editor` | Remote file editor with syntax highlighting |
| ER rendering | `crates/er_flow` | Diagram rendering (based on ferrum-flow) |
| WebView | `crates/webview` (gpui-wry) | WebView integration via Wry |

## Tech Stack

| Category | Technologies |
|----------|--------------|
| UI Framework | [GPUI](https://gpui.rs) |
| Language | Rust (2024 edition) |
| Terminal | alacritty_terminal |
| SSH / SFTP / Port Forwarding | russh, russh-sftp, SOCKS5 over SSH direct-tcpip |
| JumpServer (JMS) | Koko WebSocket terminal, tokio-tungstenite, rustls, RSA + AES web login |
| Text Editing | ropey, tree-sitter |
| Local Storage | rusqlite |
| Encryption | aes-gcm, sha2, ed25519 |
| HTTP Client | reqwest (Zed fork) |
| i18n | rust-i18n |

## FAQ

<details>
<summary><strong>What can MyTerm connect to?</strong></summary>

MyTerm focuses on remote access: SSH terminals, SFTP file transfer, local terminals, JumpServer bastion assets (over the Koko WebSocket terminal), and SSH port forwarding (local and dynamic SOCKS).
</details>

<details>
<summary><strong>How does the JumpServer integration work?</strong></summary>

MyTerm authenticates against JumpServer the same way the browser does — through the web login form with RSA + AES password encryption, image captcha, and MFA — then opens an interactive session over the Koko WebSocket terminal. It shows a dockable asset tree with server-side search and lets you open one tab per asset.
</details>

<details>
<summary><strong>Are my credentials safe?</strong></summary>

Connection credentials are encrypted at rest using a master key (AES-GCM). When a repository password is set, saved connections stay locked until you unlock them.
</details>

<details>
<summary><strong>Where can I download MyTerm?</strong></summary>

Use the GitHub [Releases](https://github.com/feigeCode/myterm/releases/latest) page. The release workflow publishes macOS, Linux, and Windows artifacts with checksums.
</details>

<details>
<summary><strong>How do I report bugs or request features?</strong></summary>

Open an issue on [GitHub Issues](https://github.com/feigeCode/myterm/issues). For code changes, please read [CONTRIBUTING.md](CONTRIBUTING.md) first.
</details>

## Community

- QQ Group: [860670605](https://qm.qq.com/cgi-bin/qm/qr?k=&group_code=860670605)

## Credits

ER diagram rendering is based on [ferrum-flow](https://github.com/tu6ge/ferrum-flow.git).

## License

Licensed under [Apache License 2.0](LICENSE-APACHE).

For licensing inquiries, contact xiaofei.hf@gmail.com.

## Star History

<a href="https://www.star-history.com/?repos=feigeCode%2Fmyterm&type=date&logscale=&legend=top-left">
 <picture>
   <source media="(prefers-color-scheme: dark)" srcset="https://api.star-history.com/chart?repos=feigeCode/myterm&type=date&theme=dark&logscale&legend=top-left" />
   <source media="(prefers-color-scheme: light)" srcset="https://api.star-history.com/chart?repos=feigeCode/myterm&type=date&logscale&legend=top-left" />
   <img alt="Star History Chart" src="https://api.star-history.com/chart?repos=feigeCode/myterm&type=date&logscale&legend=top-left" />
 </picture>
</a>
