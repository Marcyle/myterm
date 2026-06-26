# JMS 连接流程详细记录

> 本文档基于对 `https://jms.txxy.com/` 的实际抓包和 Playwright 自动化测试整理，
> 用于指导 `crates/jms` 及 `main/src/jms_connection_window.rs` 的实现与优化。

---

## 1. 认证流程（Web 登录）

JumpServer v4 采用 Django 表单认证，**必须**先获取 Web session cookie，后续 REST API、
connect-token 创建、Koko WebSocket 握手都依赖该 session。

### 1.1 获取登录页

```http
GET /core/auth/login/ HTTP/1.1
```

响应设置 Cookie：

- `jms_csrftoken` —— Django CSRF Token
- `jms_public_key` —— Base64 编码的 RSA 公钥（用于前端加密密码）
- `jms_sessionid` —— 匿名 session

页面 HTML 中还包含：

```html
<input type="hidden" name="csrfmiddlewaretoken" value="...">
```

### 1.2 提交用户名密码

```http
POST /core/auth/login/ HTTP/1.1
Content-Type: application/x-www-form-urlencoded
Cookie: jms_csrftoken=...; jms_sessionid=...
X-CSRFToken: ...
Referer: https://jms.txxy.com/core/auth/login/
```

请求体：

```
csrfmiddlewaretoken=...
&username=<用户名>
&password=<RSA+AES加密后的密码>
```

密码加密算法（与前端 `encryptPassword` 一致）：

1. 生成随机 AES Key（类似 `(Math.random() + 1).toString(36).substring(2)`）
2. 用 RSA 公钥加密 AES Key
3. 用 AES-128-ECB + ZeroPadding 加密密码明文
4. 结果格式：`base64(rsa_key_cipher):base64(aes_password_cipher)`

响应：

- 成功 → `302` → `/core/auth/login/guard/`
- 需要 MFA → `302` → `/core/auth/login/mfa/`
- 失败 → 仍返回登录页 HTML

### 1.3 MFA 验证

```http
POST /core/auth/login/mfa/ HTTP/1.1
Content-Type: application/x-www-form-urlencoded
```

请求体：

```
csrfmiddlewaretoken=...
&mfa_type=otp
&code=<6位OTP>
```

响应成功后最终重定向到 `/`，后续再进入 Luna `/luna/` 或 UI `/ui/#/console/dashboard`。

### 1.4 登录后的关键 Cookie

| Cookie | 作用 |
|--------|------|
| `jms_sessionid` | Django Session，httpOnly |
| `jms_csrftoken` | CSRF Token |
| `jms_public_key` | RSA 公钥 |
| `X-JMS-ORG` | 当前组织 ID |

---

## 2. 会话保持（避免每次登录）

在普通浏览器中，只要 `jms_sessionid` 未过期，用户访问 `/luna/` 时即自动登录。

**优化建议**：

- 将 `jms_sessionid`、`jms_csrftoken`、`jms_public_key`、`X-JMS-ORG` 持久化保存；
- 启动 `JmsClient` 时优先恢复这些 Cookie；
- 调用 `/api/v1/users/profile/` 校验 session 是否仍有效；
- 仅当 session 失效时才要求用户重新输入密码 + OTP。

---

## 3. 资产树加载

### 3.1 获取资产树

```http
GET /api/v1/perms/users/self/nodes/children-with-assets/tree/ HTTP/1.1
X-JMS-ORG: ...
Cookie: jms_sessionid=...; jms_csrftoken=...
```

返回扁平的 ztree 节点列表，每个节点包含：

- `id` —— 树节点 ID
- `name` / `title`
- `pId` —— 父节点 ID
- `isParent` —— 是否有子节点
- `meta.type` —— `"asset"` 或 `"node"`
- `meta.data.id` —— 资产真实 UUID（仅 asset 节点）
- `meta.data.key` —— 懒加载 key

### 3.2 懒加载子节点

```http
GET /api/v1/perms/users/self/nodes/children-with-assets/tree/?key=<node_key> HTTP/1.1
```

### 3.3 按 IP 搜索资产

```http
GET /api/v1/assets/assets/?address=10.60.23.100 HTTP/1.1
```

返回资产列表，可用于快速定位目标机器。

---

## 4. 获取资产账号

### 4.1 推荐端点

```http
GET /api/v1/assets/assets/<asset_id>/ HTTP/1.1
```

响应中 `accounts` 数组包含可用账号：

```json
{
  "id": "7ea2e557-d1e9-4e70-b737-1ea4d842f9b7",
  "name": "hngl_ops_admin_10.60.23.100",
  "address": "10.60.23.100",
  "protocols": [
    { "name": "sftp", "port": 22 },
    { "name": "ssh", "port": 22 }
  ],
  "accounts": [
    {
      "id": "ad1eee94-79db-4ff0-b137-de77f79542d6",
      "name": "zhaojh",
      "username": "zhaojh",
      "secret_type": { "value": "password", "label": "Password" }
    },
    {
      "id": "eecc9342-2834-4724-8bdc-230bc374a0c8",
      "name": "yw",
      "username": "yw",
      "secret_type": { "value": "ssh_key", "label": "SSH Key" }
    }
  ]
}
```

### 4.2 账号字段说明

- `name` / `username`：在大多数情况下相同，**connect-token 请求中使用 `name` 作为 account 参数**；
- `secret_type.value`：`password` / `ssh_key`，决定后端使用密码或私钥连接目标资产。

---

## 5. 创建连接 Token

### 5.1 请求

```http
POST /api/v1/authentication/connection-token/ HTTP/1.1
Content-Type: application/json
X-CSRFToken: ...
X-JMS-ORG: ...
Cookie: jms_sessionid=...; jms_csrftoken=...
Referer: https://jms.txxy.com/luna/?oid=...
```

请求体：

```json
{
  "asset": "7ea2e557-d1e9-4e70-b737-1ea4d842f9b7",
  "account": "yw",
  "protocol": "ssh",
  "input_username": "yw",
  "input_secret": "",
  "connect_method": "web_cli",
  "connect_options": {
    "file_name_conflict_resolution": "replace",
    "terminal_theme_name": "Default"
  }
}
```

注意：

- `account` 使用账号的 `name` 字段；
- `connect_method` 对于 SSH Web CLI 固定为 `web_cli`；
- 必须有 `X-CSRFToken` 和正确的 `Cookie`，否则返回 `403 CSRF Failed`。

### 5.2 响应

```json
{
  "id": "01e1c1fd-de99-4502-a3f0-9c390e0d3e73",
  "value": "XhcWvgNW54H03MGP",
  "asset": { "id": "...", "name": "hngl_ops_admin_10.60.23.100" },
  "account": "yw",
  "protocol": "ssh",
  "connect_method": "web_cli",
  "date_expired": "2026/06/25 15:26:35 +0800"
}
```

后续 Koko WebSocket 使用 `id`（UUID）作为 `?token=` 参数；
value 在 native SSH Client 模式下作为一次性密码。

---

## 6. Koko WebSocket 连接

### 6.1 WebSocket URL

```
wss://jms.txxy.com/koko/ws/terminal/?disableautohash=false&token=<token_id>&_=<timestamp>
```

### 6.2 握手 Header

```http
Sec-WebSocket-Protocol: JMS-KOKO
Sec-WebSocket-Extensions: permessage-deflate; client_max_window_bits
Origin: https://jms.txxy.com
Cookie: X-JMS-LUNA-ORG=...; X-JMS-ORG=...; django_language=zh-hans; jms_sessionid=...
```

注意：该 JMS 实例**必须**携带 `jms_sessionid` Cookie，否则 WS 握手会被 302 到登录页。

### 6.3 消息协议

JSON 信封：

```json
{ "id": "", "type": "TERMINAL_DATA", "data": "base64(...)" }
```

关键消息类型：

| 类型 | 方向 | 说明 |
|------|------|------|
| `CONNECT` | 服务端 → 客户端 | 携带服务端分配的终端 id，后续上行消息 id 必须复用 |
| `TERMINAL_INIT` | 客户端 → 服务端 | 发送初始终端尺寸 `{cols, rows, code:""}` |
| `TERMINAL_DATA` | 双向 | data 为 base64 编码的字节流 |
| `TERMINAL_RESIZE` | 客户端 → 服务端 | 调整终端尺寸 |
| `PING` / `PONG` | 双向 | 心跳 |
| `CLOSE` | 服务端 → 客户端 | 连接关闭 |

---

## 7. Web CLI 浏览器页面

除了 WebSocket，也可以直接让浏览器/WebView 打开：

```
https://jms.txxy.com/koko/connect?token=<token_id>
```

该页面会渲染完整的 xterm.js 终端，无需应用自己实现 Koko 协议。
适用于：

- 快速验证连接 token 是否有效；
- 在应用内嵌 WebView 展示终端。

---

## 8. 连接方式列表

```http
GET /api/v1/terminal/components/connect-methods/ HTTP/1.1
```

返回按协议分类的可用连接方式，例如 SSH 下包含：

- `web_cli` —— Web CLI（Koko）
- `ssh_client` —— 本地 SSH 客户端
- `ssh_guide` —— SSH 连接指引

SFTP 下包含：

- `web_sftp` —— Web SFTP（打开 `/koko/elfinder/sftp/`）

未来可据此扩展协议选择。

---

## 9. 优化 checklist

- [ ] 持久化 `jms_sessionid` / `jms_csrftoken` / `jms_public_key` / `X-JMS-ORG`；
- [ ] 启动时优先校验并恢复 session，避免重复 MFA；
- [ ] 账号列表优先使用 `/api/v1/assets/assets/{id}/`；
- [ ] 支持按 IP / 地址搜索资产；
- [ ] connect-token 请求携带完整 Cookie 和 CSRF；
- [ ] Koko WS 握手必须携带 `jms_sessionid` Cookie；
- [ ] 支持 Web CLI 页面 URL `/koko/connect?token=` 作为备选。
