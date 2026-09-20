<div align="center">
  <p>
    <img src="logo.svg" alt="MyTerm" width="120" />
  </p>

  <h1>MyTerm</h1>

  <p><strong>A native, minimalist desktop workspace: SSH, SFTP, local terminal, JumpServer, and port forwarding.</strong></p>



## Overview

MyTerm is a minimalist desktop terminal tool with the JumpServer web terminal login flow built in. It is written in Rust on top of the [GPUI](https://gpui.rs) framework.


## Getting Started

1. Open MyTerm, set a master password, and create your first connection from the home page.
2. Add an SSH host and open a remote terminal, or start a local terminal directly.
3. Open the SFTP sidebar to browse remote directories, or drag files in to upload.
4. Connect to a JumpServer bastion, log in with captcha/MFA, and pick an asset from the sidebar asset tree.
5. Create a port forwarding connection from an SSH host to set up a local tunnel or SOCKS proxy on demand.


## Tech Stack

| Category | Technologies |
|----------|--------------|
| UI Framework | [GPUI](https://gpui.rs) |
| Language | Rust |
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

MyTerm authenticates against JumpServer the same way the browser does — through the web login form with RSA + AES password encryption, image captcha, and MFA — then opens an interactive session over the Koko WebSocket terminal. It provides a dockable asset tree with server-side search.
</details>

<details>
<summary><strong>Are my credentials safe?</strong></summary>

Connection credentials are encrypted at rest using a master key (AES-GCM). Once a repository password is set, saved connections stay locked until you unlock them.
</details>
