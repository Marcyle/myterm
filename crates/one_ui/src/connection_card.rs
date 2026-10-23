use gpui::prelude::FluentBuilder;
use gpui::{
    App, ClickEvent, ElementId, FontWeight, Hsla, InteractiveElement, IntoElement, ParentElement,
    RenderOnce, SharedString, Stateful, StatefulInteractiveElement, Styled, Window, div, px,
};
use gpui_component::{
    ActiveTheme, Icon, IconName, InteractiveElementExt, Sizable,
    button::{Button, ButtonVariants},
    checkbox::Checkbox,
    h_flex,
    menu::DropdownMenu,
    v_flex,
};
use std::rc::Rc;

const CARD_GROUP: &str = "connection-card";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum CardDensity {
    #[default]
    Grid,
    List,
}

/// 通用连接紧凑卡片，用于首页等场景。
#[derive(IntoElement)]
pub struct ConnectionCard {
    id: ElementId,
    #[allow(dead_code)]
    icon: Option<Icon>,
    icon_bg: Option<Hsla>,
    title: SharedString,
    subtitle: Option<SharedString>,
    team_badge: Option<SharedString>,
    protocol: Option<SharedString>,
    endpoint: Option<SharedString>,
    env: Option<SharedString>,
    tags: Vec<SharedString>,
    last_used: Option<SharedString>,
    selected: bool,
    active: bool,
    is_favorite: bool,
    checked: bool,
    show_checkbox: bool,
    density: CardDensity,
    actions: Vec<Button>,
    menu_id: ElementId,
    menu_builder: Option<
        Rc<
            dyn Fn(
                gpui_component::menu::PopupMenu,
                &mut Window,
                &mut gpui::Context<gpui_component::menu::PopupMenu>,
            ) -> gpui_component::menu::PopupMenu,
        >,
    >,
    on_click: Option<Rc<dyn Fn(&ClickEvent, &mut Window, &mut App)>>,
    on_double_click: Option<Rc<dyn Fn(&ClickEvent, &mut Window, &mut App)>>,
    on_check: Option<Rc<dyn Fn(bool, &mut Window, &mut App)>>,
    on_connect: Option<Rc<dyn Fn(&mut Window, &mut App)>>,
    on_toggle_favorite: Option<Rc<dyn Fn(&mut Window, &mut App)>>,
}

impl ConnectionCard {
    pub fn new(
        id: impl Into<ElementId>,
        icon: impl Into<Icon>,
        title: impl Into<SharedString>,
    ) -> Self {
        Self {
            id: id.into(),
            icon: Some(icon.into()),
            icon_bg: None,
            title: title.into(),
            subtitle: None,
            team_badge: None,
            protocol: None,
            endpoint: None,
            env: None,
            tags: Vec::new(),
            last_used: None,
            selected: false,
            active: false,
            is_favorite: false,
            checked: false,
            show_checkbox: false,
            density: CardDensity::Grid,
            actions: Vec::new(),
            menu_id: "connection-card-more".into(),
            menu_builder: None,
            on_click: None,
            on_double_click: None,
            on_check: None,
            on_connect: None,
            on_toggle_favorite: None,
        }
    }

    pub fn icon_bg(mut self, bg: impl Into<Hsla>) -> Self {
        self.icon_bg = Some(bg.into());
        self
    }

    pub fn subtitle(mut self, subtitle: impl Into<SharedString>) -> Self {
        self.subtitle = Some(subtitle.into());
        self
    }

    pub fn team_badge(mut self, badge: impl Into<SharedString>) -> Self {
        self.team_badge = Some(badge.into());
        self
    }

    pub fn protocol(mut self, protocol: impl Into<SharedString>) -> Self {
        self.protocol = Some(protocol.into());
        self
    }

    pub fn endpoint(mut self, endpoint: impl Into<SharedString>) -> Self {
        self.endpoint = Some(endpoint.into());
        self
    }

    pub fn env(mut self, env: impl Into<SharedString>) -> Self {
        self.env = Some(env.into());
        self
    }

    pub fn tag(mut self, tag: impl Into<SharedString>) -> Self {
        self.tags.push(tag.into());
        self
    }

    pub fn tags(mut self, tags: impl IntoIterator<Item = impl Into<SharedString>>) -> Self {
        self.tags.extend(tags.into_iter().map(|t| t.into()));
        self
    }

    pub fn last_used(mut self, last_used: impl Into<SharedString>) -> Self {
        self.last_used = Some(last_used.into());
        self
    }

    pub fn selected(mut self, selected: bool) -> Self {
        self.selected = selected;
        self
    }

    pub fn active(mut self, active: bool) -> Self {
        self.active = active;
        self
    }

    pub fn favorite(mut self, is_favorite: bool) -> Self {
        self.is_favorite = is_favorite;
        self
    }

    pub fn checked(mut self, checked: bool) -> Self {
        self.checked = checked;
        self
    }

    pub fn show_checkbox(mut self, show: bool) -> Self {
        self.show_checkbox = show;
        self
    }

    pub fn density(mut self, density: CardDensity) -> Self {
        self.density = density;
        self
    }

    pub fn action(mut self, action: Button) -> Self {
        self.actions.push(action);
        self
    }

    pub fn menu_id(mut self, id: impl Into<ElementId>) -> Self {
        self.menu_id = id.into();
        self
    }

    pub fn menu(
        mut self,
        builder: impl Fn(
            gpui_component::menu::PopupMenu,
            &mut Window,
            &mut gpui::Context<gpui_component::menu::PopupMenu>,
        ) -> gpui_component::menu::PopupMenu
        + 'static,
    ) -> Self {
        self.menu_builder = Some(Rc::new(builder));
        self
    }

    pub fn on_click(
        mut self,
        handler: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static,
    ) -> Self {
        self.on_click = Some(Rc::new(handler));
        self
    }

    pub fn on_double_click(
        mut self,
        handler: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static,
    ) -> Self {
        self.on_double_click = Some(Rc::new(handler));
        self
    }

    pub fn on_check(mut self, handler: impl Fn(bool, &mut Window, &mut App) + 'static) -> Self {
        self.on_check = Some(Rc::new(handler));
        self
    }

    pub fn on_connect(mut self, handler: impl Fn(&mut Window, &mut App) + 'static) -> Self {
        self.on_connect = Some(Rc::new(handler));
        self
    }

    pub fn on_toggle_favorite(mut self, handler: impl Fn(&mut Window, &mut App) + 'static) -> Self {
        self.on_toggle_favorite = Some(Rc::new(handler));
        self
    }
}

impl RenderOnce for ConnectionCard {
    fn render(self, _window: &mut Window, cx: &mut App) -> impl IntoElement {
        let is_list = self.density == CardDensity::List;

        if is_list {
            self.render_list_card(cx)
        } else {
            self.render_grid_card(cx)
        }
    }
}

impl ConnectionCard {
    fn render_grid_card(self, cx: &App) -> Stateful<gpui::Div> {
        let is_checked = self.checked;
        let show_chk = self.show_checkbox || is_checked;
        let on_check = self.on_check;
        let on_connect = self.on_connect;
        let on_toggle_favorite = self.on_toggle_favorite;
        let is_fav = self.is_favorite;

        let mut card = v_flex()
            .id(self.id.clone())
            .w_full()
            .h(px(92.0))
            .rounded(px(10.0))
            .bg(cx.theme().surface)
            .px_3()
            .py_2()
            .border_1()
            .relative()
            .overflow_hidden()
            .justify_between()
            .group(CARD_GROUP)
            .when(self.selected, |this| {
                this.border_color(cx.theme().accent).border_l_3()
            })
            .when(!self.selected, |this| this.border_color(cx.theme().border))
            .hover(|style| style.border_color(cx.theme().accent))
            .when_some(self.on_click, |this, on_click| {
                this.cursor_pointer().on_click(move |ev, window, cx| {
                    on_click(ev, window, cx);
                })
            })
            .when_some(self.on_double_click, |this, on_double_click| {
                this.on_double_click(move |ev, window, cx| {
                    on_double_click(ev, window, cx);
                })
            });

        // Row 1: Checkbox + Online Dot + Title + Fav + Menu
        let mut row1 = h_flex().items_center().gap_1p5().w_full();

        let chk_id = ElementId::NamedInteger("chk".into(), 1);
        let check_cb = on_check.clone();
        row1 = row1.child(
            div()
                .flex_shrink_0()
                .when(!show_chk, |this| {
                    this.invisible().group_hover(CARD_GROUP, |s| s.visible())
                })
                .child(Checkbox::new(chk_id).checked(is_checked).on_click(
                    move |checked, window, cx| {
                        cx.stop_propagation();
                        if let Some(cb) = &check_cb {
                            cb(*checked, window, cx);
                        }
                    },
                )),
        );

        // Status dot (在线带微光轮廓，离线灰色)
        let dot = div()
            .w(px(8.0))
            .h(px(8.0))
            .rounded_full()
            .flex_shrink_0()
            .when(self.active, |this| {
                this.bg(cx.theme().success)
                    .border_2()
                    .border_color(cx.theme().success.opacity(0.4))
                    .shadow_sm()
            })
            .when(!self.active, |this| this.bg(cx.theme().muted));
        row1 = row1.child(dot);

        // Name
        let name_el = div()
            .text_sm()
            .font_weight(FontWeight::SEMIBOLD)
            .text_color(cx.theme().foreground)
            .overflow_hidden()
            .text_ellipsis()
            .whitespace_nowrap()
            .flex_1()
            .min_w_0()
            .child(self.title.clone());
        row1 = row1.child(name_el);

        // Fav star（常显：已收藏 ★ / 未收藏 ☆，点击切换，保证可发现性）
        if on_toggle_favorite.is_some() {
            let fav_cb = on_toggle_favorite.clone();
            row1 = row1.child(
                div()
                    .id(ElementId::NamedInteger("fav-star".into(), 1))
                    .text_sm()
                    .flex_shrink_0()
                    .cursor_pointer()
                    .when(is_fav, |d| d.text_color(cx.theme().warning))
                    .when(!is_fav, |d| {
                        d.text_color(cx.theme().muted_foreground.opacity(0.55))
                            .hover(|s| s.text_color(cx.theme().warning))
                    })
                    .on_click(move |_, window, cx| {
                        cx.stop_propagation();
                        if let Some(cb) = &fav_cb {
                            cb(window, cx);
                        }
                    })
                    .child(if is_fav { "★" } else { "☆" }),
            );
        }

        // Team badge (if any)
        if let Some(team_badge) = self.team_badge {
            row1 = row1.child(
                div()
                    .flex_shrink_0()
                    .px_1()
                    .rounded(px(3.0))
                    .bg(cx.theme().accent.opacity(0.15))
                    .text_color(cx.theme().accent)
                    .text_xs()
                    .child(team_badge),
            );
        }

        // Extra action buttons (e.g. stop port forwarding)
        if !self.actions.is_empty() {
            row1 = row1.children(self.actions);
        }

        // More menu ⋯
        if let Some(menu_builder) = self.menu_builder {
            row1 = row1.child(
                div().ml_auto().flex_shrink_0().child(
                    Button::new(self.menu_id)
                        .icon(IconName::Ellipsis)
                        .xsmall()
                        .ghost()
                        .tooltip("更多操作")
                        .dropdown_menu(move |menu, window, cx| menu_builder(menu, window, cx)),
                ),
            );
        }

        // Row 2: Protocol Badge + user@host:port (monospace)
        let mut row2 = h_flex().items_center().gap_1p5().w_full().min_w_0();

        if let Some(proto) = self.protocol {
            let proto_color = match proto.to_uppercase().as_str() {
                "SSH" | "SFTP" => cx.theme().connection_ssh,
                "JMS" => cx.theme().connection_jms,
                "PORTFORWARDING" => cx.theme().connection_port_forwarding,
                _ => cx.theme().muted_foreground,
            };
            row2 = row2.child(
                div()
                    .px_1p5()
                    .py_0p5()
                    .rounded(px(4.0))
                    .border_1()
                    .border_color(proto_color.opacity(0.35))
                    .bg(proto_color.opacity(0.08))
                    .text_xs()
                    .text_color(proto_color)
                    .flex_shrink_0()
                    .child(proto),
            );
        }

        let endpoint_text = self
            .endpoint
            .unwrap_or_else(|| self.subtitle.unwrap_or_default());
        row2 = row2.child(
            div()
                .font_family("Consolas")
                .text_xs()
                .text_color(cx.theme().muted_foreground)
                .overflow_hidden()
                .text_ellipsis()
                .whitespace_nowrap()
                .flex_1()
                .min_w_0()
                .child(endpoint_text),
        );

        // Row 3: Env Tag + Tags + Last Used
        let mut row3 = h_flex().items_center().gap_1p5().w_full().min_w_0();

        if let Some(env_str) = self.env {
            let (border_c, text_c) = match env_str.to_lowercase().as_str() {
                "dev" => (cx.theme().warning, cx.theme().warning),
                "prod" => (cx.theme().danger, cx.theme().danger),
                "test" => (cx.theme().success, cx.theme().success),
                _ => (cx.theme().border, cx.theme().muted_foreground),
            };
            row3 = row3.child(
                div()
                    .px_1p5()
                    .py_0p5()
                    .rounded_full()
                    .border_1()
                    .border_color(border_c)
                    .text_color(text_c)
                    .text_xs()
                    .flex_shrink_0()
                    .child(env_str),
            );
        }

        for tag in self.tags {
            row3 = row3.child(
                div()
                    .px_1p5()
                    .py_0p5()
                    .rounded_full()
                    .border_1()
                    .border_color(cx.theme().border)
                    .text_color(cx.theme().muted_foreground)
                    .text_xs()
                    .flex_shrink_0()
                    .child(tag),
            );
        }

        if let Some(last_used) = self.last_used {
            row3 = row3.child(
                div()
                    .ml_auto()
                    .text_xs()
                    .text_color(cx.theme().muted_foreground)
                    .flex_shrink_0()
                    .when(on_connect.is_some(), |this| {
                        this.group_hover(CARD_GROUP, |s| s.invisible())
                    })
                    .child(last_used),
            );
        }

        card = card.child(row1).child(row2).child(row3);

        // Hover: "▶ 连接" 按钮在右下角（悬浮时才出现，平滑替代上次使用时间，消除重叠冲突）
        if let Some(connect_cb) = on_connect {
            let connect_btn = div()
                .absolute()
                .right_2()
                .bottom_1p5()
                .invisible()
                .group_hover(CARD_GROUP, |s| s.visible())
                .child(
                    Button::new("quick-connect-btn")
                        .xsmall()
                        .primary()
                        .label("▶ 连接")
                        .tooltip("单击直连")
                        .on_click(move |_, window, cx| {
                            cx.stop_propagation();
                            connect_cb(window, cx);
                        }),
                );
            card = card.child(connect_btn);
        }

        card
    }

    fn render_list_card(self, cx: &App) -> Stateful<gpui::Div> {
        let is_checked = self.checked;
        let show_chk = self.show_checkbox || is_checked;
        let on_check = self.on_check;
        let on_connect = self.on_connect;
        let on_toggle_favorite = self.on_toggle_favorite;
        let is_fav = self.is_favorite;

        let mut card = h_flex()
            .id(self.id.clone())
            .w_full()
            .h(px(40.0))
            .rounded(px(8.0))
            .bg(cx.theme().surface)
            .px_3()
            .border_1()
            .items_center()
            .gap_2()
            .overflow_hidden()
            .group(CARD_GROUP)
            .when(self.selected, |this| {
                this.border_color(cx.theme().accent).border_l_3()
            })
            .when(!self.selected, |this| this.border_color(cx.theme().border))
            .hover(|style| style.border_color(cx.theme().accent))
            .when_some(self.on_click, |this, on_click| {
                this.cursor_pointer().on_click(move |ev, window, cx| {
                    on_click(ev, window, cx);
                })
            })
            .when_some(self.on_double_click, |this, on_double_click| {
                this.on_double_click(move |ev, window, cx| {
                    on_double_click(ev, window, cx);
                })
            });

        // 列表模式采用扁平单行 + 固定宽度列，保证协议/地址/时间各列跨行对齐：
        // [勾选][状态点][名称 flex_1][收藏][协议 w44][地址 w240] ┊ [env][标签][时间 w72][连接][菜单]（右对齐集群）
        // Col: Checkbox（悬浮显示，常驻占位保证列对齐）
        let chk_id = ElementId::NamedInteger("chk-list".into(), 1);
        let check_cb = on_check.clone();
        card = card.child(
            div()
                .w(px(18.0))
                .flex_shrink_0()
                .when(!show_chk, |this| {
                    this.invisible().group_hover(CARD_GROUP, |s| s.visible())
                })
                .child(Checkbox::new(chk_id).checked(is_checked).on_click(
                    move |checked, window, cx| {
                        cx.stop_propagation();
                        if let Some(cb) = &check_cb {
                            cb(*checked, window, cx);
                        }
                    },
                )),
        );

        // Col: 在线状态点
        let dot = div()
            .w(px(8.0))
            .h(px(8.0))
            .rounded_full()
            .flex_shrink_0()
            .when(self.active, |this| {
                this.bg(cx.theme().success)
                    .border_2()
                    .border_color(cx.theme().success.opacity(0.4))
                    .shadow_sm()
            })
            .when(!self.active, |this| this.bg(cx.theme().muted));
        card = card.child(dot);

        // Col: 名称（唯一弹性列，吸收剩余宽度）
        card = card.child(
            div()
                .text_sm()
                .font_weight(FontWeight::SEMIBOLD)
                .text_color(cx.theme().foreground)
                .overflow_hidden()
                .text_ellipsis()
                .whitespace_nowrap()
                .flex_1()
                .min_w_0()
                .child(self.title.clone()),
        );

        // Col: 收藏星标（常显可点击）
        if on_toggle_favorite.is_some() {
            let fav_cb = on_toggle_favorite.clone();
            card = card.child(
                div()
                    .id(ElementId::NamedInteger("fav-list".into(), 1))
                    .text_sm()
                    .flex_shrink_0()
                    .cursor_pointer()
                    .when(is_fav, |d| d.text_color(cx.theme().warning))
                    .when(!is_fav, |d| {
                        d.text_color(cx.theme().muted_foreground.opacity(0.55))
                            .hover(|s| s.text_color(cx.theme().warning))
                    })
                    .on_click(move |_, window, cx| {
                        cx.stop_propagation();
                        if let Some(cb) = &fav_cb {
                            cb(window, cx);
                        }
                    })
                    .child(if is_fav { "★" } else { "☆" }),
            );
        }

        // Col: 协议（固定宽度短标签，跨行对齐）
        if let Some(proto) = self.protocol {
            let proto_color = match proto.to_uppercase().as_str() {
                "SSH" | "SFTP" => cx.theme().connection_ssh,
                "JMS" => cx.theme().connection_jms,
                "PORTFORWARDING" => cx.theme().connection_port_forwarding,
                _ => cx.theme().muted_foreground,
            };
            let proto_upper = proto.to_uppercase();
            let proto_short: SharedString = if proto_upper == "PORTFORWARDING" {
                "PF".into()
            } else {
                proto_upper.clone().into()
            };
            card = card.child(
                div()
                    .w(px(44.0))
                    .flex_shrink_0()
                    .flex()
                    .justify_center()
                    .px_1()
                    .py_0p5()
                    .rounded(px(4.0))
                    .border_1()
                    .border_color(proto_color.opacity(0.35))
                    .bg(proto_color.opacity(0.08))
                    .text_xs()
                    .text_color(proto_color)
                    .overflow_hidden()
                    .text_ellipsis()
                    .whitespace_nowrap()
                    .child(proto_short),
            );
        }

        // Col: 主机地址（固定宽度等宽字体，跨行对齐；极窄窗口时允许收缩）
        let endpoint_text = self
            .endpoint
            .unwrap_or_else(|| self.subtitle.unwrap_or_default());
        card = card.child(
            div()
                .w(px(240.0))
                .min_w(px(120.0))
                .font_family("Consolas")
                .text_xs()
                .text_color(cx.theme().muted_foreground)
                .overflow_hidden()
                .text_ellipsis()
                .whitespace_nowrap()
                .child(endpoint_text),
        );

        // 尾部集群（右对齐）：env + 标签 + 时间 + 连接按钮 + 菜单
        let mut tail = h_flex()
            .items_center()
            .gap_2()
            .ml_auto()
            .flex_shrink_0()
            .min_w_0();

        if let Some(env_str) = self.env {
            let (border_c, text_c) = match env_str.to_lowercase().as_str() {
                "dev" => (cx.theme().warning, cx.theme().warning),
                "prod" => (cx.theme().danger, cx.theme().danger),
                "test" => (cx.theme().success, cx.theme().success),
                _ => (cx.theme().border, cx.theme().muted_foreground),
            };
            tail = tail.child(
                div()
                    .px_1p5()
                    .py_0p5()
                    .rounded_full()
                    .border_1()
                    .border_color(border_c)
                    .text_color(text_c)
                    .text_xs()
                    .flex_shrink_0()
                    .child(env_str),
            );
        }

        for tag in self.tags {
            tail = tail.child(
                div()
                    .px_1p5()
                    .py_0p5()
                    .rounded_full()
                    .border_1()
                    .border_color(cx.theme().border)
                    .text_color(cx.theme().muted_foreground)
                    .text_xs()
                    .flex_shrink_0()
                    .child(tag),
            );
        }

        if let Some(last_used) = self.last_used {
            tail = tail.child(
                div()
                    .w(px(72.0))
                    .flex_shrink_0()
                    .flex()
                    .justify_end()
                    .text_xs()
                    .text_color(cx.theme().muted_foreground)
                    .overflow_hidden()
                    .text_ellipsis()
                    .whitespace_nowrap()
                    .child(last_used),
            );
        }

        if let Some(connect_cb) = on_connect {
            tail = tail.child(
                div()
                    .invisible()
                    .group_hover(CARD_GROUP, |s| s.visible())
                    .flex_shrink_0()
                    .child(
                        Button::new("quick-connect-list-btn")
                            .xsmall()
                            .primary()
                            .label("▶ 连接")
                            .on_click(move |_, window, cx| {
                                cx.stop_propagation();
                                connect_cb(window, cx);
                            }),
                    ),
            );
        }

        if let Some(menu_builder) = self.menu_builder {
            tail = tail.child(
                Button::new(self.menu_id)
                    .icon(IconName::Ellipsis)
                    .xsmall()
                    .ghost()
                    .dropdown_menu(move |menu, window, cx| menu_builder(menu, window, cx)),
            );
        }

        card = card.child(tail);
        card
    }
}
