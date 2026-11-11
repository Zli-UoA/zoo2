//! `control-panel.tsx`。設定画面の枠 (`SettingsSection`)、行 (`SettingRow`)、タブ。

use gpui::{
    AnyElement, App, ElementId, SharedString, StyleRefinement, Window, div, prelude::*, px,
};

use super::{TextSpec, alpha, icon_colored, palette, ring, shadow_sm};
use crate::fonts;
use crate::icons::Icon;
use crate::tokens::radius;

#[derive(IntoElement)]
pub struct SettingsSection {
    title: Option<SharedString>,
    description: Option<SharedString>,
    children: Vec<AnyElement>,
    style: StyleRefinement,
}

impl SettingsSection {
    pub fn new() -> Self {
        Self {
            title: None,
            description: None,
            children: Vec::new(),
            style: StyleRefinement::default(),
        }
    }

    pub fn title(mut self, title: impl Into<SharedString>) -> Self {
        self.title = Some(title.into());
        self
    }

    pub fn description(mut self, description: impl Into<SharedString>) -> Self {
        self.description = Some(description.into());
        self
    }
}

impl ParentElement for SettingsSection {
    fn extend(&mut self, elements: impl IntoIterator<Item = AnyElement>) {
        self.children.extend(elements);
    }
}

impl Styled for SettingsSection {
    fn style(&mut self) -> &mut StyleRefinement {
        &mut self.style
    }
}

impl RenderOnce for SettingsSection {
    fn render(self, _: &mut Window, cx: &mut App) -> impl IntoElement {
        let p = palette(cx);
        let t = p.t;
        let (sm, xs) = (TextSpec::SM, TextSpec::XS);
        // `space-y-3 rounded-lg border p-4`
        let mut section = div()
            .flex()
            .flex_col()
            .gap(px(12.0))
            .rounded(radius::LG)
            .border_1()
            .border_color(t.border)
            .p(px(16.0))
            .when_some(self.title, |d, title| {
                d.child(
                    div()
                        .child(
                            div()
                                .text_size(sm.size())
                                .line_height(sm.line())
                                .font(fonts::sans(600.0))
                                .child(title),
                        )
                        .children(self.description.map(|desc| {
                            div()
                                .mt(px(4.0))
                                .text_size(xs.size())
                                .line_height(xs.line())
                                .text_color(t.muted_foreground)
                                .child(desc)
                        })),
                )
            })
            .children(self.children);
        section.style().refine(&self.style);
        section
    }
}

#[derive(IntoElement)]
pub struct SettingRow {
    icon: Option<Icon>,
    label: SharedString,
    description: Option<SharedString>,
    controls: Vec<AnyElement>,
}

impl SettingRow {
    pub fn new(label: impl Into<SharedString>) -> Self {
        Self {
            icon: None,
            label: label.into(),
            description: None,
            controls: Vec::new(),
        }
    }

    pub fn icon(mut self, icon: Icon) -> Self {
        self.icon = Some(icon);
        self
    }

    pub fn description(mut self, description: impl Into<SharedString>) -> Self {
        self.description = Some(description.into());
        self
    }

    /// 右側の操作。
    pub fn control(mut self, control: impl IntoElement) -> Self {
        self.controls.push(control.into_any_element());
        self
    }
}

impl RenderOnce for SettingRow {
    fn render(self, _: &mut Window, cx: &mut App) -> impl IntoElement {
        let p = palette(cx);
        let t = p.t;
        let (sm, xs) = (TextSpec::SM, TextSpec::XS);
        // `flex min-h-16 items-start justify-between gap-4 p-4`
        div()
            .flex()
            .items_start()
            .justify_between()
            .gap(px(16.0))
            .min_h(px(64.0))
            .p(px(16.0))
            .child(
                // `flex min-h-8 min-w-0 items-center gap-3`
                div()
                    .flex()
                    .items_center()
                    .gap(px(12.0))
                    .min_h(px(32.0))
                    .min_w_0()
                    .children(
                        self.icon
                            .map(|ic| icon_colored(ic, 20.0, t.muted_foreground)),
                    )
                    .child(
                        div()
                            .min_w_0()
                            .child(
                                div()
                                    .text_size(sm.size())
                                    .line_height(sm.line())
                                    .font(fonts::sans(500.0))
                                    .child(self.label),
                            )
                            .children(self.description.map(|d| {
                                div()
                                    .mt(px(4.0))
                                    .text_size(xs.size())
                                    .line_height(xs.line())
                                    .text_color(t.muted_foreground)
                                    .child(d)
                            })),
                    ),
            )
            .child(
                div()
                    .flex()
                    .flex_none()
                    .items_center()
                    .gap(px(8.0))
                    .children(self.controls),
            )
    }
}

/// `panelTabClassName`。選ばれているタブは背景色と影が付く。
#[derive(IntoElement)]
pub struct PanelTab {
    id: ElementId,
    label: SharedString,
    active: bool,
    hovered: bool,
    focused: bool,
}

impl PanelTab {
    pub fn new(id: impl Into<ElementId>, label: impl Into<SharedString>) -> Self {
        Self {
            id: id.into(),
            label: label.into(),
            active: false,
            hovered: false,
            focused: false,
        }
    }

    pub fn active(mut self, active: bool) -> Self {
        self.active = active;
        self
    }

    pub fn hovered(mut self, hovered: bool) -> Self {
        self.hovered = hovered;
        self
    }

    pub fn focused(mut self, focused: bool) -> Self {
        self.focused = focused;
        self
    }
}

impl RenderOnce for PanelTab {
    fn render(self, _: &mut Window, cx: &mut App) -> impl IntoElement {
        let p = palette(cx);
        let t = p.t;
        let sm = TextSpec::SM;
        let color = if self.active || self.hovered {
            t.foreground
        } else {
            t.muted_foreground
        };
        let hover_fg = t.foreground;
        div()
            .id(self.id)
            .px(px(12.0))
            .py(px(8.0))
            .rounded(radius::MD)
            .text_size(sm.size())
            .line_height(sm.line())
            .font(fonts::sans(500.0))
            .text_color(color)
            .when(!self.active, |d| d.hover(move |s| s.text_color(hover_fg)))
            .when(self.active, |d| d.bg(t.background).shadow(shadow_sm()))
            .when(self.focused, |d| {
                d.shadow(vec![ring(alpha(t.ring, 0.5), 3.0)])
            })
            .child(self.label)
    }
}
