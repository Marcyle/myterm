use crate::{ActiveTheme, StyledExt};
use gpui::{
    Animation, AnimationExt, IntoElement, RenderOnce, StyleRefinement, Styled, bounce, div,
    ease_in_out,
};
use std::time::Duration;

/// A skeleton loading placeholder element.
#[derive(IntoElement)]
pub struct Skeleton {
    style: StyleRefinement,
    secondary: bool,
}

impl Skeleton {
    /// Create a new Skeleton element.
    pub fn new() -> Self {
        Self {
            style: StyleRefinement::default(),
            secondary: false,
        }
    }

    /// Set use secondary color.
    pub fn secondary(mut self) -> Self {
        self.secondary = true;
        self
    }
}

impl Styled for Skeleton {
    fn style(&mut self) -> &mut gpui::StyleRefinement {
        &mut self.style
    }
}

impl RenderOnce for Skeleton {
    fn render(self, _: &mut gpui::Window, cx: &mut gpui::App) -> impl IntoElement {
        div()
            .w_full()
            .h_4()
            .rounded(cx.theme().radius_sm)
            .bg(if self.secondary {
                cx.theme().skeleton.opacity(0.5)
            } else {
                cx.theme().skeleton
            })
            .refine_style(&self.style)
            .with_animation(
                "skeleton",
                Animation::new(Duration::from_millis(1400))
                    .repeat()
                    .with_easing(bounce(ease_in_out)),
                move |this, delta| {
                    // 在 0.4~1.0 之间呼吸，避免完全消失带来的闪烁感
                    let v = 1.0 - delta * 0.6;
                    this.opacity(v)
                },
            )
    }
}
