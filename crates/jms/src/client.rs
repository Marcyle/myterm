//! JMS API 客户端

use gpui::http_client::{self, http, HttpClient};
use serde_json::Value;
use std::sync::Arc;

use crate::models::*;

/// 安全地截取字符串前若干字节用于日志预览。
///
/// 直接用 `&s[..n]` 按字节切片在 `n` 落入多字节 UTF-8 字符中间时会 panic
/// (例如响应体含中文时)。本函数向前回退到最近的字符边界,保证不会 panic。
fn truncate_for_log(s: &str, max_bytes: usize) -> &str {
    let mut end = max_bytes.min(s.len());
    while end > 0 && !s.is_char_boundary(end) {
        end -= 1;
    }
    &s[..end]
}

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
        if let Some(ref cookie) = self.session_cookie {
            let mut cookie_value = format!(
                "jms_sessionid={sid}; X-JMS-ORG={org}; X-JMS-LUNA-ORG={org}",
                sid = cookie,
                org = self.org_id
            );
            if let Some(ref csrf) = self.csrf_token {
                cookie_value.push_str(&format!("; jms_csrftoken={}", csrf));
            }
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
            let s = serde_json::to_string(body)
                .map_err(|e| JmsError::ParseError(e.to_string()))?;
            builder = builder.header("Content-Type", "application/json");
            s
        } else {
            String::new()
        };

        let request = builder
            .body(http_client::AsyncBody::from(body_str))
            .map_err(|e| JmsError::NetworkError(e.to_string()))?;

        tracing::debug!("发送 HTTP 请求: method={}, url={}", method, url);

        let response = self.client.send(request).await.map_err(|e| {
            tracing::error!("HTTP 请求失败: {}", e);
            JmsError::NetworkError(e.to_string())
        })?;

        // 提取 session cookie / csrf token——来自 Set-Cookie 响应头
        for (name, value) in response.headers() {
            if name.as_str().eq_ignore_ascii_case("set-cookie") {
                if let Ok(cookie_str) = value.to_str() {
                    tracing::debug!("收到 Set-Cookie: {}", cookie_str);
                    if let Some(sid) = Self::parse_session_id(cookie_str) {
                        tracing::info!("已提取 jms_sessionid: {}", truncate_for_log(&sid, 16));
                        self.session_cookie = Some(sid);
                    }
                    if let Some(csrf) = Self::parse_cookie_value(cookie_str, "jms_csrftoken") {
                        self.csrf_token = Some(csrf);
                    }
                }
            }
        }

        let status = response.status().as_u16();
        tracing::debug!("HTTP 响应状态码: {}", status);

        let mut body = response.into_body();
        let mut bytes = Vec::new();
        use futures::AsyncReadExt;
        body.read_to_end(&mut bytes)
            .await
            .map_err(|e| {
                tracing::error!("读取响应体失败: {}", e);
                JmsError::NetworkError(e.to_string())
            })?;
        let text = String::from_utf8(bytes)
            .map_err(|e| {
                tracing::error!("解析响应体失败: {}", e);
                JmsError::ParseError(e.to_string())
            })?;

        tracing::debug!("HTTP 响应内容: {}", truncate_for_log(&text, 500));

        Ok((status, text))
    }

    /// 从 Set-Cookie 字符串中解析 jms_sessionid
    fn parse_session_id(cookie_str: &str) -> Option<String> {
        Self::parse_cookie_value(cookie_str, "jms_sessionid")
    }

    /// 从 Set-Cookie 字符串中解析指定名称的 cookie 值
    fn parse_cookie_value(cookie_str: &str, name: &str) -> Option<String> {
        let prefix = format!("{}=", name);
        for part in cookie_str.split(';') {
            let part = part.trim();
            if let Some(value) = part.strip_prefix(&prefix) {
                if value.is_empty() {
                    return None;
                }
                return Some(value.to_string());
            }
        }
        None
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

    /// 设置组织 ID
    pub fn set_org_id(&mut self, org_id: String) {
        self.org_id = org_id;
    }

    /// 获取资产树
    pub async fn get_asset_tree(&mut self) -> Result<Vec<JmsAssetNode>, JmsError> {
        let url = format!(
            "{}/api/v1/perms/users/self/nodes/children-with-assets/tree/",
            self.base_url
        );

        tracing::info!("获取资产树: url={}", url);
        let (status, text) = self.get_json(&url).await?;
        tracing::info!("资产树响应: status={}, body={}", status, truncate_for_log(&text, 500));

        if status >= 200 && status < 300 {
            let nodes: Vec<JmsAssetNode> = serde_json::from_str(&text)
                .map_err(|e| {
                    tracing::error!("资产树 JSON 解析失败: {}", e);
                    JmsError::ParseError(e.to_string())
                })?;
            Ok(nodes)
        } else {
            Err(JmsError::ApiError(format!(
                "获取资产树失败: {}",
                text
            )))
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

        tracing::info!("懒加载子节点: key={}, url={}", node_key, url);
        let (status, text) = self.get_json(&url).await?;
        tracing::info!("子节点响应: status={}, body={}", status, truncate_for_log(&text, 500));

        if status >= 200 && status < 300 {
            let nodes: Vec<JmsAssetNode> = serde_json::from_str(&text).map_err(|e| {
                tracing::error!("子节点 JSON 解析失败: {}", e);
                JmsError::ParseError(e.to_string())
            })?;
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

        tracing::info!("搜索资产: keyword={}, url={}", keyword, url);
        let (status, text) = self.get_json(&url).await?;
        tracing::info!("搜索响应: status={}, body={}", status, truncate_for_log(&text, 300));

        if status >= 200 && status < 300 {
            let nodes: Vec<JmsAssetNode> = serde_json::from_str(&text).map_err(|e| {
                tracing::error!("搜索结果 JSON 解析失败: {}", e);
                JmsError::ParseError(e.to_string())
            })?;
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

    /// 获取授权规则（包含有权限的资产信息）
    pub async fn get_asset_permissions(&mut self) -> Result<Vec<JmsAssetPermission>, JmsError> {
        let url = format!("{}/api/v1/perms/asset-permissions/", self.base_url);

        tracing::info!("获取授权规则: url={}", url);
        let (status, text) = self.get_json(&url).await?;

        if status >= 200 && status < 300 {
            let permissions: Vec<JmsAssetPermission> = serde_json::from_str(&text)
                .map_err(|e| {
                    tracing::error!("授权规则 JSON 解析失败: {}", e);
                    JmsError::ParseError(e.to_string())
                })?;
            Ok(permissions)
        } else {
            Err(JmsError::ApiError(format!(
                "获取授权规则失败: {}",
                text
            )))
        }
    }

    /// 获取资产 SSH 连接信息
    pub async fn get_asset_connect_info(
        &mut self,
        asset_id: &str,
    ) -> Result<JmsConnectInfo, JmsError> {
        // JumpServer v4: 先尝试获取资产详情
        let detail_url = format!(
            "{}/api/v1/assets/assets/{}/",
            self.base_url, asset_id
        );

        tracing::info!("获取资产详情: asset_id={}", asset_id);
        let (status, text) = self.get_json(&detail_url).await?;
        tracing::info!("资产详情响应: status={}, body={}", status, truncate_for_log(&text, 500));

        if status >= 200 && status < 300 {
            let json: Value = serde_json::from_str(&text)
                .map_err(|e| JmsError::ParseError(e.to_string()))?;

            let host = json.get("ip")
                .or_else(|| json.get("address"))
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string();
            let port = json.get("port")
                .and_then(|v| v.as_u64())
                .unwrap_or(22) as u16;
            let _platform = json.get("platform")
                .and_then(|v| v.as_str())
                .unwrap_or("Linux")
                .to_string();

            return Ok(JmsConnectInfo {
                host,
                port,
                username: String::new(), // 账号通过 accounts API 获取
                password: String::new(),
                private_key: String::new(),
                protocol: "ssh".to_string(),
                token: String::new(),
                koko_host: String::new(),
            });
        }

        // 回退：尝试创建 Koko 连接 token
        tracing::info!("资产详情接口不可用，尝试创建连接 token");
        self.create_connect_token(asset_id, "").await.map(|token| JmsConnectInfo {
            token: token.token,
            protocol: token.protocol,
            ..Default::default()
        })
    }

    /// 获取资产关联的账号列表
    pub async fn list_asset_accounts(
        &mut self,
        asset_id: &str,
    ) -> Result<Vec<JmsAssetAccount>, JmsError> {
        // JumpServer v4 账号端点探测：尝试多个可能的路径
        let candidates = [
            format!("{}/api/v1/assets/accounts/?asset={}", self.base_url, asset_id),
            format!("{}/api/v1/accounts/accounts/?asset={}", self.base_url, asset_id),
            format!("{}/api/v1/accounts/assets/{}/accounts/", self.base_url, asset_id),
            format!("{}/api/v1/perms/users/self/assets/{}/accounts/", self.base_url, asset_id),
        ];

        for url in &candidates {
            tracing::info!("尝试获取账号列表: url={}", url);
            let (status, text) = self.get_json(url).await?;
            tracing::info!("账号列表响应: status={}, body={}", status, truncate_for_log(&text, 500));

            if status >= 200 && status < 300 {
                if let Ok(accounts) = serde_json::from_str::<Vec<JmsAssetAccount>>(&text) {
                    if !accounts.is_empty() {
                        return Ok(accounts);
                    }
                }
                // 如果返回空数组或解析失败，继续尝试下一个
            }
        }

        // 所有端点都失败
        Err(JmsError::ApiError("无法获取账号列表，所有端点均返回 404".to_string()))
    }

    /// 创建 Koko 连接 token（用于 WebSocket 终端）
    pub async fn create_connect_token(
        &mut self,
        asset_id: &str,
        account: &str,
    ) -> Result<JmsConnectToken, JmsError> {
        // JumpServer v4: POST /api/v1/authentication/connection-token/
        let url = format!(
            "{}/api/v1/authentication/connection-token/",
            self.base_url
        );

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

        tracing::info!("创建连接 token: asset_id={}, account={}", asset_id, account);
        let (status, text) = self.post_json(&url, &body).await?;
        tracing::info!("连接 token 响应: status={}, body={}", status, truncate_for_log(&text, 500));

        if status >= 200 && status < 300 {
            let token: JmsConnectToken = serde_json::from_str(&text).map_err(|e| {
                tracing::error!("连接 token JSON 解析失败: {}", e);
                JmsError::ParseError(e.to_string())
            })?;
            Ok(token)
        } else {
            Err(JmsError::ApiError(format!(
                "创建连接 token 失败: {}",
                text
            )))
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
            tracing::info!("获取 JMS Web 登录页: {}", login_url);
            let (status, text, cookies) = self.raw_get(&login_url).await?;
            tracing::info!(
                "登录页响应: status={}, body_len={}, cookies={:?}",
                status, text.len(), cookies.keys()
            );
            if status != 200 {
                return Err(JmsError::ApiError(format!("获取登录页失败: status={}", status)));
            }

            if let Some(sid) = cookies.get("jms_sessionid") {
                self.session_cookie = Some(sid.clone());
            }
            let csrf = cookies
                .get("jms_csrftoken")
                .cloned()
                .filter(|s| !s.is_empty())
                .unwrap_or_else(|| Self::extract_csrf_from_html(&text));
            if !csrf.is_empty() {
                self.csrf_token = Some(csrf);
            }
            if let Some(pk) = cookies.get("jms_public_key") {
                self.public_key = Some(pk.clone());
            }
        }

        let csrf_token = self.csrf_token.clone().unwrap_or_default();
        let public_key = self.public_key.clone().unwrap_or_default();
        let login_session_id = self.session_cookie.clone().unwrap_or_default();

        // 2. 加密密码(RSA+AES 混合,与浏览器 encryptPassword 一致)
        let encrypted_password = Self::encrypt_password_with_public_key(password, &public_key)
            .unwrap_or_else(|e| {
                tracing::warn!("密码加密失败,回退明文: {}", e);
                password.to_string()
            });

        // 3. 构造表单
        let mut form_map = serde_json::Map::new();
        form_map.insert("username".to_string(), Value::String(username.to_string()));
        form_map.insert("password".to_string(), Value::String(encrypted_password));
        form_map.insert("csrfmiddlewaretoken".to_string(), Value::String(csrf_token.clone()));
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

        tracing::info!("提交 JMS Web 登录表单");
        let (status, text, cookies) = self.raw_post_form(&login_url, &form, headers).await?;
        tracing::info!(
            "登录提交响应: status={}, body_len={}, cookies={:?}",
            status, text.len(), cookies.keys()
        );

        // 更新 session / csrf
        if let Some(sid) = cookies.get("jms_sessionid") {
            self.session_cookie = Some(sid.clone());
        }
        if let Some(csrf) = cookies.get("jms_csrftoken").filter(|s| !s.is_empty()) {
            self.csrf_token = Some(csrf.clone());
        }

        // 4. 判定结果
        let location = cookies.get("location").cloned().unwrap_or_default();
        tracing::info!("登录 POST 后 status={}, location={}", status, location);

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
                    tracing::info!("Web 登录成功(跳转→{})", location);
                    return Ok(WebLoginResult::Success { session_id: sid.clone() });
                }
            }
            let next_url = Self::absolute_url(&self.base_url, &location);
            let (status, body, cookies) = self.raw_get(&next_url).await?;
            tracing::info!("跟随登录跳转: url={}, status={}", next_url, status);
            if let Some(sid) = cookies.get("jms_sessionid") {
                self.session_cookie = Some(sid.clone());
            }
            if let Some(csrf) = cookies.get("jms_csrftoken").filter(|s| !s.is_empty()) {
                self.csrf_token = Some(csrf.clone());
            }

            // 继续重定向
            if (300..400).contains(&status) {
                if let Some(loc) = cookies.get("location") {
                    if loc.contains("/luna") || loc.contains("/ui") {
                        if let Some(ref sid) = self.session_cookie {
                            tracing::info!("Web 登录成功(跳转链→{})", loc);
                            return Ok(WebLoginResult::Success { session_id: sid.clone() });
                        }
                    }
                    location = loc.clone();
                    continue;
                }
            }

            // 200:已到实页面(MFA 页 / 登录页),按内容判定
            if next_url.contains("/luna") || next_url.contains("/ui") {
                if let Some(ref sid) = self.session_cookie {
                    return Ok(WebLoginResult::Success { session_id: sid.clone() });
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
        tracing::info!(
            "页面判定: 仍登录页={}, 需验证码={}, 需 MFA={}, 错误={:?}",
            still_login_page, needs_captcha, needs_mfa, err_msg
        );

        // 既不是登录页也不是 MFA 页 → 已认证成功
        if !still_login_page && !needs_mfa {
            if let Some(ref sid) = self.session_cookie {
                tracing::info!("Web 登录成功(页面判定)");
                return Ok(WebLoginResult::Success { session_id: sid.clone() });
            }
        }

        // 验证码优先:无论是否同时需要 MFA,先让用户过验证码
        if needs_captcha && err_msg.is_some() {
            // 仅当有错误(验证码错误)时才回退到验证码;首次成功提交不应再要验证码
            if let Some(info) = Self::parse_captcha_from_html(text, &self.base_url) {
                self.last_captcha_key = Some(info.key.clone());
                let info = self.fill_captcha_image(info).await;
                tracing::info!("Web 登录需要重新输入验证码, key={}", info.key);
                return Ok(WebLoginResult::CaptchaRequired(info));
            }
        }
        if needs_captcha && !needs_mfa {
            if let Some(info) = Self::parse_captcha_from_html(text, &self.base_url) {
                self.last_captcha_key = Some(info.key.clone());
                let info = self.fill_captcha_image(info).await;
                tracing::info!("Web 登录需要验证码, key={}", info.key);
                return Ok(WebLoginResult::CaptchaRequired(info));
            }
            tracing::warn!("检测到验证码但解析 key/图片失败");
        }
        if needs_mfa {
            // 已提供 OTP → 直接提交 MFA 表单
            if let Some(code) = otp_code.filter(|s| !s.is_empty()) {
                tracing::info!("提交 MFA 验证码");
                return Box::pin(self.submit_mfa(text, code)).await;
            }
            tracing::info!("Web 登录需要 MFA 验证码");
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
        let (status, page, cookies) = self.raw_get(&mfa_url).await?;
        tracing::info!("获取 MFA 页: status={}", status);
        if let Some(sid) = cookies.get("jms_sessionid") {
            self.session_cookie = Some(sid.clone());
        }
        if let Some(csrf) = cookies.get("jms_csrftoken").filter(|s| !s.is_empty()) {
            self.csrf_token = Some(csrf.clone());
        }
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
        form_map.insert("csrfmiddlewaretoken".to_string(), Value::String(csrf.clone()));
        form_map.insert("mfa_type".to_string(), Value::String("otp".to_string()));
        form_map.insert("code".to_string(), Value::String(code.to_string()));
        let form = Value::Object(form_map);

        let session_id = self.session_cookie.clone().unwrap_or_default();
        let headers = self.build_web_headers(&mfa_url, &session_id, &csrf);

        let (status, text, cookies) = self.raw_post_form(&mfa_url, &form, headers).await?;
        tracing::info!("MFA 提交响应: status={}, location={:?}", status, cookies.get("location"));
        if let Some(sid) = cookies.get("jms_sessionid") {
            self.session_cookie = Some(sid.clone());
        }
        if let Some(csrf) = cookies.get("jms_csrftoken").filter(|s| !s.is_empty()) {
            self.csrf_token = Some(csrf.clone());
        }

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
        let url = format!("{}/core/auth/captcha/refresh/", self.base_url);
        let (status, text, _) = self.raw_get(&url).await?;
        if status != 200 {
            return Err(JmsError::ApiError(format!("刷新验证码失败: status={}", status)));
        }
        let json: Value = serde_json::from_str(&text)
            .map_err(|e| JmsError::ParseError(format!("验证码响应解析失败: {e}")))?;
        let key = json.get("key").and_then(|v| v.as_str()).unwrap_or_default().to_string();
        let image_url_raw = json.get("image_url").and_then(|v| v.as_str()).unwrap_or_default();
        let image_url = Self::absolute_url(&self.base_url, image_url_raw);
        self.last_captcha_key = Some(key.clone());
        let info = CaptchaInfo { key, image_url, image_data: None, image_mime: None };
        Ok(self.fill_captcha_image(info).await)
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
            Err(e) => tracing::warn!("下载验证码图片失败: {}", e),
        }
        info
    }

    /// 从登录页 HTML 解析验证码 key 和图片地址
    fn parse_captcha_from_html(html: &str, base_url: &str) -> Option<CaptchaInfo> {
        // <input ... name="captcha_0" value="<key>" ...>
        let key_re = regex::Regex::new(
            r#"name=['"]captcha_0['"][^>]*value=['"]([^'"]+)['"]"#,
        ).ok()?;
        let key = key_re.captures(html).and_then(|c| c.get(1)).map(|m| m.as_str().to_string())?;
        // <img ... src="/core/auth/captcha/image/<hash>/" class="captcha" ...>
        let img_re = regex::Regex::new(r#"src=['"](/core/auth/captcha/image/[^'"]+)['"]"#).ok()?;
        let image_url = img_re
            .captures(html)
            .and_then(|c| c.get(1))
            .map(|m| Self::absolute_url(base_url, m.as_str()))
            .unwrap_or_default();
        Some(CaptchaInfo { key, image_url, image_data: None, image_mime: None })
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
        let cookie_header = format!(
            "jms_sessionid={sid}; X-JMS-ORG={org}; jms_csrftoken={csrf}; django_language=zh-hans",
            sid = session_id,
            org = self.org_id,
            csrf = csrf_token
        );
        headers.insert("Cookie".to_string(), cookie_header);
        headers.insert("Referer".to_string(), login_url.to_string());
        headers.insert("X-CSRFToken".to_string(), csrf_token.to_string());
        headers.insert("Accept".to_string(), "text/html,application/xhtml+xml,application/xml;q=0.9,*/*;q=0.8".to_string());
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
        let re = regex::Regex::new(r#"name=['\"]csrfmiddlewaretoken['\"]\s+value=['\"]([^'\"]+)['\"]"#)
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
            .header("Accept", "text/html,application/xhtml+xml,application/xml;q=0.9,*/*;q=0.8")
            .header("User-Agent", "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36");

        // 携带当前会话 cookie,否则 guard/MFA 等中间页会判定未认证并打回登录
        if let Some(ref sid) = self.session_cookie {
            let mut cookie = format!(
                "jms_sessionid={sid}; X-JMS-ORG={org}; django_language=zh-hans",
                sid = sid,
                org = self.org_id
            );
            if let Some(ref csrf) = self.csrf_token {
                cookie.push_str(&format!("; jms_csrftoken={}", csrf));
            }
            builder = builder.header("Cookie", cookie);
        }
        // 中间跳转页同样不自动跟随,便于逐跳读取 Set-Cookie
        builder = builder.extension(http_client::RedirectPolicy::NoFollow);

        let request = builder
            .body(http_client::AsyncBody::from(String::new()))
            .map_err(|e| JmsError::NetworkError(e.to_string()))?;

        let response = self.client.send(request).await.map_err(|e| {
            JmsError::NetworkError(format!("GET {} 失败: {}", url, e))
        })?;

        let status = response.status().as_u16();
        let mut cookies = Self::parse_all_cookies(response.headers());
        if let Some(loc) = response.headers().get("location").and_then(|v| v.to_str().ok()) {
            cookies.insert("location".to_string(), loc.to_string());
        }
        let body = response.into_body();
        let text = Self::read_body_to_string(body).await?;
        Ok((status, text, cookies))
    }

    /// 发送 GET 请求并返回原始字节 + Content-Type(用于下载验证码图片)
    async fn raw_get_bytes(
        &mut self,
        url: &str,
    ) -> Result<(Vec<u8>, Option<String>), JmsError> {
        let mut builder = http::Request::builder()
            .method("GET")
            .uri(url)
            .header("Accept", "image/avif,image/webp,image/apng,image/*,*/*;q=0.8")
            .header("User-Agent", "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36");
        if let Some(ref sid) = self.session_cookie {
            let cookie = format!(
                "jms_sessionid={sid}; X-JMS-ORG={org}",
                sid = sid,
                org = self.org_id
            );
            builder = builder.header("Cookie", cookie);
        }
        let request = builder
            .body(http_client::AsyncBody::from(String::new()))
            .map_err(|e| JmsError::NetworkError(e.to_string()))?;

        let response = self.client.send(request).await.map_err(|e| {
            JmsError::NetworkError(format!("GET {} 失败: {}", url, e))
        })?;
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
            .header("Accept", "text/html,application/xhtml+xml,application/xml;q=0.9,*/*;q=0.8")
            .header("Content-Type", "application/x-www-form-urlencoded")
            .header("User-Agent", "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36");

        for (k, v) in extra_headers {
            builder = builder.header(k, v);
        }

        // 不自动跟随 302:登录成功的「已认证 session」由 302 响应的 Set-Cookie 携带,
        // 若跟随重定向到 /luna/,我们只会读到最终响应,丢失已认证 cookie,导致后续 401。
        builder = builder.extension(http_client::RedirectPolicy::NoFollow);

        let request = builder
            .body(http_client::AsyncBody::from(body_str))
            .map_err(|e| JmsError::NetworkError(e.to_string()))?;

        let response = self.client.send(request).await.map_err(|e| {
            JmsError::NetworkError(format!("POST {} 失败: {}", url, e))
        })?;

        let status = response.status().as_u16();
        let mut cookies = Self::parse_all_cookies(response.headers());
        // 记录 Location 头,用于判定登录是否成功跳转
        if let Some(loc) = response.headers().get("location").and_then(|v| v.to_str().ok()) {
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
    fn parse_all_cookies(
        headers: &http::HeaderMap,
    ) -> std::collections::HashMap<String, String> {
        let mut cookies = std::collections::HashMap::new();
        for (name, value) in headers {
            if name.as_str().eq_ignore_ascii_case("set-cookie") {
                if let Ok(s) = value.to_str() {
                    if let Some((k, v)) = s.split_once('=') {
                        let v = v.split(';').next().unwrap_or("").to_string();
                        cookies.insert(k.trim().to_string(), v);
                    }
                }
            }
        }
        cookies
    }

    /// 从登录页 HTML 中提取错误提示文本
    fn extract_login_error(html: &str) -> Option<String> {
        let re = regex::Regex::new(r#"<div[^>]*class\s*=\s*['"]alert[^'"]*['"][^>]*>(.*?)</div>"#).ok()?;
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

        // cookie 值通常是 base64(PEM)
        let pem = if public_key_cookie_value.contains("BEGIN PUBLIC KEY")
            || public_key_cookie_value.contains("BEGIN RSA PUBLIC KEY")
        {
            public_key_cookie_value.to_string()
        } else {
            let decoded = base64::engine::general_purpose::STANDARD
                .decode(public_key_cookie_value)
                .map_err(|e| anyhow::anyhow!("base64 解码 jms_public_key 失败: {e}"))?;
            String::from_utf8(decoded)
                .map_err(|e| anyhow::anyhow!("jms_public_key 解码后不是 UTF-8: {e}"))?
        };

        let public_key = RsaPublicKey::from_public_key_pem(&pem)
            .map_err(|e| anyhow::anyhow!("解析 RSA 公钥失败: {e} (pem 前缀={})", truncate_for_log(&pem, 80)))?;

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
        use aes::cipher::{BlockEncrypt, KeyInit};
        use aes::Aes128;

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
