use gpui::{
    AnyView, AnyWindowHandle, App, Context, Entity, FocusHandle, Focusable, FontWeight,
    InteractiveElement, IntoElement, KeyBinding, ParentElement, Render, SharedString, Styled,
    Window, actions, div, px,
};
use gpui_component::{
    ActiveTheme, Disableable, Icon, Sizable, Size, TitleBar,
    button::{Button, ButtonVariants as _},
    h_flex,
    scroll::ScrollableElement,
    sidebar::{Sidebar, SidebarMenu, SidebarMenuItem},
    v_flex,
};
use rust_i18n::t;

use crate::home_tab::HomePage;
use crate::new_connection::connection_kind::{NewConnectionCategory, NewConnectionKind};
use crate::new_connection::form_page::{NewConnectionFormPage, NewConnectionFormResult};
use one_ui::KindCard;

const KEY_CONTEXT: &str = "NewConnectionWindow";

actions!(
    new_connection_window,
    [
        SelectPreviousConnectionKind,
        SelectNextConnectionKind,
        OpenConnectionKind
    ]
);

pub(crate) struct NewConnectionWindow {
    parent: Entity<HomePage>,
    parent_window: AnyWindowHandle,
    focus_handle: FocusHandle,
    selected_category: NewConnectionCategory,
    selected_kind: Option<NewConnectionKind>,
    form: Option<AnyView>,
}

impl NewConnectionWindow {
    pub(crate) fn new(
        parent: Entity<HomePage>,
        parent_window: AnyWindowHandle,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        cx.bind_keys([
            KeyBinding::new("up", SelectPreviousConnectionKind, Some(KEY_CONTEXT)),
            KeyBinding::new("left", SelectPreviousConnectionKind, Some(KEY_CONTEXT)),
            KeyBinding::new("down", SelectNextConnectionKind, Some(KEY_CONTEXT)),
            KeyBinding::new("right", SelectNextConnectionKind, Some(KEY_CONTEXT)),
            KeyBinding::new("enter", OpenConnectionKind, Some(KEY_CONTEXT)),
        ]);

        let focus_handle = cx.focus_handle();
        focus_handle.focus(window, cx);

        Self {
            parent,
            parent_window,
            focus_handle,
            selected_category: NewConnectionCategory::All,
            selected_kind: Self::first_visible_item(NewConnectionCategory::All),
            form: None,
        }
    }

    fn first_visible_item(category: NewConnectionCategory) -> Option<NewConnectionKind> {
        NewConnectionKind::all()
            .into_iter()
            .find(|kind| category == NewConnectionCategory::All || kind.category() == category)
    }

    fn visible_items(&self) -> Vec<NewConnectionKind> {
        NewConnectionKind::all()
            .into_iter()
            .filter(|kind| {
                self.selected_category == NewConnectionCategory::All
                    || kind.category() == self.selected_category
            })
            .collect()
    }

    fn select_visible_item(&mut self, offset: isize, cx: &mut Context<Self>) {
        let items = self.visible_items();
        if items.is_empty() {
            return;
        }

        let current_index = self
            .selected_kind
            .as_ref()
            .and_then(|selected| items.iter().position(|kind| kind == selected));
        let next_index = match current_index {
            Some(index) => (index as isize + offset).rem_euclid(items.len() as isize) as usize,
            None if offset < 0 => items.len() - 1,
            None => 0,
        };

        self.selected_kind = Some(items[next_index].clone());
        cx.notify();
    }

    fn open_selected(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(kind) = self.selected_kind.clone() else {
            return;
        };

        match kind.build_form_view(self.parent.clone(), self.parent_window, window, cx) {
            NewConnectionFormResult::Form(form) => {
                self.form = Some(form);
                cx.notify();
            }
            NewConnectionFormResult::Done => {
                window.remove_window();
            }
            NewConnectionFormResult::Blocked => {
                cx.notify();
            }
        }
    }

    fn go_back_to_selection(&mut self, cx: &mut Context<Self>) {
        self.form = None;
        cx.notify();
    }

    fn on_action_select_previous(
        &mut self,
        _: &SelectPreviousConnectionKind,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.select_visible_item(-1, cx);
    }

    fn on_action_select_next(
        &mut self,
        _: &SelectNextConnectionKind,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.select_visible_item(1, cx);
    }

    fn on_action_open_selected(
        &mut self,
        _: &OpenConnectionKind,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.open_selected(window, cx);
    }

    fn render_header(&self, cx: &mut Context<Self>) -> impl IntoElement {
        TitleBar::new().child(
            div()
                .flex()
                .items_center()
                .justify_center()
                .flex_1()
                .text_sm()
                .font_weight(FontWeight::MEDIUM)
                .text_color(cx.theme().foreground)
                .child(t!("Home.new_connection").to_string()),
        )
    }

    fn render_sidebar(&self, cx: &mut Context<Self>) -> impl IntoElement {
        Sidebar::new("new-connection-category-sidebar").child(SidebarMenu::new().children(
            NewConnectionCategory::all().into_iter().map(|category| {
                let is_selected = self.selected_category == category;
                SidebarMenuItem::new(category.label())
                    .icon(Icon::new(category.icon()).mono().with_size(Size::Medium))
                    .active(is_selected)
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.selected_category = category;
                        this.selected_kind = Self::first_visible_item(category);
                        cx.notify();
                    }))
            }),
        ))
    }

    fn render_card_area(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let mut grid = div().flex().flex_wrap().w_full().gap_3();
        for kind in self.visible_items() {
            grid = grid.child(
                div()
                    .flex_1()
                    .min_w(px(240.0))
                    .max_w(px(340.0))
                    .child(self.render_connection_type_card(kind, cx)),
            );
        }

        v_flex()
            .flex_1()
            .h_full()
            .overflow_y_scrollbar()
            .bg(cx.theme().muted)
            .p_6()
            .gap_4()
            .child(
                v_flex()
                    .gap_1()
                    .child(
                        div()
                            .text_lg()
                            .font_weight(FontWeight::SEMIBOLD)
                            .text_color(cx.theme().foreground)
                            .child(t!("NewConnection.select_type_title").to_string()),
                    )
                    .child(
                        div()
                            .text_sm()
                            .text_color(cx.theme().muted_foreground)
                            .child(t!("NewConnection.select_type_hint").to_string()),
                    ),
            )
            .child(grid)
    }

    fn render_connection_type_card(
        &self,
        kind: NewConnectionKind,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let is_selected = self.selected_kind.as_ref() == Some(&kind);
        let click_kind = kind.clone();
        let double_click_kind = kind.clone();

        KindCard::new(
            SharedString::from(format!("new-connection-kind-{}", kind.label())),
            kind.icon(cx),
            kind.label(),
            kind.description(),
        )
        .selected(is_selected)
        .on_click(cx.listener(move |this, _, _, cx| {
            this.selected_kind = Some(click_kind.clone());
            cx.notify();
        }))
        .on_double_click(cx.listener(move |this, _, window, cx| {
            this.selected_kind = Some(double_click_kind.clone());
            this.open_selected(window, cx);
        }))
    }

    fn render_selection_footer(&self, cx: &mut Context<Self>) -> impl IntoElement {
        h_flex()
            .justify_end()
            .gap_2()
            .p_4()
            .border_t_1()
            .border_color(cx.theme().border)
            .child(
                Button::new("cancel-new-connection")
                    .small()
                    .label(t!("Common.cancel").to_string())
                    .on_click(cx.listener(|_, _, window, cx| {
                        window.remove_window();
                        cx.notify();
                    })),
            )
            .child(
                Button::new("next-new-connection")
                    .small()
                    .primary()
                    .label(t!("Common.next").to_string())
                    .disabled(self.selected_kind.is_none())
                    .on_click(cx.listener(|this, _, window, cx| {
                        this.open_selected(window, cx);
                    })),
            )
    }

    fn render_form_page(&self, form: AnyView, cx: &mut Context<Self>) -> impl IntoElement {
        div().size_full().relative().child(form).child(
            div().absolute().left(px(16.0)).bottom(px(16.0)).child(
                Button::new("back-to-new-connection-kind")
                    .small()
                    .outline()
                    .label(t!("Common.previous").to_string())
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.go_back_to_selection(cx);
                    })),
            ),
        )
    }
}

impl Focusable for NewConnectionWindow {
    fn focus_handle(&self, _cx: &App) -> FocusHandle {
        self.focus_handle.clone()
    }
}

impl Render for NewConnectionWindow {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        if let Some(form) = self.form.clone() {
            return self.render_form_page(form, cx).into_any_element();
        }

        v_flex()
            .key_context(KEY_CONTEXT)
            .size_full()
            .track_focus(&self.focus_handle)
            .on_action(cx.listener(Self::on_action_select_previous))
            .on_action(cx.listener(Self::on_action_select_next))
            .on_action(cx.listener(Self::on_action_open_selected))
            .bg(cx.theme().background)
            .child(self.render_header(cx))
            .child(
                h_flex()
                    .flex_1()
                    .w_full()
                    .overflow_hidden()
                    .child(self.render_sidebar(cx))
                    .child(self.render_card_area(cx)),
            )
            .child(self.render_selection_footer(cx))
            .into_any_element()
    }
}
