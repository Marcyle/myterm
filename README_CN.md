<div align="center">
  <p>
    <img src="logo.svg" alt="MyTerm" width="120" />
  </p>

  <h1>MyTerm</h1>

  <p><strong>原生极简桌面工作台：支持SSH、SFTP、本地终端、JumpServer、端口转发。</strong></p>



## 概述

MyTerm 是一款极简风格的桌面终端工具，集成了 Jumpserver web terminal 登录流程。使用 Rust 编写，基于[GPUI](https://gpui.rs) 框架。

## 快速上手

1. 打开 MyTerm，设置一个主密码，在首页创建第一个连接。
2. 添加 SSH 主机并打开远程终端，或直接启动本地终端。
3. 打开 SFTP 侧边栏浏览远程目录，或拖入文件进行上传。
4. 连接 JumpServer 堡垒机，通过验证码/MFA 登录，并从侧边栏资产树选择资产。
5. 基于 SSH 主机创建端口转发连接，按需建立本地隧道或 SOCKS 代理。


## 技术栈

| 类别 | 技术 |
|------|------|
| UI 框架 | [GPUI](https://gpui.rs) |
| 语言 | Rust |
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

MyTerm 以与浏览器一致的方式向 JumpServer 认证——通过 Web 登录表单完成 RSA + AES 密码加密、图形验证码与 MFA——随后通过 Koko WebSocket 终端打开交互式会话。它提供可停靠、支持服务端搜索的资产树。
</details>

<details>
<summary><strong>我的凭据安全吗？</strong></summary>

连接凭据使用主密钥（AES-GCM）静态加密。设置仓库密码后，保存的连接在解锁前保持锁定。
</details>
