use gpui::prelude::FluentBuilder;
use gpui::{
    App, ClickEvent, Hsla, InteractiveElement, IntoElement, ParentElement, RenderOnce,
    SharedString, StatefulInteractiveElement, Styled, Window, div, px,
};
use gpui_component::{ActiveTheme, Icon, InteractiveElementExt, button::Button, h_flex, v_flex};
use std::rc::Rc;

const CARD_GROUP: &str = "connection-card";

/// 通用连接/资源卡片，用于首页等场景。
#[derive(IntoElement)]
pub struct ConnectionCard {
    id: gpui::ElementId,
    icon: Icon,
    icon_bg: Option<Hsla>,
    title: SharedString,
    subtitle: Option<SharedString>,
    team_badge: Option<SharedString>,
    selected: bool,
    active: bool,
    actions: Vec<Button>,
    on_click: Option<Rc<dyn Fn(&ClickEvent, &mut Window, &mut App)>>,
    on_double_click: Option<Rc<dyn Fn(&ClickEvent, &mut Window, &mut App)>>,
}

impl ConnectionCard {
    pub fn new(
        id: impl Into<gpui::ElementId>,
        icon: impl Into<Icon>,
        title: impl Into<SharedString>,
    ) -> Self {
        Self {
            id: id.into(),
            icon: icon.into(),
            icon_bg: None,
            title: title.into(),
            subtitle: None,
            team_badge: None,
            selected: false,
            active: false,
            actions: Vec::new(),
            on_click: None,
            on_double_click: None,
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

    pub fn selected(mut self, selected: bool) -> Self {
        self.selected = selected;
        self
    }

    pub fn active(mut self, active: bool) -> Self {
        self.active = active;
        self
    }

    pub fn action(mut self, action: Button) -> Self {
        self.actions.push(action);
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
}

impl RenderOnce for ConnectionCard {
    fn render(self, _window: &mut Window, cx: &mut App) -> impl IntoElement {
        let mut card = v_flex()
            .justify_center()
            .id(self.id)
            .w_full()
            .h(px(90.0))
            .rounded_lg()
            .bg(cx.theme().surface)
            .p_3()
            .border_1()
            .relative()
            .overflow_hidden()
            .shadow_sm()
            .group(CARD_GROUP)
            .when(self.selected, |this| {
                this.border_color(cx.theme().list_active_border)
                    .shadow_lg()
                    .border_l_3()
            })
            .when(!self.selected, |this| this.border_color(cx.theme().border))
            .hover(|style| {
                style
                    .shadow_lg()
                    .border_color(cx.theme().list_active_border)
            })
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

        if self.active {
            card = card.child(
                div()
                    .absolute()
                    .top(px(6.0))
                    .left(px(6.0))
                    .w(px(10.0))
                    .h(px(10.0))
                    .rounded_full()
                    .bg(cx.theme().success)
                    .shadow_lg(),
            );
        }

        if !self.actions.is_empty() {
            card = card.child(
                h_flex()
                    .absolute()
                    .top_2()
                    .right_2()
                    .gap_1()
                    .group_hover(CARD_GROUP, |style| style.opacity(1.0))
                    .opacity(0.0)
                    .children(self.actions),
            );
        }

        card.child(
            h_flex()
                .items_center()
                .gap_2()
                .w_full()
                .child(
                    div()
                        .h(px(48.0))
                        .rounded(px(8.0))
                        .flex()
                        .items_center()
                        .justify_center()
                        .when_some(self.icon_bg, |this, bg| this.bg(bg))
                        .child(self.icon),
                )
                .child(
                    v_flex()
                        .flex_1()
                        .min_w_0()
                        .gap_0p5()
                        .overflow_hidden()
                        .child(
                            h_flex()
                                .gap_1()
                                .overflow_hidden()
                                .child(
                                    div()
                                        .text_sm()
                                        .font_weight(gpui::FontWeight::SEMIBOLD)
                                        .text_color(cx.theme().text_primary)
                                        .overflow_hidden()
                                        .text_ellipsis()
                                        .whitespace_nowrap()
                                        .flex_shrink()
                                        .min_w_0()
                                        .child(self.title),
                                )
                                .when_some(self.team_badge, |this, badge| {
                                    this.child(
                                        div()
                                            .flex_shrink_0()
                                            .px_1()
                                            .rounded(px(3.0))
                                            .bg(cx.theme().accent.opacity(0.15))
                                            .text_color(cx.theme().accent)
                                            .text_xs()
                                            .child(badge),
                                    )
                                }),
                        )
                        .when_some(self.subtitle, |this, subtitle| {
                            this.child(
                                div()
                                    .text_xs()
                                    .text_color(cx.theme().text_secondary)
                                    .overflow_hidden()
                                    .text_ellipsis()
                                    .whitespace_nowrap()
                                    .max_w_full()
                                    .child(subtitle),
                            )
                        }),
                ),
        )
    }
}
