<div align="center">
  <p>
    <img src="logo.svg" alt="MyTerm" width="120" />
  </p>

  <h1>MyTerm</h1>

  <p><strong>原生桌面工作台：SSH、SFTP、本地终端、JumpServer 堡垒机、端口转发，一站式集成。</strong></p>

  <p>
    基于 <a href="https://gpui.rs">GPUI</a> 构建 · Rust 原生桌面 · GPU 加速渲染
  </p>

  <p>
    <a href="https://github.com/feigeCode/myterm/releases"><img src="https://img.shields.io/github/downloads/feigeCode/myterm/total?style=for-the-badge&color=blue" alt="Downloads" /></a>
    <a href="https://github.com/feigeCode/myterm/actions/workflows/ci.yml"><img src="https://img.shields.io/github/actions/workflow/status/feigeCode/myterm/ci.yml?branch=main&style=for-the-badge" alt="CI" /></a>
    <a href="LICENSE-APACHE"><img src="https://img.shields.io/badge/license-Apache--2.0-blue?style=for-the-badge" alt="License" /></a>
    <a href="https://qm.qq.com/cgi-bin/qm/qr?k=&group_code=860670605"><img src="https://img.shields.io/badge/QQ%20群-860670605-EB1923?style=for-the-badge&logo=tencentqq&logoColor=white" alt="QQ 群 860670605" /></a>
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
    <a href="README.md">English</a> ·
    <a href="#安装">安装</a> ·
    <a href="https://github.com/feigeCode/myterm/releases/latest">最新版本</a> ·
    <a href="#功能特性">功能特性</a> ·
    <a href="CONTRIBUTING.md">参与贡献</a>
  </p>
</div>

## 概述

MyTerm 是一款原生跨平台桌面客户端，将远程访问工具整合到一个多标签工作台中。它使用 Rust 编写，基于 Zed 的 GPU 加速 UI 框架 [GPUI](https://gpui.rs)，无需运行在浏览器壳中，界面流畅且响应迅速。

连接在首页按**工作区**组织。每个连接（SSH/SFTP、端口转发或 JumpServer）都是一张卡片，可打开、编辑、复制或删除。打开的连接以标签页形式呈现，并支持快捷键快速切换标签。

## 为什么选择 MyTerm？

<table>
  <tr>
    <td width="50%">
      <h3>原生桌面，而非浏览器壳</h3>
      <p>基于 Rust 与 GPUI 构建，在 macOS、Windows、Linux 上提供 GPU 加速渲染与真正的原生桌面体验。</p>
    </td>
    <td width="50%">
      <h3>远程访问，一窗搞定</h3>
      <p>SSH 终端、SFTP 文件传输、本地终端、JumpServer 堡垒机访问、端口转发，全部集中在同一个多标签窗口。</p>
    </td>
  </tr>
  <tr>
    <td>
      <h3>开箱即用的堡垒机接入</h3>
      <p>通过 Koko WebSocket 终端连接 JumpServer 托管的资产，完整支持图形验证码与 MFA 的 Web 登录流程。</p>
    </td>
    <td>
      <h3>加密的连接</h3>
      <p>凭据基于主密钥（AES-GCM）静态加密存储，并可选设置仓库密码以锁定已保存的连接。</p>
    </td>
  </tr>
</table>

## 功能特性

### SSH 与本地终端

由 `alacritty_terminal` 驱动的完整终端体验，支持本地 Shell 与远程 SSH 主机的多标签会话：

- 缓冲区内搜索（向前/向后）、文本选择，以及可配置的复制粘贴行为（自动复制、中键粘贴、多行粘贴确认）。
- 命令自动补全，以及在面板间保持工作目录一致的路径同步选项。
- 高危命令二次确认、vi 风格导航模式，以及带缩放快捷键的字号调节。
- 快捷命令面板，可保存、置顶、复用常用命令。

### SFTP 文件管理

通过停靠在终端旁的 SFTP 侧边栏浏览与传输远程主机文件：

- 拖拽上传、目录导航与文件操作。
- 路径收藏与常用目录快速跳转。
- 基于 `russh` / `russh-sftp` 的纯 Rust SSH 栈。

### JumpServer（JMS）堡垒机

无需离开 MyTerm 即可连接 JumpServer 托管的资产。集成采用与浏览器一致的 Web 会话路径，因此在强制图形验证码与 MFA 的实例上同样可用：

- **完整登录流程：** RSA + AES 密码加密、图形验证码与 MFA。
- **Koko WebSocket 终端：** 通过 JumpServer 的 Koko 组件建立交互式会话，支持输入、输出与窗口尺寸调整。
- **资产树侧边栏：** 资产树停靠在终端旁，节点懒加载展开，可内联选择账号。
- **服务端资产搜索：** 在全部有权限的资产中搜索，而非仅在已加载的部分。
- **一资产一标签：** 第一个资产在当前标签连接，后续资产各自打开新标签，每个标签拥有独立的资产树。
- **保存连接：** 将 URL、用户名与加密密码保存为可复用的连接卡片，并自动回填凭据。

### 端口转发

基于已有 SSH/SFTP 服务器创建可复用的 SSH 端口转发连接：

- **本地转发：** 通过远程主机访问数据库或内网 HTTP 端点。
- **动态 SOCKS 隧道：** 基于 SSH `direct-tcpip` 实现，将工具流量路由经远程主机。

### 远程文件编辑

直接在 MyTerm 中编辑远程文件，支持语法高亮与查找替换，无需切换到独立编辑器。

### 工作区与连接管理

- 将连接归入工作区，并在首页按工作区或连接类型筛选。
- 快速打开对话框，支持按连接名称、主机、用户名、端口搜索。
- 在连接卡片上内联复制、编辑、删除连接。

### 安全、主题与多语言

- 连接凭据基于主密钥（AES-GCM）静态加密；设置仓库密码后，保存的连接在解锁前保持锁定。
- 基于 Token 的设计系统，支持浅色/深色主题，以及可配置的全局 HTTP 代理。
- 支持英文、简体中文、繁体中文。

## 安装

从 [Releases](https://github.com/feigeCode/myterm/releases/latest) 页面下载最新构建。

| 平台 | 架构 | 产物 |
|------|------|------|
| macOS | Apple Silicon、Intel | `.dmg`、`.tar.gz` |
| Linux | x86_64 | `.tar.gz` |
| Windows | x86_64 | `.zip` |

每个版本都会发布 `sha256sums.txt` 校验和。

### macOS Gatekeeper

如果通过 DMG 安装后 macOS 提示"无法验证开发者"而阻止运行，请执行：

```bash
sudo xattr -rd com.apple.quarantine /Applications/MyTerm.app
```

## 快速上手

1. 打开 MyTerm，在首页创建第一个连接。
2. 添加 SSH 主机并打开远程终端，或直接启动本地终端。
3. 打开 SFTP 侧边栏浏览远程目录，或拖入文件进行上传。
4. 连接 JumpServer 堡垒机，通过验证码/MFA 登录，并从侧边栏资产树选择资产。
5. 基于 SSH 主机创建端口转发连接，按需建立本地隧道或 SOCKS 代理。

## 从源码构建

### 环境要求

- Rust（2024 edition）
- 平台相关系统依赖

### 系统依赖

**macOS / Linux：**

```bash
./script/bootstrap
```

**Windows（PowerShell）：**

```powershell
.\script\install-window.ps1
```

### 运行

```bash
cargo run -p main
```

### 开发检查

```bash
# 构建
cargo build

# 测试
cargo test --all

# Lint
cargo clippy -- --deny warnings

# 格式检查
cargo fmt --check
```

完整开发指南见 [CONTRIBUTING.md](CONTRIBUTING.md)。

## 架构

MyTerm 是一个 Cargo workspace（Rust 2024 edition），主要 crate 如下：

| 层级 | Crate | 职责 |
|------|-------|------|
| 应用 | `main` | 入口、首页、连接窗口、设置、标签编排 |
| 核心 | `crates/core` | 连接存储、加密、配置、标签容器 |
| UI 组件库 | `crates/ui`（gpui-component） | 可复用组件库（60+ 组件）与主题 |
| 应用 UI | `crates/one_ui` | 应用专属组件（卡片、表格、编辑器） |
| 终端 | `crates/terminal`、`crates/terminal_view` | 终端引擎与视图，侧边栏（SFTP、快捷命令） |
| SSH / SFTP | `crates/ssh`、`crates/sftp`、`crates/sftp_view` | SSH 传输与 SFTP 文件操作 |
| 端口转发 | `crates/port_forwarding`、`crates/port_forwarding_view` | 本地与动态 SSH 隧道 |
| JumpServer | `crates/jms` | JumpServer Web 登录 + Koko WebSocket 终端客户端 |
| 远程编辑 | `crates/remote_file_editor` | 带语法高亮的远程文件编辑器 |
| ER 渲染 | `crates/er_flow` | 图表渲染（基于 ferrum-flow） |
| WebView | `crates/webview`（gpui-wry） | 通过 Wry 集成 WebView |

## 技术栈

| 类别 | 技术 |
|------|------|
| UI 框架 | [GPUI](https://gpui.rs) |
| 语言 | Rust（2024 edition） |
| 终端 | alacritty_terminal |
| SSH / SFTP / 端口转发 | russh、russh-sftp、SSH direct-tcpip 之上的 SOCKS5 |
| JumpServer（JMS） | Koko WebSocket 终端、tokio-tungstenite、rustls、RSA + AES Web 登录 |
| 文本编辑 | ropey、tree-sitter |
| 本地存储 | rusqlite |
| 加密 | aes-gcm、sha2、ed25519 |
| HTTP 客户端 | reqwest（Zed fork） |
| 多语言 | rust-i18n |

## 常见问题

<details>
<summary><strong>MyTerm 能连接哪些目标？</strong></summary>

MyTerm 专注于远程访问：SSH 终端、SFTP 文件传输、本地终端、JumpServer 堡垒机资产（通过 Koko WebSocket 终端），以及 SSH 端口转发（本地与动态 SOCKS）。
</details>

<details>
<summary><strong>JumpServer 集成是如何工作的？</strong></summary>

MyTerm 以与浏览器一致的方式向 JumpServer 认证——通过 Web 登录表单完成 RSA + AES 密码加密、图形验证码与 MFA——随后通过 Koko WebSocket 终端打开交互式会话。它提供可停靠、支持服务端搜索的资产树，并支持一资产一标签。
</details>

<details>
<summary><strong>我的凭据安全吗？</strong></summary>

连接凭据使用主密钥（AES-GCM）静态加密。设置仓库密码后，保存的连接在解锁前保持锁定。
</details>

<details>
<summary><strong>在哪里下载 MyTerm？</strong></summary>

请使用 GitHub [Releases](https://github.com/feigeCode/myterm/releases/latest) 页面。发布流程会发布带校验和的 macOS、Linux、Windows 产物。
</details>

<details>
<summary><strong>如何反馈 Bug 或提交功能需求？</strong></summary>

在 [GitHub Issues](https://github.com/feigeCode/myterm/issues) 提交 issue。如需提交代码变更，请先阅读 [CONTRIBUTING.md](CONTRIBUTING.md)。
</details>

## 社区

- QQ 群：[860670605](https://qm.qq.com/cgi-bin/qm/qr?k=&group_code=860670605)

## 致谢

ER 图渲染基于 [ferrum-flow](https://github.com/tu6ge/ferrum-flow.git)。

## 许可证

基于 [Apache License 2.0](LICENSE-APACHE) 授权。

许可相关咨询请联系 xiaofei.hf@gmail.com。
