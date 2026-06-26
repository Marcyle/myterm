//! JMS 连接窗口

use gpui::prelude::FluentBuilder;
use gpui::{
    App, AppContext, AsyncApp, Context, Entity, EventEmitter, FocusHandle, Focusable,
    InteractiveElement, IntoElement, ParentElement, Render, StatefulInteractiveElement, Styled,
    Window, div, img, px,
};
use gpui_component::{
    ActiveTheme, Disableable, Icon, IconName, Sizable, Size,
    button::{Button, ButtonVariants as _},
    h_flex,
    input::{Input, InputState},
    scroll::ScrollableElement,
    v_flex,
};
use jms::JmsClient;
use std::collections::HashSet;

/// JMS 连接窗口事件
#[derive(Clone, Debug)]
pub enum JmsConnectionEvent {
    /// 连接成功
    Connected { token: String },
    /// 关闭窗口
    Closed,
}

/// JMS 连接窗口状态
#[derive(Clone, PartialEq, Eq)]
enum ConnectionState {
    /// 输入连接信息
    Input,
    /// 验证用户名密码
    Verifying,
    /// 输入图片验证码
    CaptchaInput,
    /// 输入 MFA 验证码
    MfaInput,
    /// 加载资产树
    LoadingAssetTree,
    /// 资产树浏览
    AssetTree,
    /// 选择连接账号
    SelectAccount,
}

/// JMS 连接窗口
pub struct JmsConnectionWindow {
    focus_handle: FocusHandle,
    state: ConnectionState,
    url_input: Entity<InputState>,
    username_input: Entity<InputState>,
    password_input: Entity<InputState>,
    mfa_input: Entity<InputState>,
    captcha_input: Entity<InputState>,
    captcha_info: Option<jms::CaptchaInfo>,
    /// 验证码图片落地的临时文件路径(供 gpui img() 渲染)
    captcha_image_path: Option<std::path::PathBuf>,
    error_message: Option<String>,
    client: Option<JmsClient>,
    use_local_proxy: bool,
    /// 若从已保存连接打开,记录其 id(保存时执行更新而非新增)
    saved_connection_id: Option<i64>,
    tree_roots: Vec<jms::JmsAssetTreeNode>,
    expanded_ids: HashSet<String>,
    parent: gpui::Entity<crate::home_tab::HomePage>,
    #[allow(dead_code)]
    parent_window: gpui::AnyWindowHandle,
    /// 本弹出窗口自身的 window handle(用于连接成功后关闭自己)
    own_window: gpui::AnyWindowHandle,
    /// 当前选中资产的可选账号列表
    pending_accounts: Vec<jms::JmsAssetAccount>,
    pending_asset_id: String,
    pending_asset_name: String,
}

impl JmsConnectionWindow {
    pub fn new(
        parent: gpui::Entity<crate::home_tab::HomePage>,
        parent_window: gpui::AnyWindowHandle,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        Self::new_with_prefill(parent, parent_window, None, window, cx)
    }

    /// 带预填充的构造(从已保存的 JMS 连接打开时使用)
    pub fn new_with_prefill(
        parent: gpui::Entity<crate::home_tab::HomePage>,
        parent_window: gpui::AnyWindowHandle,
        prefill: Option<(Option<i64>, one_core::storage::JmsParams)>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let focus_handle = cx.focus_handle();

        let (saved_id, prefill_params) = match prefill {
            Some((id, p)) => (id, Some(p)),
            None => (None, None),
        };
        let init_url = prefill_params.as_ref().map(|p| p.url.clone()).unwrap_or_default();
        let init_username = prefill_params.as_ref().map(|p| p.username.clone()).unwrap_or_default();
        let init_password = prefill_params.as_ref().map(|p| p.password.clone()).unwrap_or_default();
        let init_proxy = prefill_params.as_ref().map(|p| p.use_local_proxy).unwrap_or(true);

        let url_input = cx.new(|cx| {
            let mut s = InputState::new(window, cx)
                .placeholder("JMS 服务器地址 (如 https://jumpserver.example.com)")
                .clean_on_escape();
            if !init_url.is_empty() {
                s.set_value(&init_url, window, cx);
            }
            s
        });

        let username_input = cx.new(|cx| {
            let mut s = InputState::new(window, cx)
                .placeholder("用户名")
                .clean_on_escape();
            if !init_username.is_empty() {
                s.set_value(&init_username, window, cx);
            }
            s
        });

        let password_input = cx.new(|cx| {
            let mut s = InputState::new(window, cx)
                .placeholder("密码")
                .masked(true)
                .clean_on_escape();
            if !init_password.is_empty() {
                s.set_value(&init_password, window, cx);
            }
            s
        });

        let mfa_input = cx.new(|cx| {
            InputState::new(window, cx)
                .placeholder("MFA 验证码")
                .clean_on_escape()
        });

        let captcha_input = cx.new(|cx| {
            InputState::new(window, cx)
                .placeholder("图片验证码")
                .clean_on_escape()
        });

        Self {
            focus_handle,
            state: ConnectionState::Input,
            url_input,
            username_input,
            password_input,
            mfa_input,
            captcha_input,
            captcha_info: None,
            captcha_image_path: None,
            error_message: None,
            client: None,
            use_local_proxy: init_proxy,
            saved_connection_id: saved_id,
            tree_roots: Vec::new(),
            expanded_ids: HashSet::new(),
            parent,
            parent_window,
            own_window: window.window_handle(),
            pending_accounts: Vec::new(),
            pending_asset_id: String::new(),
            pending_asset_name: String::new(),
        }
    }

    /// 保存当前 JMS 连接信息(URL/用户名/密码)到本地连接列表
    fn handle_save_connection(&mut self, cx: &mut Context<Self>) {
        let url = self.url_input.read(cx).text().to_string();
        let username = self.username_input.read(cx).text().to_string();
        let password = self.password_input.read(cx).text().to_string();
        if url.is_empty() || username.is_empty() {
            self.error_message = Some("请先填写服务器地址和用户名再保存".to_string());
            cx.notify();
            return;
        }

        // 密码需加密存储:确保主密钥已就绪
        if !one_core::crypto::has_master_key() {
            self.error_message = Some("主密钥未解锁,无法安全保存密码".to_string());
            cx.notify();
            return;
        }

        let params = one_core::storage::JmsParams {
            url,
            username: username.clone(),
            password,
            use_local_proxy: self.use_local_proxy,
        };
        let name = format!("JMS - {}", username);
        let mut conn = one_core::storage::StoredConnection::new_jms(name, params, None);
        conn.id = self.saved_connection_id;

        let storage = cx
            .global::<one_core::storage::GlobalStorageState>()
            .storage
            .clone();
        let is_editing = conn.id.is_some();
        let parent = self.parent.clone();
        cx.spawn(async move |this, cx: &mut AsyncApp| {
            use one_core::storage::traits::Repository as _;
            let mut conn = conn;
            let result = smol::spawn(async move {
                let repo = storage
                    .get::<one_core::storage::ConnectionRepository>()
                    .ok_or_else(|| anyhow::anyhow!("ConnectionRepository not found"))?;
                if is_editing {
                    repo.update(&conn)?;
                } else {
                    repo.insert(&mut conn)?;
                }
                Ok::<_, anyhow::Error>(conn)
            })
            .await;

            match result {
                Ok(saved) => {
                    let _ = this.update(cx, |this, cx| {
                        this.saved_connection_id = saved.id;
                        this.error_message = Some("已保存连接信息".to_string());
                        cx.notify();
                    });
                    let _ = parent.update(cx, |home, cx| {
                        home.load_connections(cx);
                    });
                }
                Err(e) => {
                    let _ = this.update(cx, |this, cx| {
                        this.error_message = Some(format!("保存失败: {e}"));
                        cx.notify();
                    });
                }
            }
        })
        .detach();
    }

    fn handle_connect(&mut self, _window: &mut Window, cx: &mut Context<Self>) {
        let url = self.url_input.read(cx).text().to_string();
        let username = self.username_input.read(cx).text().to_string();
        let password = self.password_input.read(cx).text().to_string();
        let use_local_proxy = self.use_local_proxy;
        if url.is_empty() || username.is_empty() || password.is_empty() {
            self.error_message = Some("请填写所有必填字段".to_string());
            cx.notify();
            return;
        }

        self.state = ConnectionState::Verifying;
        self.error_message = None;
        cx.notify();

        // 获取项目中配置的 HTTP 客户端
        let http_client = cx.read_global(|settings: &one_core::settings::AppSettings, _cx| {
            let proxy = &settings.global_proxy;
            if use_local_proxy && proxy.enabled {
                let proxy_url = proxy.to_proxy_url().ok().flatten();
                reqwest_client::ReqwestClient::proxy_and_user_agent(proxy_url, "myterm")
                    .map(|c| std::sync::Arc::new(c) as std::sync::Arc<dyn gpui::http_client::HttpClient>)
                    .ok()
            } else {
                reqwest_client::ReqwestClient::user_agent("myterm")
                    .map(|c| std::sync::Arc::new(c) as std::sync::Arc<dyn gpui::http_client::HttpClient>)
                    .ok()
            }
        }).unwrap_or_else(|| {
            std::sync::Arc::new(reqwest_client::ReqwestClient::new()) as std::sync::Arc<dyn gpui::http_client::HttpClient>
        });

        cx.spawn(async move |this, cx: &mut AsyncApp| {
            let mut client = JmsClient::new(&url, http_client);

            // 纯 Web 表单登录,首次登录刷新登录页
            let (client, result) = smol::spawn(async move {
                let r = client
                    .web_login(&username, &password, None, None, true)
                    .await;
                (client, r)
            })
            .await;

            Self::handle_login_result(this, cx, client, result).await;
        })
        .detach();
    }

    /// 统一处理 web_login 返回结果(成功 → 资产树 / 验证码 / MFA / 失败)
    async fn handle_login_result(
        this: gpui::WeakEntity<Self>,
        cx: &mut AsyncApp,
        client: JmsClient,
        result: Result<jms::WebLoginResult, jms::JmsError>,
    ) {
        match result {
            Ok(jms::WebLoginResult::Success { session_id }) => {
                tracing::info!("JMS Web 登录成功, session 前缀={}", &session_id[..session_id.len().min(16)]);
                let _ = this.update(cx, |this, cx| {
                    this.client = Some(client);
                    this.error_message = None;
                    cx.notify();
                });
                Self::start_load_asset_tree(this, cx).await;
            }
            Ok(jms::WebLoginResult::CaptchaRequired(info)) => {
                let _ = this.update(cx, |this, cx| {
                    this.client = Some(client);
                    this.captcha_image_path = Self::persist_captcha_image(&info);
                    this.captcha_info = Some(info);
                    this.state = ConnectionState::CaptchaInput;
                    this.error_message = None;
                    cx.notify();
                });
            }
            Ok(jms::WebLoginResult::MfaRequired) => {
                let _ = this.update(cx, |this, cx| {
                    this.client = Some(client);
                    this.state = ConnectionState::MfaInput;
                    this.error_message = None;
                    cx.notify();
                });
            }
            Err(e) => {
                let _ = this.update(cx, |this, cx| {
                    this.client = Some(client);
                    // 回退:验证码/MFA 状态保持,否则回到输入
                    this.state = match this.state {
                        ConnectionState::CaptchaInput => ConnectionState::CaptchaInput,
                        ConnectionState::MfaInput => ConnectionState::MfaInput,
                        _ => ConnectionState::Input,
                    };
                    this.error_message = Some(e.to_string());
                    cx.notify();
                });
            }
        }
    }

    /// 提交图片验证码继续登录
    fn handle_captcha_submit(&mut self, _window: &mut Window, cx: &mut Context<Self>) {
        let captcha = self.captcha_input.read(cx).text().to_string();
        if captcha.is_empty() {
            self.error_message = Some("请输入验证码".to_string());
            cx.notify();
            return;
        }
        let mut client = match self.client.take() {
            Some(c) => c,
            None => return,
        };
        let username = self.username_input.read(cx).text().to_string();
        let password = self.password_input.read(cx).text().to_string();

        self.state = ConnectionState::Verifying;
        self.error_message = None;
        cx.notify();

        cx.spawn(async move |this, cx: &mut AsyncApp| {
            let (client, result) = smol::spawn(async move {
                let r = client
                    .web_login(&username, &password, Some(&captcha), None, false)
                    .await;
                (client, r)
            })
            .await;
            // 失败时回到验证码界面
            let _ = this.update(cx, |this, _cx| {
                this.state = ConnectionState::CaptchaInput;
            });
            Self::handle_login_result(this, cx, client, result).await;
        })
        .detach();
    }

    /// 刷新图片验证码
    fn handle_refresh_captcha(&mut self, cx: &mut Context<Self>) {
        let mut client = match self.client.take() {
            Some(c) => c,
            None => return,
        };
        cx.spawn(async move |this, cx: &mut AsyncApp| {
            let (client, result) = smol::spawn(async move {
                let r = client.refresh_captcha().await;
                (client, r)
            })
            .await;
            let _ = this.update(cx, |this, cx| {
                this.client = Some(client);
                match result {
                    Ok(info) => {
                        this.captcha_image_path = Self::persist_captcha_image(&info);
                        this.captcha_info = Some(info);
                    }
                    Err(e) => this.error_message = Some(format!("刷新验证码失败: {e}")),
                }
                cx.notify();
            });
        })
        .detach();
    }

    /// 把验证码图片字节写入临时文件,返回路径(供 gpui img() 渲染)
    fn persist_captcha_image(info: &jms::CaptchaInfo) -> Option<std::path::PathBuf> {
        let bytes = info.image_data.as_ref()?;
        let ext = match info.image_mime.as_deref() {
            Some(m) if m.contains("jpeg") || m.contains("jpg") => "jpg",
            Some(m) if m.contains("gif") => "gif",
            _ => "png",
        };
        let mut path = std::env::temp_dir();
        path.push(format!("myterm_jms_captcha_{}.{}", info.key, ext));
        match std::fs::write(&path, bytes) {
            Ok(_) => Some(path),
            Err(e) => {
                tracing::warn!("写入验证码图片失败: {}", e);
                None
            }
        }
    }

    fn handle_mfa_verify(&mut self, _window: &mut Window, cx: &mut Context<Self>) {
        let code = self.mfa_input.read(cx).text().to_string();
        let mut client = self.client.take().expect("没有 JMS 客户端，请重新连接");

        self.state = ConnectionState::Verifying;
        self.error_message = None;
        cx.notify();

        cx.spawn(async move |this, cx: &mut AsyncApp| {
            let (client, result) = smol::spawn(async move {
                let r = client.submit_mfa_code(&code).await;
                (client, r)
            })
            .await;
            let _ = this.update(cx, |this, _cx| {
                this.state = ConnectionState::MfaInput;
            });
            Self::handle_login_result(this, cx, client, result).await;
        })
        .detach();
    }

    /// 点击资产，获取账号列表并弹出账号选择
    fn handle_asset_click(&mut self, asset_id: String, cx: &mut Context<Self>) {
        tracing::info!("点击资产: asset_id={}", asset_id);
        let mut client = match self.client.take() {
            Some(c) => c,
            None => {
                self.error_message = Some("客户端未连接，请重新认证".to_string());
                cx.notify();
                return;
            }
        };
        let asset_name = self
            .tree_roots
            .iter()
            .flat_map(|r| Self::find_asset(r, &asset_id))
            .next()
            .unwrap_or_else(|| asset_id.clone());

        self.state = ConnectionState::Verifying;
        self.error_message = None;
        cx.notify();

        let asset_id2 = asset_id.clone();

        cx.spawn(async move |this, cx: &mut AsyncApp| {
            // 获取账号列表
            let (client, accounts_result) = smol::spawn(async move {
                let r = client.list_asset_accounts(&asset_id2).await;
                (client, r)
            })
            .await;

            let accounts = match accounts_result {
                Ok(accts) => accts,
                Err(e) => {
                    tracing::warn!("获取账号列表失败: {}", e);
                    let _ = this.update(cx, |this, cx| {
                        this.client = Some(client);
                        this.state = ConnectionState::AssetTree;
                        this.error_message = Some(format!("获取账号列表失败: {}", e));
                        cx.notify();
                    });
                    return;
                }
            };

            // 展示接口返回的全部账号（不按 active 过滤，避免字段缺失误删）
            let active_accounts: Vec<_> = accounts;

            let _ = this.update(cx, |this, cx| {
                this.client = Some(client);
                this.pending_accounts = active_accounts;
                this.pending_asset_id = asset_id;
                this.pending_asset_name = asset_name;
                this.state = ConnectionState::SelectAccount;
                cx.notify();
            });
        })
        .detach();
    }

    /// 用户选择了某个账号后连接
    fn handle_account_selected(&mut self, account_name: String, cx: &mut Context<Self>) {
        let mut client = match self.client.take() {
            Some(c) => c,
            None => return,
        };
        let asset_id = self.pending_asset_id.clone();
        let asset_name = self.pending_asset_name.clone();
        let parent = self.parent.clone();
        let own_window = self.own_window;

        // 同步读取代理配置(支持 SOCKS5 与 HTTP/HTTPS CONNECT)
        let koko_proxy = if self.use_local_proxy {
            cx.read_global(|settings: &one_core::settings::AppSettings, _cx| {
                let p = &settings.global_proxy;
                if !p.enabled {
                    return None;
                }
                let username = (!p.username.is_empty()).then(|| p.username.clone());
                let password = (!p.password.is_empty()).then(|| p.password.clone());
                match p.proxy_type {
                    one_core::settings::ProxyType::Socks5 => Some(jms::KokoProxy::Socks5 {
                        host: p.host.clone(),
                        port: p.port,
                        username,
                        password,
                    }),
                    one_core::settings::ProxyType::Http
                    | one_core::settings::ProxyType::Https => Some(jms::KokoProxy::Http {
                        host: p.host.clone(),
                        port: p.port,
                        username,
                        password,
                    }),
                }
            })
        } else {
            None
        };

        self.state = ConnectionState::Verifying;
        self.error_message = None;
        cx.notify();

        // 克隆已认证 client + 资产树,供新终端的资产树侧栏常驻使用
        let sidebar_client = client.clone();
        let sidebar_tree_roots = self.tree_roots.clone();
        let sidebar_proxy = koko_proxy.clone();

        cx.spawn(async move |this, cx: &mut AsyncApp| {
            // 登录已在进入资产树前完成,session 已认证,直接创建连接 token
            let connect_result = client.create_connect_token(&asset_id, &account_name).await;

            match connect_result {
                Ok(token) => {
                    // Koko WS 用连接 token 的 id（UUID）作为 ?token= 参数
                    let params = jms::KokoConnectParams {
                        base_url: client.base_url().to_string(),
                        token_id: token.id.clone(),
                        session_cookie: client.session_cookie().map(|s| s.to_string()),
                        org_id: client.org_id().to_string(),
                        proxy: koko_proxy,
                        title: format!("[JMS] {} ({})", asset_name, account_name),
                    };
                    tracing::info!(
                        "JMS Koko 连接参数就绪: token_id={}, base_url={}",
                        params.token_id,
                        params.base_url
                    );

                    let sidebar_ctx = terminal_view::JmsSidebarContext {
                        client: sidebar_client,
                        tree_roots: sidebar_tree_roots,
                        proxy: sidebar_proxy,
                    };

                    let _ = parent.update(cx, |home_page, _app| {
                        home_page
                            .pending_jms_koko
                            .push((params, Some(sidebar_ctx)));
                    });

                    // 关闭连接窗口:发事件 + 移除自身弹出窗口
                    let _ = this.update(cx, |_this, cx| {
                        cx.emit(JmsConnectionEvent::Closed);
                    });
                    let _ = cx.update_window(own_window, |_, window, _| {
                        window.remove_window();
                    });
                }
                Err(e) => {
                    tracing::warn!("创建连接 token 失败: {}", e);
                    let _ = this.update(cx, |this, cx| {
                        this.client = Some(client);
                        this.state = ConnectionState::SelectAccount;
                        this.error_message = Some(format!("连接失败: {}", e));
                        cx.notify();
                    });
                }
            }
        })
        .detach();
    }

    /// 在树中按 ID 查找资产名称
    fn find_asset<'a>(node: &'a jms::JmsAssetTreeNode, asset_id: &str) -> Option<String> {
        if node.node.id == asset_id {
            return Some(node.node.title.clone());
        }
        for child in &node.children {
            if let Some(name) = Self::find_asset(child, asset_id) {
                return Some(name);
            }
        }
        None
    }

    /// 认证成功后加载资产树
    async fn start_load_asset_tree(this: gpui::WeakEntity<Self>, cx: &mut AsyncApp) {
        let mut client = match this.update(cx, |this, _cx| {
            this.state = ConnectionState::LoadingAssetTree;
            this.client.take()
        }) {
            Ok(Some(c)) => c,
            Ok(None) => {
                tracing::error!("没有 JMS 客户端");
                return;
            }
            Err(_) => return,
        };

        // 在后台线程中获取资产树
        let (client, result) = smol::spawn(async move {
            let r = client.get_asset_tree().await;
            (client, r)
        })
        .await;

        match result {
            Ok(nodes) => {
                let tree = jms::build_asset_tree(nodes);
                let _ = this.update(cx, |this, cx| {
                    this.tree_roots = tree;
                    this.client = Some(client);
                    this.state = ConnectionState::AssetTree;
                    cx.notify();
                });
            }
            Err(e) => {
                let _ = this.update(cx, |this, cx| {
                    this.state = ConnectionState::Input;
                    this.client = Some(client);
                    this.error_message = Some(format!("加载资产树失败: {e}"));
                    cx.notify();
                });
            }
        }
    }

    /// 展开/折叠节点，必要时懒加载子节点
    fn handle_node_toggle(
        &mut self,
        node_id: String,
        load_key: String,
        loaded: bool,
        cx: &mut Context<Self>,
    ) {
        // 已展开 → 折叠
        if self.expanded_ids.contains(&node_id) {
            self.expanded_ids.remove(&node_id);
            cx.notify();
            return;
        }

        // 展开
        self.expanded_ids.insert(node_id.clone());

        // 子节点已加载，直接展开即可
        if loaded {
            cx.notify();
            return;
        }

        // 懒加载子节点
        let mut client = match self.client.take() {
            Some(c) => c,
            None => {
                self.error_message = Some("客户端未连接，请重新认证".to_string());
                cx.notify();
                return;
            }
        };
        cx.notify();

        let node_id_for_update = node_id.clone();
        cx.spawn(async move |this, cx: &mut AsyncApp| {
            let load_key2 = load_key.clone();
            let (client, result) = smol::spawn(async move {
                let r = client.get_node_children(&load_key2).await;
                (client, r)
            })
            .await;

            match result {
                Ok(nodes) => {
                    // 过滤掉返回结果中的自身节点，仅保留直接子节点
                    let children: Vec<jms::JmsAssetTreeNode> = nodes
                        .into_iter()
                        .filter(|n| n.id != node_id_for_update)
                        .map(|n| jms::JmsAssetTreeNode {
                            node: n,
                            children: Vec::new(),
                            loaded: false,
                        })
                        .collect();

                    let _ = this.update(cx, |this, cx| {
                        this.client = Some(client);
                        this.set_node_children(&node_id_for_update, children);
                        cx.notify();
                    });
                }
                Err(e) => {
                    tracing::warn!("懒加载子节点失败: {}", e);
                    let _ = this.update(cx, |this, cx| {
                        this.client = Some(client);
                        this.error_message = Some(format!("加载子节点失败: {}", e));
                        cx.notify();
                    });
                }
            }
        })
        .detach();
    }

    /// 在树中按 ID 查找节点并设置其子节点，标记为已加载
    fn set_node_children(&mut self, node_id: &str, children: Vec<jms::JmsAssetTreeNode>) {
        fn walk(
            nodes: &mut [jms::JmsAssetTreeNode],
            node_id: &str,
            children: &mut Option<Vec<jms::JmsAssetTreeNode>>,
        ) -> bool {
            for n in nodes.iter_mut() {
                if n.node.id == node_id {
                    n.children = children.take().unwrap_or_default();
                    n.loaded = true;
                    return true;
                }
                if walk(&mut n.children, node_id, children) {
                    return true;
                }
            }
            false
        }
        let mut opt = Some(children);
        walk(&mut self.tree_roots, node_id, &mut opt);
    }
}

impl Focusable for JmsConnectionWindow {
    fn focus_handle(&self, _cx: &App) -> FocusHandle {
        self.focus_handle.clone()
    }
}

impl EventEmitter<JmsConnectionEvent> for JmsConnectionWindow {}

impl Render for JmsConnectionWindow {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let is_verifying = self.state == ConnectionState::Verifying;

        div()
            .size_full()
            .track_focus(&self.focus_handle)
            .p_6()
            .child(
                v_flex()
                    .gap_4()
                    .overflow_hidden()
                    .child(
                        // 标题
                        h_flex()
                            .items_center()
                            .gap_3()
                            .child(
                                Icon::new(IconName::Server)
                                    .with_size(Size::Large)
                                    .text_color(cx.theme().accent),
                            )
                            .child(
                                div()
                                    .text_xl()
                                    .font_weight(gpui::FontWeight::SEMIBOLD)
                                    .text_color(cx.theme().foreground)
                                    .child("JMS 连接"),
                            ),
                    )
                    // 错误信息
                    .when_some(self.error_message.clone(), |this, msg| {
                        this.child(
                            div()
                                .p_3()
                                .rounded(px(8.0))
                                .bg(cx.theme().danger.opacity(0.1))
                                .border_1()
                                .border_color(cx.theme().danger)
                                .child(
                                    div()
                                        .text_sm()
                                        .text_color(cx.theme().danger)
                                        .child(msg),
                                ),
                        )
                    })
                    // 根据状态显示不同内容
                    .child(match self.state {
                        ConnectionState::Input | ConnectionState::Verifying => {
                            self.render_input_form(is_verifying, cx)
                        }
                        ConnectionState::CaptchaInput => self.render_captcha_form(is_verifying, cx),
                        ConnectionState::MfaInput => self.render_mfa_form(is_verifying, cx),
                        ConnectionState::LoadingAssetTree => {
                            self.render_loading_tree(cx)
                        }
                        ConnectionState::AssetTree => {
                            self.render_asset_tree(cx)
                        }
                        ConnectionState::SelectAccount => {
                            self.render_account_selection(cx)
                        }
                    })
            )
    }
}

// ===== 账号选择 + 树渲染辅助方法 =====
impl JmsConnectionWindow {
    fn render_account_selection(&self, cx: &mut Context<Self>) -> gpui::AnyElement {
        v_flex()
            .gap_4()
            .child(
                div()
                    .text_sm()
                    .font_weight(gpui::FontWeight::MEDIUM)
                    .child(format!("选择连接账号 - {}", self.pending_asset_name)),
            )
            .child(
                div()
                    .text_xs()
                    .text_color(cx.theme().muted_foreground)
                    .child("请选择用于连接此资产的账号："),
            )
            .child(
                v_flex()
                    .gap_2()
                    .children(self.pending_accounts.iter().map(|account| {
                        let name = account.name.clone();
                        let username = account.username.clone();
                        let name_for_click = name.clone();
                        let is_privileged = account.privileged;
                        div()
                            .id(gpui::SharedString::from(format!("account-{}", account.id)))
                            .w_full()
                            .p_3()
                            .rounded(px(8.0))
                            .bg(cx.theme().secondary)
                            .border_1()
                            .border_color(cx.theme().border)
                            .cursor_pointer()
                            .hover(|style| style.bg(cx.theme().list_hover))
                            .on_click(cx.listener(move |this, _, _window, cx| {
                                this.handle_account_selected(name_for_click.clone(), cx);
                            }))
                            .child(
                                h_flex()
                                    .items_center()
                                    .gap_3()
                                    .child(
                                        h_flex()
                                            .gap_2()
                                            .child(
                                                div()
                                                    .text_sm()
                                                    .font_weight(gpui::FontWeight::SEMIBOLD)
                                                    .child(username),
                                            )
                                            .child(
                                                div()
                                                    .text_sm()
                                                    .text_color(cx.theme().muted_foreground)
                                                    .child(name),
                                            ),
                                    )
                                    .when(is_privileged, |this| {
                                        this.child(
                                            div()
                                                .text_xs()
                                                .px_2()
                                                .py_0p5()
                                                .rounded(px(4.0))
                                                .bg(cx.theme().accent.opacity(0.2))
                                                .text_color(cx.theme().accent)
                                                .child("特权"),
                                        )
                                    }),
                            )
                            .into_any_element()
                    })),
            )
            .child(
                h_flex()
                    .justify_end()
                    .child(
                        gpui_component::button::Button::new("back-to-tree")
                            .outline()
                            .label("返回")
                            .on_click(cx.listener(|this, _, _window, cx| {
                                this.state = ConnectionState::AssetTree;
                                this.error_message = None;
                                cx.notify();
                            })),
                    ),
            )
            .into_any_element()
    }

    fn render_loading_tree(&self, cx: &mut Context<Self>) -> gpui::AnyElement {
        h_flex()
            .flex_1()
            .items_center()
            .justify_center()
            .child(
                v_flex()
                    .gap_3()
                    .items_center()
                    .child(Icon::new(IconName::Loader).with_size(Size::Large))
                    .child(
                        div()
                            .text_sm()
                            .text_color(cx.theme().muted_foreground)
                            .child("正在加载资产树..."),
                    ),
            )
            .into_any_element()
    }

    fn render_asset_tree(&mut self, cx: &mut Context<Self>) -> gpui::AnyElement {
        if self.tree_roots.is_empty() {
            tracing::info!("资产树为空，无根节点");
            return v_flex()
                .flex_1()
                .items_center()
                .justify_center()
                .child(
                    div()
                        .text_sm()
                        .text_color(cx.theme().muted_foreground)
                        .child("没有可用的资产"),
                )
                .into_any_element();
        }

        tracing::info!(
            "渲染资产树: {} 个根节点, {} 个已展开节点",
            self.tree_roots.len(),
            self.expanded_ids.len()
        );
        let rows = self.collect_visible_rows(cx);

        v_flex()
            .flex_1()
            .overflow_y_scrollbar()
            .child(div().w_full().children(rows))
            .into_any_element()
    }

    /// 收集所有可见的行（扁平化树结构）
    fn collect_visible_rows(&mut self, cx: &mut Context<Self>) -> Vec<gpui::AnyElement> {
        let mut rows = Vec::new();
        self.collect_rows(&self.tree_roots.clone(), 0, &mut rows, cx);
        rows
    }

    /// 递归收集树行
    fn collect_rows(
        &mut self,
        nodes: &[jms::JmsAssetTreeNode],
        depth: usize,
        rows: &mut Vec<gpui::AnyElement>,
        cx: &mut Context<Self>,
    ) {
        for node in nodes {
            let is_asset = node.node.meta.as_ref()
                .map(|m| m.node_type == "asset")
                .unwrap_or(false);
            let has_children = node.node.is_parent;
            let is_expanded = self.expanded_ids.contains(&node.node.id);

            rows.push(self.render_tree_row(node, depth, is_asset, has_children, is_expanded, cx));

            if is_expanded {
                self.collect_rows(&node.children, depth + 1, rows, cx);
            }
        }
    }

    /// 渲染单个树行
    fn render_tree_row(
        &mut self,
        node: &jms::JmsAssetTreeNode,
        depth: usize,
        _is_asset: bool,
        has_children: bool,
        is_expanded: bool,
        cx: &mut Context<Self>,
    ) -> gpui::AnyElement {
        let node_id = node.node.id.clone();
        let node_id_for_toggle = node_id.clone();
        let load_key = node.load_key();
        let loaded = node.loaded;
        // JMS 树节点 ID 与真实资产 UUID 不同：目录节点用 "1:4:8" 路径格式，
        // 资产叶子节点的真实 UUID 在顶层 id（meta.data.id 为空时回退到顶层 id）
        let real_asset_id = node.node.meta.as_ref()
            .map(|m| m.data.id.clone())
            .filter(|s| !s.is_empty())
            .unwrap_or_else(|| node_id.clone());
        let node_id_for_connect = real_asset_id;
        let indent = px(depth as f32 * 20.0);

        h_flex()
            .id(gpui::SharedString::from(format!("jms-node-{}", node_id)))
            .pl(indent)
            .py_1()
            .px_2()
            .gap_1()
            .items_center()
            .cursor_pointer()
            .rounded(px(4.0))
            .hover(|style| style.bg(cx.theme().list_hover))
            .child(
                // 展开/折叠箭头
                if has_children {
                    Icon::new(if is_expanded {
                        IconName::ChevronDown
                    } else {
                        IconName::ChevronRight
                    })
                    .with_size(Size::Small)
                    .text_color(cx.theme().muted_foreground)
                    .into_any_element()
                } else {
                    div().w(px(16.0)).into_any_element()
                },
            )
            .child(
                // 节点图标
                if _is_asset {
                    Icon::new(IconName::SquareTerminal)
                        .with_size(Size::Small)
                        .text_color(cx.theme().accent)
                        .into_any_element()
                } else {
                    Icon::new(IconName::Folder)
                        .with_size(Size::Small)
                        .text_color(cx.theme().foreground)
                        .into_any_element()
                },
            )
            .child(
                // 节点标题
                div()
                    .text_sm()
                    .text_color(cx.theme().foreground)
                    .child(node.node.title.clone()),
            )
            .when(has_children, |this| {
                this.on_click(cx.listener(move |this, _, _window, cx| {
                    tracing::info!(
                        "展开/折叠: id={}, was_expanded={}, loaded={}",
                        node_id_for_toggle, is_expanded, loaded
                    );
                    this.handle_node_toggle(
                        node_id_for_toggle.clone(),
                        load_key.clone(),
                        loaded,
                        cx,
                    );
                }))
            })
            .when(!has_children, |this| {
                this.on_click(cx.listener(move |this, _, _window, cx| {
                    this.handle_asset_click(node_id_for_connect.clone(), cx);
                }))
            })
            .into_any_element()
    }
}

// ===== 表单渲染 =====
impl JmsConnectionWindow {
    fn render_input_form(&self, is_verifying: bool, cx: &mut Context<Self>) -> gpui::AnyElement {
        let use_local_proxy = self.use_local_proxy;

        v_flex()
            .gap_4()
            .child(
                // 服务器地址
                v_flex()
                    .gap_2()
                    .child(
                        div()
                            .text_sm()
                            .font_weight(gpui::FontWeight::MEDIUM)
                            .text_color(cx.theme().foreground)
                            .child("服务器地址"),
                    )
                    .child(Input::new(&self.url_input).w_full()),
            )
            .child(
                // 用户名
                v_flex()
                    .gap_2()
                    .child(
                        div()
                            .text_sm()
                            .font_weight(gpui::FontWeight::MEDIUM)
                            .text_color(cx.theme().foreground)
                            .child("用户名"),
                    )
                    .child(Input::new(&self.username_input).w_full()),
            )
            .child(
                // 密码
                v_flex()
                    .gap_2()
                    .child(
                        div()
                            .text_sm()
                            .font_weight(gpui::FontWeight::MEDIUM)
                            .text_color(cx.theme().foreground)
                            .child("密码"),
                    )
                    .child(Input::new(&self.password_input).w_full()),
            )
            .child(
                // 使用本地代理
                h_flex()
                    .items_center()
                    .gap_2()
                    .child(
                        gpui_component::checkbox::Checkbox::new("use-local-proxy")
                            .checked(use_local_proxy)
                            .on_click(cx.listener(|this, _, _window, cx| {
                                this.use_local_proxy = !this.use_local_proxy;
                                cx.notify();
                            })),
                    )
                    .child(
                        div()
                            .text_sm()
                            .text_color(cx.theme().foreground)
                            .child("使用本地代理"),
                    ),
            )
            .child(
                // 保存 + 连接按钮
                h_flex()
                    .justify_end()
                    .gap_2()
                    .child(
                        Button::new("save-jms-button")
                            .outline()
                            .label("保存连接")
                            .disabled(is_verifying)
                            .on_click(cx.listener(|this, _, _window, cx| {
                                this.handle_save_connection(cx);
                            })),
                    )
                    .child(
                        Button::new("connect-button")
                            .primary()
                            .label("连接")
                            .disabled(is_verifying)
                            .on_click(cx.listener(|this, _, window, cx| {
                                this.handle_connect(window, cx);
                            })),
                    ),
            )
            .into_any_element()
    }

    fn render_mfa_form(&self, is_verifying: bool, cx: &mut Context<Self>) -> gpui::AnyElement {
        v_flex()
            .gap_4()
            .child(
                // 提示信息
                div()
                    .p_3()
                    .rounded(px(8.0))
                    .bg(cx.theme().accent.opacity(0.1))
                    .border_1()
                    .border_color(cx.theme().accent)
                    .child(
                        div()
                            .text_sm()
                            .text_color(cx.theme().accent)
                            .child("请输入 MFA 验证码"),
                    ),
            )
            .child(
                // MFA 验证码
                v_flex()
                    .gap_2()
                    .child(
                        div()
                            .text_sm()
                            .font_weight(gpui::FontWeight::MEDIUM)
                            .text_color(cx.theme().foreground)
                            .child("验证码"),
                    )
                    .child(Input::new(&self.mfa_input).w_full()),
            )
            .child(
                // 验证按钮
                h_flex()
                    .justify_end()
                    .child(
                        Button::new("verify-button")
                            .primary()
                            .label("验证")
                            .disabled(is_verifying)
                            .on_click(cx.listener(|this, _, window, cx| {
                                this.handle_mfa_verify(window, cx);
                            })),
                    ),
            )
            .into_any_element()
    }

    fn render_captcha_form(&self, is_verifying: bool, cx: &mut Context<Self>) -> gpui::AnyElement {
        v_flex()
            .gap_4()
            .child(
                div()
                    .p_3()
                    .rounded(px(8.0))
                    .bg(cx.theme().accent.opacity(0.1))
                    .border_1()
                    .border_color(cx.theme().accent)
                    .child(
                        div()
                            .text_sm()
                            .text_color(cx.theme().accent)
                            .child("请输入图片验证码"),
                    ),
            )
            .child(
                // 验证码图片 + 刷新
                h_flex()
                    .gap_3()
                    .items_center()
                    .when_some(self.captcha_image_path.clone(), |this, path| {
                        this.child(img(path).h(px(40.0)).rounded(px(4.0)))
                    })
                    .child(
                        Button::new("refresh-captcha")
                            .ghost()
                            .icon(IconName::Refresh)
                            .tooltip("刷新验证码")
                            .on_click(cx.listener(|this, _, _window, cx| {
                                this.handle_refresh_captcha(cx);
                            })),
                    ),
            )
            .child(
                v_flex()
                    .gap_2()
                    .child(
                        div()
                            .text_sm()
                            .font_weight(gpui::FontWeight::MEDIUM)
                            .text_color(cx.theme().foreground)
                            .child("验证码"),
                    )
                    .child(Input::new(&self.captcha_input).w_full()),
            )
            .child(
                h_flex()
                    .justify_end()
                    .child(
                        Button::new("captcha-submit-button")
                            .primary()
                            .label("提交")
                            .disabled(is_verifying)
                            .on_click(cx.listener(|this, _, window, cx| {
                                this.handle_captcha_submit(window, cx);
                            })),
                    ),
            )
            .into_any_element()
    }
}
