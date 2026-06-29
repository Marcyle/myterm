use gpui::prelude::FluentBuilder;
use gpui::{
    Animation, AnimationExt, App, IntoElement, ParentElement, RenderOnce, SharedString, Styled,
    Window, div,
};
use gpui_component::animation::{DURATION_SLOW, easing_decelerate};
use gpui_component::{ActiveTheme, Icon, button::Button, v_flex};

/// 空状态组件，用于列表/网格无数据时的占位。
#[derive(IntoElement)]
pub struct EmptyState {
    icon: Option<Icon>,
    title: SharedString,
    description: Option<SharedString>,
    action: Option<Button>,
}

impl EmptyState {
    pub fn new(title: impl Into<SharedString>) -> Self {
        Self {
            icon: None,
            title: title.into(),
            description: None,
            action: None,
        }
    }

    pub fn icon(mut self, icon: impl Into<Icon>) -> Self {
        self.icon = Some(icon.into());
        self
    }

    pub fn description(mut self, description: impl Into<SharedString>) -> Self {
        self.description = Some(description.into());
        self
    }

    pub fn action(mut self, action: Button) -> Self {
        self.action = Some(action);
        self
    }
}

impl RenderOnce for EmptyState {
    fn render(self, _window: &mut Window, cx: &mut App) -> impl IntoElement {
        v_flex()
            .size_full()
            .items_center()
            .justify_center()
            .gap_3()
            .py_12()
            .when_some(self.icon, |this, icon| {
                this.child(
                    div()
                        .p_3()
                        .rounded_xl()
                        .bg(cx.theme().surface_elevated)
                        .child(icon.size_8().text_color(cx.theme().muted_foreground)),
                )
            })
            .child(
                div()
                    .text_base()
                    .font_weight(gpui::FontWeight::SEMIBOLD)
                    .text_color(cx.theme().text_primary)
                    .child(self.title),
            )
            .when_some(self.description, |this, description| {
                this.child(
                    div()
                        .text_sm()
                        .text_color(cx.theme().text_secondary)
                        .child(description),
                )
            })
            .when_some(self.action, |this, action| this.child(action))
            .with_animation(
                "empty-state-fade-in",
                Animation::new(DURATION_SLOW).with_easing(easing_decelerate()),
                |this, delta| this.opacity(delta),
            )
    }
}
