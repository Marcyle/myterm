use gpui::prelude::FluentBuilder;
use gpui::{
    App, ClickEvent, InteractiveElement, IntoElement, ParentElement, RenderOnce, SharedString,
    StatefulInteractiveElement, Styled, Window, div, px,
};
use gpui_component::{ActiveTheme, Icon, InteractiveElementExt, h_flex, v_flex};
use std::rc::Rc;

const CARD_GROUP: &str = "kind-card";

/// 新建连接等场景使用的类型选择卡片。
#[derive(IntoElement)]
pub struct KindCard {
    id: gpui::ElementId,
    icon: Icon,
    title: SharedString,
    description: SharedString,
    selected: bool,
    on_click: Option<Rc<dyn Fn(&ClickEvent, &mut Window, &mut App)>>,
    on_double_click: Option<Rc<dyn Fn(&ClickEvent, &mut Window, &mut App)>>,
}

impl KindCard {
    pub fn new(
        id: impl Into<gpui::ElementId>,
        icon: impl Into<Icon>,
        title: impl Into<SharedString>,
        description: impl Into<SharedString>,
    ) -> Self {
        Self {
            id: id.into(),
            icon: icon.into(),
            title: title.into(),
            description: description.into(),
            selected: false,
            on_click: None,
            on_double_click: None,
        }
    }

    pub fn selected(mut self, selected: bool) -> Self {
        self.selected = selected;
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

impl RenderOnce for KindCard {
    fn render(self, _window: &mut Window, cx: &mut App) -> impl IntoElement {
        v_flex()
            .id(self.id)
            .justify_center()
            .w_full()
            .h(px(112.0))
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
            .when_some(self.on_click.clone(), |this, on_click| {
                this.cursor_pointer().on_click(move |ev, window, cx| {
                    on_click(ev, window, cx);
                })
            })
            .when_some(self.on_double_click, |this, on_double_click| {
                this.on_double_click(move |ev, window, cx| {
                    on_double_click(ev, window, cx);
                })
            })
            .child(
                h_flex()
                    .items_center()
                    .gap_3()
                    .w_full()
                    .child(
                        div()
                            .w(px(48.0))
                            .h(px(48.0))
                            .rounded(px(8.0))
                            .flex()
                            .items_center()
                            .justify_center()
                            .bg(cx.theme().surface_elevated)
                            .child(self.icon),
                    )
                    .child(
                        v_flex()
                            .flex_1()
                            .min_w_0()
                            .gap_1()
                            .child(
                                div()
                                    .text_sm()
                                    .font_weight(gpui::FontWeight::SEMIBOLD)
                                    .text_color(cx.theme().text_primary)
                                    .overflow_hidden()
                                    .text_ellipsis()
                                    .whitespace_nowrap()
                                    .child(self.title),
                            )
                            .child(
                                div()
                                    .text_xs()
                                    .text_color(cx.theme().text_secondary)
                                    .overflow_hidden()
                                    .text_ellipsis()
                                    .whitespace_nowrap()
                                    .child(self.description),
                            ),
                    ),
            )
    }
}
