use gpui::{App, IntoElement, ParentElement, Pixels, RenderOnce, Styled, Window, div, px};
use gpui_component::skeleton::Skeleton;
use gpui_component::v_flex;

/// 卡片式骨架屏占位网格。
#[derive(IntoElement)]
pub struct LoadingGrid {
    columns: usize,
    rows: usize,
    min_card_width: Pixels,
    max_card_width: Pixels,
    card_height: Pixels,
}

impl LoadingGrid {
    pub fn new(rows: usize) -> Self {
        Self {
            columns: 0,
            rows,
            min_card_width: px(260.0),
            max_card_width: px(360.0),
            card_height: px(90.0),
        }
    }

    pub fn columns(mut self, columns: usize) -> Self {
        self.columns = columns;
        self
    }

    pub fn card_size(mut self, min_w: Pixels, max_w: Pixels, height: Pixels) -> Self {
        self.min_card_width = min_w;
        self.max_card_width = max_w;
        self.card_height = height;
        self
    }
}

impl RenderOnce for LoadingGrid {
    fn render(self, _window: &mut Window, _cx: &mut App) -> impl IntoElement {
        let mut grid = div().flex().flex_wrap().w_full().gap_3();

        for _ in 0..self.rows {
            grid = grid.child(
                div()
                    .flex_1()
                    .min_w(self.min_card_width)
                    .max_w(self.max_card_width)
                    .h(self.card_height)
                    .child(
                        v_flex()
                            .size_full()
                            .gap_2()
                            .justify_center()
                            .child(Skeleton::new().w_3_4())
                            .child(Skeleton::new().w_1_2().secondary()),
                    ),
            );
        }

        grid
    }
}
