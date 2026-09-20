//! JMS API 客户端

use gpui::http_client::{self, HttpClient, http};
use serde_json::Value;
use std::sync::Arc;

use crate::models::*;

/// JMS API 客户端
///
/// 采用纯 Web Session 认证:通过 `/core/auth/login/` 表单登录获取**已认证**的
/// `jms_sessionid`,后续资产树、账号列表、connect-token、Koko 全部复用同一 session
/// + `X-CSRFToken`。不再使用 API Bearer token(其返回的是匿名 session,Koko 不认)。
///
/// 字段全部 cheaply-cloneable,可 Clone 后交给终端侧栏长期持有(只读资产树/账号)。
#[derive(Clone)]
pub struct JmsClient {
    client: Arc<dyn HttpClient>,
    base_url: String,
    /// 已认证的 jms_sessionid
    session_cookie: Option<String>,
    /// 组织 ID
    org_id: String,
    /// CSRF token(jms_csrftoken),写操作必须携带 X-CSRFToken 头
    csrf_token: Option<String>,
    /// 登录页 RSA 公钥(base64 PEM),用于加密密码
    public_key: Option<String>,
    /// 最近一次验证码的 key(captcha_0),提交时回填
    last_captcha_key: Option<String>,
}

impl JmsClient {
    /// 创建新的 JMS 客户端
    pub fn new(base_url: &str, client: Arc<dyn HttpClient>) -> Self {
        Self {
            client,
            base_url: base_url.trim_end_matches('/').to_string(),
            session_cookie: None,
            org_id: "00000000-0000-0000-0000-000000000002".to_string(),
            csrf_token: None,
            public_key: None,
            last_captcha_key: None,
        }
    }

    /// 发送 HTTP 请求，自动处理 Cookie 和会话管理
    async fn request(
        &mut self,
        method: &str,
        url: &str,
        body: Option<&Value>,
    ) -> Result<(u16, String), JmsError> {
        let mut builder = http::Request::builder()
            .method(method)
            .uri(url)
            .header("Accept", "application/json");

        // 添加组织 ID
        builder = builder.header("X-JMS-ORG", &self.org_id);

        // 纯 Web Session 认证:携带已认证的 jms_sessionid + csrf cookie
        if let Some(cookie) = &self.session_cookie {
            let cookie_value = self.build_cookie_header(cookie, self.csrf_token.as_deref());
            builder = builder.header("Cookie", &cookie_value);
        }

        // 写操作必须携带 X-CSRFToken,否则 Django 返回 403 CSRF Failed
        if method != "GET" {
            if let Some(ref csrf) = self.csrf_token {
                builder = builder.header("X-CSRFToken", csrf);
                builder = builder.header("Referer", &self.base_url);
            }
        }

        let body_str = if let Some(body) = body {
            let s = serde_json::to_string(body).map_err(|e| JmsError::ParseError(e.to_string()))?;
            builder = builder.header("Content-Type", "application/json");
            s
        } else {
            String::new()
        };

        let request = builder
            .body(http_client::AsyncBody::from(body_str))
            .map_err(|e| JmsError::NetworkError(e.to_string()))?;

        let response = self
            .client
            .send(request)
            .await
            .map_err(|e| JmsError::NetworkError(e.to_string()))?;

        // 提取 session cookie / csrf token——来自 Set-Cookie 响应头
        let cookies = Self::parse_all_cookies(response.headers());
        self.absorb_session_cookies(&cookies);

        let status = response.status().as_u16();

        let mut body = response.into_body();
        let mut bytes = Vec::new();
        use futures::AsyncReadExt;
        body.read_to_end(&mut bytes)
            .await
            .map_err(|e| JmsError::NetworkError(e.to_string()))?;
        let text = String::from_utf8(bytes).map_err(|e| JmsError::ParseError(e.to_string()))?;

        Ok((status, text))
    }

    /// 构造浏览器风格 Cookie 头。
    ///
    /// JumpServer 的 Django 会话以 `sessionid`/`jms_sessionid` 双写跟踪同一登录态，
    /// 只携带其中一个会被服务端判定为另一会话，因此所有请求路径统一双写。
    fn build_cookie_header(&self, sid: &str, csrf: Option<&str>) -> String {
        let mut cookie = format!(
            "jms_sessionid={sid}; sessionid={sid}; X-JMS-ORG={org}; django_language=zh-hans",
            sid = sid,
            org = self.org_id
        );
        if let Some(csrf) = csrf {
            cookie.push_str(&format!("; jms_csrftoken={csrf}; csrftoken={csrf}"));
        }
        cookie
    }

    /// 从解析出的 Cookie 表中回填 session/csrf（`jms_sessionid` 优先，回退 `sessionid`）。
    fn absorb_session_cookies(&mut self, cookies: &std::collections::HashMap<String, String>) {
        if let Some(sid) = cookies
            .get("jms_sessionid")
            .or_else(|| cookies.get("sessionid"))
        {
            self.session_cookie = Some(sid.clone());
        }
        if let Some(csrf) = cookies
            .get("jms_csrftoken")
            .or_else(|| cookies.get("csrftoken"))
            .filter(|s| !s.is_empty())
        {
            self.csrf_token = Some(csrf.clone());
        }
    }

    /// 从 Set-Cookie 字符串中解析 jms_sessionid（生产路径走 `parse_all_cookies` + `absorb_session_cookies`，此函数仅服务单测）
    #[cfg(test)]
    fn parse_session_id(cookie_str: &str) -> Option<String> {
        Self::parse_cookie_value(cookie_str, "jms_sessionid")
            .or_else(|| Self::parse_cookie_value(cookie_str, "sessionid"))
    }

    /// 从 Set-Cookie 字符串中解析指定名称的 cookie 值
    #[cfg(test)]
    fn parse_cookie_value(cookie_str: &str, name: &str) -> Option<String> {
        let prefix = format!("{}=", name);
        for part in cookie_str.split(';') {
            let part = part.trim();
            if let Some(value) = part.strip_prefix(&prefix) {
                if value.is_empty() {
                    return None;
                }
                return Some(Self::normalize_cookie_value(value));
            }
        }
        None
    }

    /// 按浏览器 `document.cookie` 的读取方式规范化 Cookie 值。
    ///
    /// JumpServer 的 `jms_public_key` 可能以 URL 编码、双引号包裹的形式出现在
    /// `Set-Cookie` 中；前端先 `decodeURIComponent`，再移除引号后才交给 RSA。
    fn normalize_cookie_value(value: &str) -> String {
        let decoded = urlencoding::decode(value)
            .map(|value| value.into_owned())
            .unwrap_or_else(|_| value.to_string());
        decoded
            .trim()
            .trim_matches(|ch| ch == '"' || ch == '\'')
            .to_string()
    }

    /// 发送 POST 请求
    async fn post_json(&mut self, url: &str, body: &Value) -> Result<(u16, String), JmsError> {
        self.request("POST", url, Some(body)).await
    }

    /// 发送 GET 请求
    async fn get_json(&mut self, url: &str) -> Result<(u16, String), JmsError> {
        self.request("GET", url, None).await
    }

    /// 设置 session cookie（从认证响应中获取）
    pub fn set_session_cookie(&mut self, cookie: String) {
        self.session_cookie = Some(cookie);
    }

    /// 获取 session cookie
    pub fn session_cookie(&self) -> Option<&str> {
        self.session_cookie.as_deref()
    }

    /// 获取资产树
    pub async fn get_asset_tree(&mut self) -> Result<Vec<JmsAssetNode>, JmsError> {
        let url = format!(
            "{}/api/v1/perms/users/self/nodes/children-with-assets/tree/",
            self.base_url
        );
        let (status, text) = self.get_json(&url).await?;

        if (200..300).contains(&status) {
            let nodes: Vec<JmsAssetNode> =
                serde_json::from_str(&text).map_err(|e| JmsError::ParseError(e.to_string()))?;
            Ok(nodes)
        } else {
            Err(JmsError::ApiError(format!("获取资产树失败: {}", text)))
        }
    }

    /// 懒加载：获取指定节点 key 下的子节点（含资产）
    ///
    /// JumpServer 树是懒加载的，点击节点时通过 `?key=<node_key>` 获取该节点的直接子节点。
    pub async fn get_node_children(
        &mut self,
        node_key: &str,
    ) -> Result<Vec<JmsAssetNode>, JmsError> {
        let url = format!(
            "{}/api/v1/perms/users/self/nodes/children-with-assets/tree/?key={}",
            self.base_url, node_key
        );
        let (status, text) = self.get_json(&url).await?;

        if (200..300).contains(&status) {
            let nodes: Vec<JmsAssetNode> =
                serde_json::from_str(&text).map_err(|e| JmsError::ParseError(e.to_string()))?;
            Ok(nodes)
        } else {
            Err(JmsError::ApiError(format!("获取子节点失败: {}", text)))
        }
    }

    /// 按关键字搜索资产(服务端搜索,返回扁平的资产节点列表)
    ///
    /// 资产树是懒加载的,客户端只持有已展开的部分,因此搜索走服务端
    /// `/api/v1/perms/users/self/assets/tree/?search=<keyword>` 端点,
    /// 直接返回匹配的资产叶子节点(与 nodes 树端点不同,后者忽略 search 参数)。
    pub async fn search_assets(&mut self, keyword: &str) -> Result<Vec<JmsAssetNode>, JmsError> {
        let encoded = urlencoding::encode(keyword);
        let url = format!(
            "{}/api/v1/perms/users/self/assets/tree/?search={}",
            self.base_url, encoded
        );
        let (status, text) = self.get_json(&url).await?;

        if (200..300).contains(&status) {
            let nodes: Vec<JmsAssetNode> =
                serde_json::from_str(&text).map_err(|e| JmsError::ParseError(e.to_string()))?;
            // 仅保留资产叶子节点(过滤目录节点)
            let assets = nodes
                .into_iter()
                .filter(|n| {
                    n.meta
                        .as_ref()
                        .map(|m| m.node_type == "asset")
                        .unwrap_or(false)
                })
                .collect();
            Ok(assets)
        } else {
            Err(JmsError::ApiError(format!("搜索资产失败: {}", text)))
        }
    }

    /// 获取资产关联的账号列表
    pub async fn list_asset_accounts(
        &mut self,
        asset_id: &str,
    ) -> Result<Vec<JmsAssetAccount>, JmsError> {
        // 不同 JumpServer 版本的账号接口路径不同;单个 404 不能中断后续探测。
        let candidates = [
            format!(
                "{}/api/v1/assets/accounts/?asset={}",
                self.base_url, asset_id
            ),
            format!(
                "{}/api/v1/accounts/accounts/?asset={}",
                self.base_url, asset_id
            ),
            format!(
                "{}/api/v1/accounts/assets/{}/accounts/",
                self.base_url, asset_id
            ),
            format!(
                "{}/api/v1/perms/users/self/assets/{}/accounts/",
                self.base_url, asset_id
            ),
        ];
        let mut last_error = None;

        for url in &candidates {
            match self.get_json(url).await {
                Ok((status, text)) => {
                    if (200..300).contains(&status) {
                        match serde_json::from_str::<Vec<JmsAssetAccount>>(&text) {
                            Ok(accounts) if !accounts.is_empty() => return Ok(accounts),
                            Ok(_) => {
                                last_error = Some("接口返回空账号列表".to_string());
                            }
                            Err(e) => {
                                last_error = Some(format!("解析账号列表失败: {e}"));
                            }
                        }
                    } else {
                        last_error = Some(format!("status={status}"));
                    }
                }
                Err(e) => {
                    last_error = Some(e.to_string());
                }
            }
        }

        Err(JmsError::ApiError(format!(
            "无法获取账号列表，已探测所有候选接口: {}",
            last_error.unwrap_or_else(|| "没有可用响应".to_string())
        )))
    }

    /// 创建 Koko 连接 token（用于 WebSocket 终端）
    pub async fn create_connect_token(
        &mut self,
        asset_id: &str,
        account: &str,
    ) -> Result<JmsConnectToken, JmsError> {
        // JumpServer v4: POST /api/v1/authentication/connection-token/
        let url = format!("{}/api/v1/authentication/connection-token/", self.base_url);

        let body = serde_json::json!({
            "asset": asset_id,
            "account": account,
            "protocol": "ssh",
            "input_username": account,
            "input_secret": "",
            "connect_method": "web_cli",
            "connect_options": {
                "charset": "default",
                "disableautohash": false,
                "resolution": "auto",
                "backspaceAsCtrlH": false,
                "appletConnectMethod": "web",
                "reusable": false
            }
        });
        let (status, text) = self.post_json(&url, &body).await?;

        if (200..300).contains(&status) {
            let token: JmsConnectToken =
                serde_json::from_str(&text).map_err(|e| JmsError::ParseError(e.to_string()))?;
            Ok(token)
        } else {
            Err(JmsError::ApiError(format!("创建连接 token 失败: {}", text)))
        }
    }

    /// 通过 JumpServer Web 表单登录获取**已认证**的会话 cookie
    ///
    /// 完整复刻浏览器登录流程:
    /// 1. 首次调用(`refresh=true`)先 GET `/core/auth/login/` 获取 csrf/public_key/匿名 session
    /// 2. POST 表单(密码 RSA+AES 加密),携带可选验证码/MFA
    /// 3. 返回 [`WebLoginResult`] 区分成功/需验证码/需 MFA/失败
    ///
    /// 成功后 `jms_sessionid` 被标记为已认证,可用于资产树、connect-token、Koko。
    pub async fn web_login(
        &mut self,
        username: &str,
        password: &str,
        captcha_value: Option<&str>,
        otp_code: Option<&str>,
        refresh_page: bool,
    ) -> Result<WebLoginResult, JmsError> {
        let login_url = format!("{}/core/auth/login/", self.base_url);

        // 1. GET 登录页,获取 csrf token、public key、匿名 session
        //    首次登录或显式刷新时执行;带验证码/MFA 重试时复用已有 csrf/session
        if refresh_page || self.public_key.is_none() {
            let (status, text, cookies) = self.raw_get(&login_url).await?;
            if status != 200 {
                return Err(JmsError::ApiError(format!(
                    "获取登录页失败: status={}",
                    status
                )));
            }

            self.absorb_session_cookies(&cookies);
            if self.csrf_token.is_none() {
                // Cookie 中没有 csrf 时从登录页 HTML 中提取
                let csrf = Self::extract_csrf_from_html(&text);
                if !csrf.is_empty() {
                    self.csrf_token = Some(csrf);
                }
            }
            if let Some(pk) = cookies.get("jms_public_key") {
                self.public_key = Some(pk.clone());
            }

            // 检查首次加载登录页时是否就已经需要验证码（例如服务端配置了强制验证码或存在失败尝试）
            if captcha_value.is_none() {
                let needs_captcha = text.contains("captcha_0")
                    || text.contains("captcha-field")
                    || text.contains("captcha-challenge")
                    || text.contains("name=\"captcha\"");
                if needs_captcha {
                    if let Some(info) = Self::parse_captcha_from_html(&text, &self.base_url) {
                        self.last_captcha_key = Some(info.key.clone());
                        let info = self.fill_captcha_image(info).await;
                        return Ok(WebLoginResult::CaptchaRequired(info));
                    }
                }
            }
        }

        let csrf_token = self.csrf_token.clone().unwrap_or_default();
        let public_key = self.public_key.clone().unwrap_or_default();
        let login_session_id = self.session_cookie.clone().unwrap_or_default();

        // 2. 加密密码(RSA+AES 混合,与浏览器 encryptPassword 一致)
        // 浏览器只有在没有公钥时才回退明文。公钥存在但解析失败时继续提交明文，
        // 会让密码校验失败；如果此时启用了验证码，服务端会重新返回验证码页，
        // 表面上就变成了“验证码一直不对”。
        let encrypted_password = if public_key.trim().is_empty() {
            password.to_string()
        } else {
            Self::encrypt_password_with_public_key(password, &public_key)
                .map_err(|e| JmsError::AuthError(format!("解析 JMS 公钥并加密密码失败: {e}")))?
        };

        // 3. 构造表单
        let mut form_map = serde_json::Map::new();
        form_map.insert("username".to_string(), Value::String(username.to_string()));
        form_map.insert("password".to_string(), Value::String(encrypted_password));
        form_map.insert(
            "csrfmiddlewaretoken".to_string(),
            Value::String(csrf_token.clone()),
        );
        form_map.insert("next".to_string(), Value::String("/luna/".to_string()));
        if let Some(value) = captcha_value.filter(|s| !s.is_empty()) {
            // 验证码 key 从最近一次解析的 captcha_info 获取
            if let Some(ref info) = self.last_captcha_key {
                form_map.insert("captcha_0".to_string(), Value::String(info.clone()));
            }
            form_map.insert("captcha_1".to_string(), Value::String(value.to_string()));
        }
        if let Some(code) = otp_code.filter(|s| !s.is_empty()) {
            form_map.insert("otp_code".to_string(), Value::String(code.to_string()));
        }
        let form = Value::Object(form_map);

        let headers = self.build_web_headers(&login_url, &login_session_id, &csrf_token);
        let (status, text, cookies) = self.raw_post_form(&login_url, &form, headers).await?;

        // 更新 session / csrf
        self.absorb_session_cookies(&cookies);

        // 4. 判定结果
        let location = cookies.get("location").cloned().unwrap_or_default();

        // 302 跳转:逐跳跟随重定向链(POST→guard→mfa/luna),直到落到实页面
        if (300..400).contains(&status) && !location.is_empty() {
            return self.follow_login_redirects(&location, otp_code).await;
        }

        // 200:仍在登录/MFA 页,按响应体判定
        self.classify_login_page(&text, otp_code).await
    }

    /// 跟随登录重定向链(最多若干跳),落到 luna/ui 即成功,落到 MFA/登录页则判定
    async fn follow_login_redirects(
        &mut self,
        first_location: &str,
        otp_code: Option<&str>,
    ) -> Result<WebLoginResult, JmsError> {
        let mut location = first_location.to_string();
        for _ in 0..6 {
            if location.contains("/luna") || location.contains("/ui") {
                if let Some(ref sid) = self.session_cookie {
                    return Ok(WebLoginResult::Success {
                        session_id: sid.clone(),
                    });
                }
            }
            let next_url = Self::absolute_url(&self.base_url, &location);
            let (status, body, cookies) = self.raw_get(&next_url).await?;
            self.absorb_session_cookies(&cookies);

            // 继续重定向
            if (300..400).contains(&status) {
                if let Some(loc) = cookies.get("location") {
                    if loc.contains("/luna") || loc.contains("/ui") {
                        if let Some(ref sid) = self.session_cookie {
                            return Ok(WebLoginResult::Success {
                                session_id: sid.clone(),
                            });
                        }
                    }
                    location = loc.clone();
                    continue;
                }
            }

            // 200:已到实页面(MFA 页 / 登录页),按内容判定
            if next_url.contains("/luna") || next_url.contains("/ui") {
                if let Some(ref sid) = self.session_cookie {
                    return Ok(WebLoginResult::Success {
                        session_id: sid.clone(),
                    });
                }
            }
            return Box::pin(self.classify_login_page(&body, otp_code)).await;
        }
        Err(JmsError::AuthError("登录重定向次数过多".to_string()))
    }

    /// 根据登录/MFA 页面 HTML 判定下一步(验证码 / MFA / 成功 / 失败)
    async fn classify_login_page(
        &mut self,
        text: &str,
        otp_code: Option<&str>,
    ) -> Result<WebLoginResult, JmsError> {
        let still_login_page = text.contains("login-form") || text.contains("id=\"login-form\"");
        let needs_captcha = text.contains("captcha_0")
            || text.contains("captcha-field")
            || text.contains("captcha-challenge")
            || text.contains("name=\"captcha\"");
        let needs_mfa = text.contains("name=\"otp_code\"")
            || text.contains("mfa-form")
            || text.contains("id=\"otp_code\"")
            || text.contains("mfa-select")
            || text.contains("name=\"mfa_type\"")
            || text.contains("/core/auth/login/mfa")
            || text.contains("/core/auth/mfa");
        let err_msg = Self::extract_login_error(text);

        // 既不是登录页也不是 MFA 页 → 已认证成功
        if !still_login_page && !needs_mfa {
            if let Some(ref sid) = self.session_cookie {
                return Ok(WebLoginResult::Success {
                    session_id: sid.clone(),
                });
            }
        }

        // 验证码优先:无论是否同时需要 MFA,先让用户过验证码
        if needs_captcha && err_msg.is_some() {
            // 仅当有错误(验证码错误)时才回退到验证码;首次成功提交不应再要验证码
            if let Some(info) = Self::parse_captcha_from_html(text, &self.base_url) {
                self.last_captcha_key = Some(info.key.clone());
                let info = self.fill_captcha_image(info).await;
                return Ok(WebLoginResult::CaptchaRequired(info));
            } else if let Ok(info) = self.refresh_captcha().await {
                return Ok(WebLoginResult::CaptchaRequired(info));
            }
        }
        if needs_captcha && !needs_mfa {
            if let Some(info) = Self::parse_captcha_from_html(text, &self.base_url) {
                self.last_captcha_key = Some(info.key.clone());
                let info = self.fill_captcha_image(info).await;
                return Ok(WebLoginResult::CaptchaRequired(info));
            } else if let Ok(info) = self.refresh_captcha().await {
                return Ok(WebLoginResult::CaptchaRequired(info));
            }
        }
        if needs_mfa {
            // 已提供 OTP → 直接提交 MFA 表单
            if let Some(code) = otp_code.filter(|s| !s.is_empty()) {
                return Box::pin(self.submit_mfa(text, code)).await;
            };
            return Ok(WebLoginResult::MfaRequired);
        }

        Err(JmsError::AuthError(
            err_msg.unwrap_or_else(|| "Web 登录失败".to_string()),
        ))
    }

    /// 在 MFA 页提交 OTP 验证码(由 UI 在 MfaRequired 后调用)
    ///
    /// 先 GET MFA 页拿到最新 csrf,再 POST `code`+`mfa_type`。
    pub async fn submit_mfa_code(&mut self, code: &str) -> Result<WebLoginResult, JmsError> {
        let mfa_url = format!("{}/core/auth/login/mfa/", self.base_url);
        let (_, page, cookies) = self.raw_get(&mfa_url).await?;
        self.absorb_session_cookies(&cookies);
        // 若已被打回登录页/已认证,交给通用判定
        if !page.contains("mfa_type") && !page.contains("mfa-select") {
            return self.classify_login_page(&page, Some(code)).await;
        }
        self.submit_mfa(&page, code).await
    }

    /// 在 MFA 页提交 OTP 验证码(POST /core/auth/login/mfa/)
    async fn submit_mfa(
        &mut self,
        mfa_page_html: &str,
        code: &str,
    ) -> Result<WebLoginResult, JmsError> {
        let mfa_url = format!("{}/core/auth/login/mfa/", self.base_url);
        let csrf = {
            let from_page = Self::extract_csrf_from_html(mfa_page_html);
            if from_page.is_empty() {
                self.csrf_token.clone().unwrap_or_default()
            } else {
                self.csrf_token = Some(from_page.clone());
                from_page
            }
        };

        let mut form_map = serde_json::Map::new();
        form_map.insert(
            "csrfmiddlewaretoken".to_string(),
            Value::String(csrf.clone()),
        );
        form_map.insert("mfa_type".to_string(), Value::String("otp".to_string()));
        form_map.insert("code".to_string(), Value::String(code.to_string()));
        let form = Value::Object(form_map);

        let session_id = self.session_cookie.clone().unwrap_or_default();
        let headers = self.build_web_headers(&mfa_url, &session_id, &csrf);

        let (status, text, cookies) = self.raw_post_form(&mfa_url, &form, headers).await?;
        self.absorb_session_cookies(&cookies);

        // MFA 成功 → 302 到 guard/luna,跟随重定向链
        let location = cookies.get("location").cloned().unwrap_or_default();
        if (300..400).contains(&status) && !location.is_empty() {
            return Box::pin(self.follow_login_redirects(&location, None)).await;
        }
        // 200:可能 OTP 错误,仍在 MFA 页
        Box::pin(self.classify_login_page(&text, None)).await
    }

    /// 刷新验证码,返回新的验证码图片信息
    pub async fn refresh_captcha(&mut self) -> Result<CaptchaInfo, JmsError> {
        let refresh_url = format!("{}/core/auth/captcha/refresh/", self.base_url.trim_end_matches('/'));
        let login_url = format!("{}/core/auth/login/", self.base_url.trim_end_matches('/'));

        // 优先通过标准 AJAX 刷新端点获取验证码
        let mut builder = http::Request::builder()
            .method("GET")
            .uri(&refresh_url)
            .header("Accept", "application/json, text/javascript, */*; q=0.01")
            .header("X-Requested-With", "XMLHttpRequest")
            .header(
                "User-Agent",
                "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36",
            )
            .header("Referer", &login_url);

        if let Some(csrf) = &self.csrf_token {
            builder = builder.header("X-CSRFToken", csrf);
        }
        if let Some(sid) = &self.session_cookie {
            let cookie = self.build_cookie_header(sid, self.csrf_token.as_deref());
            builder = builder.header("Cookie", cookie);
        }
        builder = builder.extension(http_client::RedirectPolicy::NoFollow);

        if let Ok(request) = builder.body(http_client::AsyncBody::from(String::new())) {
            if let Ok(response) = self.client.send(request).await {
                let status = response.status().as_u16();
                if status == 200 {
                    let cookies = Self::parse_all_cookies(response.headers());
                    self.absorb_session_cookies(&cookies);

                    let body = response.into_body();
                    if let Ok(text) = Self::read_body_to_string(body).await {
                        if let Ok(json) = serde_json::from_str::<Value>(&text) {
                            let key = json
                                .get("key")
                                .and_then(|v| v.as_str())
                                .unwrap_or_default()
                                .to_string();
                            let image_url_raw = json
                                .get("image_url")
                                .and_then(|v| v.as_str())
                                .unwrap_or_default();
                            if !key.is_empty() {
                                let image_url = if image_url_raw.is_empty() {
                                    format!(
                                        "{}/core/auth/captcha/image/{}/",
                                        self.base_url.trim_end_matches('/'),
                                        key
                                    )
                                } else {
                                    Self::absolute_url(&self.base_url, image_url_raw)
                                };
                                self.last_captcha_key = Some(key.clone());
                                let info = CaptchaInfo {
                                    key,
                                    image_url,
                                    image_data: None,
                                    image_mime: None,
                                };
                                let info = self.fill_captcha_image(info).await;
                                if info.image_data.is_some() {
                                    return Ok(info);
                                }
                            }
                        }
                    }
                }
            }
        }

        // 兜底方案：AJAX refresh 失败时，重新请求登录页获取全新会话与验证码
        tracing::info!("AJAX 刷新验证码未成功，回退到重新请求登录页获取验证码");
        let (status, text, cookies) = self.raw_get(&login_url).await?;
        if status != 200 {
            return Err(JmsError::ApiError(format!(
                "获取登录页刷新验证码失败: status={}",
                status
            )));
        }
        self.absorb_session_cookies(&cookies);
        if let Some(pk) = cookies.get("jms_public_key") {
            self.public_key = Some(pk.clone());
        }

        if let Some(info) = Self::parse_captcha_from_html(&text, &self.base_url) {
            self.last_captcha_key = Some(info.key.clone());
            let info = self.fill_captcha_image(info).await;
            return Ok(info);
        }

        Err(JmsError::ApiError("登录页未找到有效验证码".to_string()))
    }

    /// 下载验证码图片字节填充进 CaptchaInfo
    async fn fill_captcha_image(&mut self, mut info: CaptchaInfo) -> CaptchaInfo {
        if info.image_url.is_empty() {
            return info;
        }
        match self.raw_get_bytes(&info.image_url).await {
            Ok((bytes, mime)) => {
                info.image_data = Some(bytes);
                info.image_mime = mime;
            }
            Err(_) => {}
        }
        info
    }

    /// 从登录页 HTML 解析验证码 key 和图片地址
    fn parse_captcha_from_html(html: &str, base_url: &str) -> Option<CaptchaInfo> {
        // 匹配 name="captcha_0" 的 input 标签提取 key，兼容 value 在 name 前或后
        let key_re = regex::Regex::new(
            r#"(?:name=['"]captcha_0['"][^>]*value=['"]([^'"]+)['"]|value=['"]([^'"]+)['"][^>]*name=['"]captcha_0['"])"#,
        )
        .ok()?;
        let key = key_re
            .captures(html)
            .and_then(|c| c.get(1).or_else(|| c.get(2)))
            .map(|m| m.as_str().to_string())?;

        // 匹配验证码图片 URL，若未匹配到则按 JumpServer 路由规则默认构造
        let img_re = regex::Regex::new(
            r#"src=['"]([^'"]*(?:/captcha/image/[^'"]+|/core/auth/captcha/image/[^'"]+))['"]"#,
        )
        .ok()?;
        let image_url = img_re
            .captures(html)
            .and_then(|c| c.get(1))
            .map(|m| Self::absolute_url(base_url, m.as_str()))
            .unwrap_or_else(|| {
                format!(
                    "{}/core/auth/captcha/image/{}/",
                    base_url.trim_end_matches('/'),
                    key
                )
            });
        Some(CaptchaInfo {
            key,
            image_url,
            image_data: None,
            image_mime: None,
        })
    }

    /// 把相对 URL 拼成绝对 URL
    fn absolute_url(base_url: &str, path: &str) -> String {
        if path.starts_with("http") {
            path.to_string()
        } else {
            format!("{}{}", base_url.trim_end_matches('/'), path)
        }
    }

    /// 构造 Web 表单提交所需的浏览器风格请求头
    fn build_web_headers(
        &self,
        login_url: &str,
        session_id: &str,
        csrf_token: &str,
    ) -> std::collections::HashMap<String, String> {
        let mut headers = std::collections::HashMap::new();
        let cookie_header = self.build_cookie_header(session_id, Some(csrf_token));
        headers.insert("Cookie".to_string(), cookie_header);
        headers.insert("Referer".to_string(), login_url.to_string());
        headers.insert("X-CSRFToken".to_string(), csrf_token.to_string());
        headers.insert(
            "Accept".to_string(),
            "text/html,application/xhtml+xml,application/xml;q=0.9,*/*;q=0.8".to_string(),
        );
        headers.insert("Accept-Language".to_string(), "zh-CN,zh;q=0.9".to_string());
        headers.insert("Origin".to_string(), self.base_url.clone());
        headers.insert("Sec-Fetch-Dest".to_string(), "document".to_string());
        headers.insert("Sec-Fetch-Mode".to_string(), "navigate".to_string());
        headers.insert("Sec-Fetch-Site".to_string(), "same-origin".to_string());
        headers.insert("Upgrade-Insecure-Requests".to_string(), "1".to_string());
        headers
    }

    /// 从 HTML 中提取 csrfmiddlewaretoken
    fn extract_csrf_from_html(html: &str) -> String {
        let re =
            regex::Regex::new(r#"name=['\"]csrfmiddlewaretoken['\"]\s+value=['\"]([^'\"]+)['\"]"#)
                .unwrap_or_else(|_| regex::Regex::new(r"never").unwrap());
        re.captures(html)
            .and_then(|c| c.get(1))
            .map(|m| m.as_str().to_string())
            .unwrap_or_default()
    }

    /// 发送 GET 请求并返回所有 Set-Cookie(携带当前 session/csrf cookie)
    async fn raw_get(
        &mut self,
        url: &str,
    ) -> Result<(u16, String, std::collections::HashMap<String, String>), JmsError> {
        let mut builder = http::Request::builder()
            .method("GET")
            .uri(url)
            .header(
                "Accept",
                "text/html,application/xhtml+xml,application/xml;q=0.9,*/*;q=0.8",
            )
            .header(
                "User-Agent",
                "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36",
            );

        // 携带当前会话 cookie,否则 guard/MFA 等中间页会判定未认证并打回登录
        if let Some(sid) = &self.session_cookie {
            let cookie = self.build_cookie_header(sid, self.csrf_token.as_deref());
            builder = builder.header("Cookie", cookie);
        }
        // 中间跳转页同样不自动跟随,便于逐跳读取 Set-Cookie
        builder = builder.extension(http_client::RedirectPolicy::NoFollow);

        let request = builder
            .body(http_client::AsyncBody::from(String::new()))
            .map_err(|e| JmsError::NetworkError(e.to_string()))?;

        let response = self
            .client
            .send(request)
            .await
            .map_err(|e| JmsError::NetworkError(format!("GET {} 失败: {}", url, e)))?;

        let status = response.status().as_u16();
        let mut cookies = Self::parse_all_cookies(response.headers());
        if let Some(loc) = response
            .headers()
            .get("location")
            .and_then(|v| v.to_str().ok())
        {
            cookies.insert("location".to_string(), loc.to_string());
        }
        let body = response.into_body();
        let text = Self::read_body_to_string(body).await?;
        Ok((status, text, cookies))
    }

    /// 发送 GET 请求并返回原始字节 + Content-Type(用于下载验证码图片)
    async fn raw_get_bytes(&mut self, url: &str) -> Result<(Vec<u8>, Option<String>), JmsError> {
        let login_url = format!("{}/core/auth/login/", self.base_url.trim_end_matches('/'));
        let mut builder = http::Request::builder()
            .method("GET")
            .uri(url)
            .header(
                "Accept",
                "image/avif,image/webp,image/apng,image/*,*/*;q=0.8",
            )
            .header(
                "User-Agent",
                "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36",
            )
            .header("Referer", &login_url);
        if let Some(sid) = &self.session_cookie {
            let cookie = self.build_cookie_header(sid, self.csrf_token.as_deref());
            builder = builder.header("Cookie", cookie);
        }
        let request = builder
            .body(http_client::AsyncBody::from(String::new()))
            .map_err(|e| JmsError::NetworkError(e.to_string()))?;

        let response = self
            .client
            .send(request)
            .await
            .map_err(|e| JmsError::NetworkError(format!("GET {} 失败: {}", url, e)))?;

        let status = response.status().as_u16();
        if status != 200 {
            return Err(JmsError::ApiError(format!("下载图片失败: status={}", status)));
        }

        let mime = response
            .headers()
            .get("content-type")
            .and_then(|v| v.to_str().ok())
            .map(|s| s.to_string());
        let mut body = response.into_body();
        let mut bytes = Vec::new();
        use futures::AsyncReadExt;
        body.read_to_end(&mut bytes)
            .await
            .map_err(|e| JmsError::NetworkError(e.to_string()))?;
        Ok((bytes, mime))
    }

    async fn raw_post_form(
        &mut self,
        url: &str,
        form: &Value,
        extra_headers: std::collections::HashMap<String, String>,
    ) -> Result<(u16, String, std::collections::HashMap<String, String>), JmsError> {
        // 将 JSON object 转为 urlencoded
        let body_str = form
            .as_object()
            .map(|obj| {
                obj.iter()
                    .map(|(k, v)| {
                        format!(
                            "{}={}",
                            urlencoding::encode(k),
                            urlencoding::encode(v.as_str().unwrap_or(""))
                        )
                    })
                    .collect::<Vec<_>>()
                    .join("&")
            })
            .unwrap_or_default();

        let mut builder = http::Request::builder()
            .method("POST")
            .uri(url)
            .header(
                "Accept",
                "text/html,application/xhtml+xml,application/xml;q=0.9,*/*;q=0.8",
            )
            .header("Content-Type", "application/x-www-form-urlencoded")
            .header(
                "User-Agent",
                "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36",
            );

        for (k, v) in extra_headers {
            builder = builder.header(k, v);
        }

        // 不自动跟随 302:登录成功的「已认证 session」由 302 响应的 Set-Cookie 携带,
        // 若跟随重定向到 /luna/,我们只会读到最终响应,丢失已认证 cookie,导致后续 401。
        builder = builder.extension(http_client::RedirectPolicy::NoFollow);

        let request = builder
            .body(http_client::AsyncBody::from(body_str))
            .map_err(|e| JmsError::NetworkError(e.to_string()))?;

        let response = self
            .client
            .send(request)
            .await
            .map_err(|e| JmsError::NetworkError(format!("POST {} 失败: {}", url, e)))?;

        let status = response.status().as_u16();
        let mut cookies = Self::parse_all_cookies(response.headers());
        // 记录 Location 头,用于判定登录是否成功跳转
        if let Some(loc) = response
            .headers()
            .get("location")
            .and_then(|v| v.to_str().ok())
        {
            cookies.insert("location".to_string(), loc.to_string());
        }
        let body = response.into_body();
        let text = Self::read_body_to_string(body).await?;
        Ok((status, text, cookies))
    }

    /// 读取完整响应体
    async fn read_body_to_string<B>(mut body: B) -> Result<String, JmsError>
    where
        B: futures::AsyncReadExt + Unpin,
    {
        let mut bytes = Vec::new();
        body.read_to_end(&mut bytes)
            .await
            .map_err(|e| JmsError::NetworkError(e.to_string()))?;
        String::from_utf8(bytes).map_err(|e| JmsError::ParseError(e.to_string()))
    }

    /// 解析所有 Set-Cookie 到 map
    fn parse_all_cookies(headers: &http::HeaderMap) -> std::collections::HashMap<String, String> {
        let mut cookies = std::collections::HashMap::new();
        for (name, value) in headers {
            if name.as_str().eq_ignore_ascii_case("set-cookie") {
                if let Ok(s) = value.to_str() {
                    if let Some((k, v)) = s.split_once('=') {
                        let v = v.split(';').next().unwrap_or("");
                        cookies.insert(k.trim().to_string(), Self::normalize_cookie_value(v));
                    }
                }
            }
        }
        cookies
    }

    /// 从登录页 HTML 中提取错误提示文本
    fn extract_login_error(html: &str) -> Option<String> {
        let re = regex::Regex::new(r#"<div[^>]*class\s*=\s*['"]alert[^'"]*['"][^>]*>(.*?)</div>"#)
            .ok()?;
        re.captures(html)
            .and_then(|c| c.get(1))
            .map(|m| {
                let s = m.as_str();
                // 去掉 HTML 标签并清理空白
                let s = regex::Regex::new(r"<[^>]+>").unwrap().replace_all(s, "");
                s.trim().to_string()
            })
            .filter(|s| !s.is_empty())
    }

    /// 使用 jms_public_key（PEM RSA 公钥）加密密码
    ///
    /// JumpServer v4 前端采用 RSA+AES 混合加密:
    /// 1. 生成随机 AES key
    /// 2. 用 RSA 公钥加密 AES key
    /// 3. 用 AES-128-ECB/ZeroPadding 加密密码
    /// 4. 返回 `base64(rsa_encrypted_key):base64(aes_encrypted_password)`
    fn encrypt_password_with_public_key(
        password: &str,
        public_key_cookie_value: &str,
    ) -> anyhow::Result<String> {
        use base64::Engine as _;
        use rsa::pkcs8::DecodePublicKey;
        use rsa::{Pkcs1v15Encrypt, RsaPublicKey};

        let public_key_cookie_value = Self::normalize_cookie_value(public_key_cookie_value);

        // cookie 值通常是 base64(PEM)
        let pem = if public_key_cookie_value.contains("BEGIN PUBLIC KEY")
            || public_key_cookie_value.contains("BEGIN RSA PUBLIC KEY")
        {
            public_key_cookie_value
        } else {
            let decoded = base64::engine::general_purpose::STANDARD
                .decode(&public_key_cookie_value)
                .map_err(|e| anyhow::anyhow!("base64 解码 jms_public_key 失败: {e}"))?;
            String::from_utf8(decoded)
                .map_err(|e| anyhow::anyhow!("jms_public_key 解码后不是 UTF-8: {e}"))?
        };

        let public_key = RsaPublicKey::from_public_key_pem(&pem)
            .or_else(|_| {
                use rsa::pkcs1::DecodeRsaPublicKey;
                RsaPublicKey::from_pkcs1_pem(&pem)
            })
            .map_err(|e| anyhow::anyhow!("解析 RSA 公钥失败: {e}"))?;

        // 1. 生成随机 AES key(模拟 JS `(Math.random()+1).toString(36).substring(2)` 行为)
        let aes_key = Self::generate_random_aes_key();

        // 2. RSA 加密 AES key
        let mut rng = rand::thread_rng();
        let encrypted_key = public_key
            .encrypt(&mut rng, Pkcs1v15Encrypt, aes_key.as_bytes())
            .map_err(|e| anyhow::anyhow!("RSA 加密 AES key 失败: {e}"))?;

        // 3. AES-128-ECB ZeroPadding 加密密码
        let encrypted_password = Self::aes128_ecb_zero_padding_encrypt(password, &aes_key)
            .map_err(|e| anyhow::anyhow!("AES 加密密码失败: {e}"))?;

        Ok(format!(
            "{}:{}",
            base64::engine::general_purpose::STANDARD.encode(encrypted_key),
            base64::engine::general_purpose::STANDARD.encode(encrypted_password)
        ))
    }

    /// 生成随机 AES key 字符串(长度 16,可安全填充到 AES-128 key)
    fn generate_random_aes_key() -> String {
        use rand::Rng as _;
        const CHARSET: &[u8] = b"abcdefghijklmnopqrstuvwxyz0123456789";
        let mut rng = rand::thread_rng();
        (0..16)
            .map(|_| CHARSET[rng.gen_range(0..CHARSET.len())] as char)
            .collect()
    }

    /// AES-128-ECB ZeroPadding 加密
    fn aes128_ecb_zero_padding_encrypt(plaintext: &str, key: &str) -> anyhow::Result<Vec<u8>> {
        use aes::Aes128;
        use aes::cipher::{BlockEncrypt, KeyInit};

        let key_bytes = Self::fill_key(key);
        let cipher = Aes128::new_from_slice(&key_bytes)
            .map_err(|e| anyhow::anyhow!("AES key 初始化失败: {e}"))?;

        let mut bytes = plaintext.as_bytes().to_vec();
        let rem = bytes.len() % 16;
        if rem != 0 {
            bytes.resize(bytes.len() + (16 - rem), 0u8);
        }

        let mut output = Vec::with_capacity(bytes.len());
        for chunk in bytes.chunks_exact(16) {
            let mut block = aes::cipher::generic_array::GenericArray::clone_from_slice(chunk);
            cipher.encrypt_block(&mut block);
            output.extend_from_slice(&block);
        }
        Ok(output)
    }

    /// 把 UTF-8 key 字符串填充/截断为 16 字节(AES-128)
    fn fill_key(key: &str) -> Vec<u8> {
        const KEY_LEN: usize = 16;
        let key_bytes = key.as_bytes();
        let mut filled = vec![0u8; KEY_LEN];
        let len = key_bytes.len().min(KEY_LEN);
        filled[..len].copy_from_slice(&key_bytes[..len]);
        filled
    }

    /// 获取 JMS 服务器基础地址
    pub fn base_url(&self) -> &str {
        &self.base_url
    }

    /// 获取组织 ID
    pub fn org_id(&self) -> &str {
        &self.org_id
    }
}

/// Web 表单登录结果
#[derive(Debug, Clone)]
pub enum WebLoginResult {
    /// 登录成功,获得已认证 session
    Success { session_id: String },
    /// 需要输入图片验证码
    CaptchaRequired(CaptchaInfo),
    /// 需要输入 MFA 验证码
    MfaRequired,
}

/// JMS 错误类型
#[derive(Debug, thiserror::Error)]
pub enum JmsError {
    #[error("网络错误: {0}")]
    NetworkError(String),

    #[error("认证失败: {0}")]
    AuthError(String),

    #[error("解析错误: {0}")]
    ParseError(String),

    #[error("API 错误: {0}")]
    ApiError(String),

    #[error("未认证")]
    NotAuthenticated,
}

#[cfg(test)]
mod tests {
    use super::JmsClient;
    use gpui::http_client::http;

    #[test]
    fn normalizes_browser_cookie_encoding() {
        assert_eq!(
            JmsClient::normalize_cookie_value("%22a%2Bb%2Fc%3D%3D%22"),
            "a+b/c=="
        );
    }

    #[test]
    fn parses_quoted_cookie_value() {
        assert_eq!(
            JmsClient::parse_cookie_value("jms_public_key=\"abc\"; Path=/", "jms_public_key"),
            Some("abc".to_string())
        );
    }

    #[test]
    fn normalizes_set_cookie_values() {
        let mut headers = http::HeaderMap::new();
        headers.append(
            http::header::SET_COOKIE,
            http::HeaderValue::from_static("jms_public_key=%22abc%22; Path=/"),
        );

        assert_eq!(
            JmsClient::parse_all_cookies(&headers).get("jms_public_key"),
            Some(&"abc".to_string())
        );
    }

    #[test]
    fn parses_captcha_from_html_standard_and_reversed() {
        let html_std = r#"<input type="hidden" name="captcha_0" value="key_123"><img src="/core/auth/captcha/image/key_123/" class="captcha">"#;
        let info = JmsClient::parse_captcha_from_html(html_std, "https://jms.example.com").unwrap();
        assert_eq!(info.key, "key_123");
        assert_eq!(info.image_url, "https://jms.example.com/core/auth/captcha/image/key_123/");

        let html_rev = r#"<input type="hidden" value="key_456" name="captcha_0"><img class="captcha" src="/captcha/image/key_456/">"#;
        let info_rev = JmsClient::parse_captcha_from_html(html_rev, "https://jms.example.com").unwrap();
        assert_eq!(info_rev.key, "key_456");
        assert_eq!(info_rev.image_url, "https://jms.example.com/captcha/image/key_456/");

        let html_no_img = r#"<input type="hidden" name="captcha_0" value="key_789">"#;
        let info_no_img = JmsClient::parse_captcha_from_html(html_no_img, "https://jms.example.com").unwrap();
        assert_eq!(info_no_img.key, "key_789");
        assert_eq!(info_no_img.image_url, "https://jms.example.com/core/auth/captcha/image/key_789/");
    }

    #[test]
    fn parses_session_id_variants() {
        assert_eq!(
            JmsClient::parse_session_id("jms_sessionid=abc12345; Path=/"),
            Some("abc12345".to_string())
        );
        assert_eq!(
            JmsClient::parse_session_id("sessionid=xyz98765; Path=/"),
            Some("xyz98765".to_string())
        );
    }
}
