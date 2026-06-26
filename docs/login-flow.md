# JumpServer 登录流程分析

> 目标站点：`https://jms.txxy.com/`  
> 分析时间：2026-06-25  
> 工具：Playwright（headless Chromium）  
> 说明：本文档已对所有敏感凭据做脱敏处理，不保留真实密码与 OTP。

---

## 1. 概述

JumpServer 采用 **Django 表单登录 + 前端密码加密 + MFA(OTP) 二次验证** 的认证流程。

| 阶段 | URL | 说明 |
|------|-----|------|
| 入口 | `/` | 访问根地址 |
| 登录页 | `/core/auth/login/` | 重定向到 Django 登录表单 |
| 凭据提交 | `POST /core/auth/login/` | 提交用户名、加密后的密码、CSRF Token |
| MFA 页 | `/core/auth/login/mfa/` | 二次验证 OTP |
| MFA 提交 | `POST /core/auth/login/mfa/` | 提交 OTP Code |
| 登录成功 | `/ui/#/console/dashboard` | 前端 Vue 仪表盘 |

---

## 2. 登录页面结构

### 2.1 表单字段

表单 ID：`login-form`  
提交方式：`POST`  
Action：`https://jms.txxy.com/core/auth/login/`

| 字段名 | 类型 | 必填 | 说明 |
|--------|------|------|------|
| `csrfmiddlewaretoken` | hidden | 是 | Django CSRF Token，从页面表单中获取 |
| `username` | text | 是 | 用户名 |
| `password` | hidden (text) | 是 | 前端加密后的密码，真实提交字段 |
| `auto_login` | checkbox | 否 | "Remember me" |

注意：页面上可见的密码输入框是 `<input type="password" id="password">`（无 `name` 属性），
前端 `doLogin()` 函数会读取该输入框的明文、加密后写入隐藏的 `<input type="text" name="password" id="password-hidden">`，
最终表单提交的是隐藏字段。

### 2.2 前端登录触发

```javascript
function doLogin() {
    var password = $('#password').val(); // 明文密码
    var passwordEncrypted = encryptPassword(password);
    $('#password-hidden').val(passwordEncrypted);
    $('#login-form').submit();
}
```

---

## 3. 密码加密机制

JumpServer 在浏览器端使用 **RSA + AES** 混合加密密码。

### 3.1 密钥来源

- **RSA 公钥**：从 Cookie `jms_public_key` 中获取，值为 Base64 编码的 PEM 公钥。
- **AES Key**：前端随机生成 `(Math.random() + 1).toString(36).substring(2)`。

### 3.2 加密步骤

1. 生成随机 AES Key；
2. 用 RSA 公钥加密 AES Key，得到 `keyCipher`；
3. 用 AES(ECB 模式、ZeroPadding) 加密密码明文，得到 `passwordCipher`；
4. 表单提交的密码字段值为：

```
password = {keyCipher}:{passwordCipher}
```

### 3.3 AES 细节

- 模式：ECB
- Padding：ZeroPadding
- Key：将随机字符串填充/截断到 16 字节

> 如果直接通过后端或脚本模拟登录，需要复现上述 RSA+AES 加密逻辑，
> 或复用浏览器环境让前端 `encryptPassword()` 自行处理。

---

## 4. 完整登录流程

### 4.1 Step 1：访问入口

```
GET https://jms.txxy.com/
```

响应：`302` → `/core/auth/login/`

### 4.2 Step 2：加载登录页

```
GET https://jms.txxy.com/core/auth/login/
```

页面返回 Django 登录表单，同时设置以下 Cookie：

- `jms_public_key`：RSA 公钥
- `jms_csrftoken`：CSRF Token
- `jms_sessionid`：会话 ID（httpOnly）

### 4.3 Step 3：提交用户名密码

```
POST https://jms.txxy.com/core/auth/login/
Content-Type: application/x-www-form-urlencoded
```

请求体：

```
csrfmiddlewaretoken=xxx
&username=<用户名>
&password=<RSA+AES加密后的密码>
```

**响应**：`302` → `/core/auth/login/guard/`

### 4.4 Step 4：进入 MFA 流程

```
GET https://jms.txxy.com/core/auth/login/guard/
```

响应：`302` → `/core/auth/login/mfa/`

MFA 页面提供 OTP 输入框：

- `mfa_type=otp`
- `code=<6位OTP>`
- 仍需携带新的 `csrfmiddlewaretoken`

### 4.5 Step 5：提交 OTP

```
POST https://jms.txxy.com/core/auth/login/mfa/
Content-Type: application/x-www-form-urlencoded
```

请求体：

```
csrfmiddlewaretoken=xxx
&mfa_type=otp
&code=<OTP>
```

**响应**：`302` → `/core/auth/login/guard/?_=mfa_ok`

### 4.6 Step 6：登录成功跳转

```
GET https://jms.txxy.com/core/auth/login/guard/?_=mfa_ok
```

响应：`302` → `/`

```
GET https://jms.txxy.com/
```

最终重定向到前端 SPA 仪表盘：

```
https://jms.txxy.com/ui/#/console/dashboard
```

---

## 5. 重定向链总结

```
/                              (入口)
  → 302 /core/auth/login/
       POST /core/auth/login/  (凭据校验通过)
         → 302 /core/auth/login/guard/
              → 302 /core/auth/login/mfa/
                   POST /core/auth/login/mfa/  (OTP 校验通过)
                     → 302 /core/auth/login/guard/?_=mfa_ok
                          → 302 /
                               → /ui/#/console/dashboard
```

---

## 6. 关键接口

### 6.1 登录前/后都会调用的接口

| 接口 | 方法 | 登录前状态 | 登录后状态 | 说明 |
|------|------|-----------|-----------|------|
| `/api/v1/settings/i18n/lina/?lang=zh&flat=0` | GET | 200 | 200 | 前端国际化配置 |
| `/api/v1/authentication/user-session/` | GET | 403 | 200 | 用户会话状态 |
| `/api/v1/settings/public/open/` | GET | 200 | 200 | 公开配置 |
| `/api/v1/users/profile/` | GET | 401 | 200 | 用户信息 |
| `/api/v1/settings/public/` | GET | — | 200 | 系统设置 |

### 6.2 认证相关端点

- `POST /core/auth/login/`：提交用户名密码
- `GET /core/auth/login/guard/`：中间校验/跳转守卫
- `GET|POST /core/auth/login/mfa/`：MFA 验证
- `GET /core/auth/captcha/image/<captcha_0>/`：图形验证码（本环境未触发）

---

## 7. Cookie 说明

登录成功后浏览器持有的关键 Cookie：

| Cookie 名 | 作用 | 属性 |
|-----------|------|------|
| `SESSION_COOKIE_NAME_PREFIX` | 会话前缀，值为 `jms_` | 非 httpOnly |
| `jms_public_key` | RSA 公钥（Base64 PEM） | 非 httpOnly |
| `jms_csrftoken` | Django CSRF Token | 非 httpOnly |
| `jms_sessionid` | Django Session ID | **httpOnly** |
| `X-JMS-ORG` | 当前组织 ID，如 `00000000-0000-0000-0000-000000000002` | 非 httpOnly |

所有 Cookie 的 `SameSite=Lax`，`secure=false`（本站点使用 HTTP）。

---

## 8. Header 使用说明

### 8.1 登录表单提交

```http
POST /core/auth/login/ HTTP/1.1
Content-Type: application/x-www-form-urlencoded
Origin: https://jms.txxy.com
Referer: https://jms.txxy.com/core/auth/login/
```

### 8.2 MFA 提交

```http
POST /core/auth/login/mfa/ HTTP/1.1
Content-Type: application/x-www-form-urlencoded
Origin: https://jms.txxy.com
Referer: https://jms.txxy.com/core/auth/login/mfa/
```

### 8.3 登录后 API 请求

后续前端 SPA 调用 REST API 时，会携带：

```http
GET /api/v1/authentication/user-session/ HTTP/1.1
X-CSRFToken: <jms_csrftoken 的值>
X-TZ: Asia/Shanghai
Referer: https://jms.txxy.com/ui/
Accept: application/json, text/plain, */*
```

注意：
- API 调用需要 `Cookie: jms_sessionid=...; jms_csrftoken=...`；
- 部分请求显式附加 `X-CSRFToken` 头，值与 `jms_csrftoken` Cookie 一致。

---

## 9. 浏览器存储

### localStorage

登录成功后写入：

- `preOrg:<username>`：`null`
- `currentOrg:<username>`：当前组织 JSON，如

```json
{
  "id": "00000000-0000-0000-0000-000000000002",
  "name": "DEFAULT",
  "is_default": true,
  "is_root": false,
  "comment": ""
}
```

### sessionStorage

为空。

---

## 10. 集成建议（针对 myterm）

若要在 myterm 中集成 JumpServer 登录，可考虑以下两种方案：

### 方案 A：复用浏览器/ WebView

- 使用 WebView 加载 `https://jms.txxy.com/core/auth/login/`；
- 让前端 `doLogin()` 自动完成密码加密；
- 监听 URL 变化，当到达 `/ui/#/console/dashboard` 时提取 `jms_sessionid` Cookie；
- 后续 API 调用携带该 Cookie 和 `X-CSRFToken`。

### 方案 B：后端模拟登录

- 先 `GET /core/auth/login/` 获取 `jms_csrftoken` 与 `jms_public_key`；
- 使用 RSA+AES 加密密码（复现前文章节 3 的逻辑）；
- `POST /core/auth/login/` 提交表单；
- 跟随 302 跳转，处理 MFA 页；
- 用户输入 OTP 后 `POST /core/auth/login/mfa/`；
- 成功后保存 `jms_sessionid` 用于后续请求。

---

## 11. 注意事项

1. **密码不脱敏传输**：虽然走 HTTPS，但密码在客户端经过 RSA+AES 加密后才提交，服务端才能解密。
2. **CSRF 保护**：所有表单提交和 API 写操作都需要 `jms_csrftoken`。
3. **MFA 强制开启**：该站点要求 OTP 二次验证，连续失败 6 次会锁定账号 30 分钟。
4. **时区头**：API 请求中常见 `X-TZ: Asia/Shanghai`，用于后端时区处理。
5. **组织切换**：`X-JMS-ORG` Cookie 标识当前组织，切换组织时需要更新该 Cookie。
6. **验证码**：本次分析未触发图形验证码（HTML 类名为 `no-captcha-challenge`），
   但在失败次数较多或管理员配置下可能出现，字段为 `captcha_0` / `captcha_1`。

---

## 12. 附录：原始数据文件

- `/tmp/jms_login_final.json`：完整网络、Cookie、跳转数据（已脱敏）
- `/tmp/jms_login_final.har`：完整 HAR 网络记录
