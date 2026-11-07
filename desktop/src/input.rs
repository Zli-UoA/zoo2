//! 日本語 IME (`EntityInputHandler`) と複数行入力に対応したテキスト入力コンポーネント。

use std::ops::Range;

use gpui::{
    App, Bounds, ClipboardItem, ContentMask, Context, CursorStyle, Element, ElementId,
    ElementInputHandler, Entity, EntityInputHandler, EventEmitter, FocusHandle, Focusable,
    GlobalElementId, InspectorElementId, IntoElement, KeyBinding, LayoutId, MouseButton,
    MouseDownEvent, MouseMoveEvent, MouseUpEvent, PaintQuad, Pixels, Point, Render, ShapedLine,
    SharedString, Style, TextRun, UTF16Selection, UnderlineStyle, Window, actions, div, fill,
    point, prelude::*, px, relative, size,
};
use unicode_segmentation::UnicodeSegmentation;

use crate::theme;

actions!(
    zoo2_text_input,
    [
        Backspace,
        Delete,
        Left,
        Right,
        Up,
        Down,
        SelectLeft,
        SelectRight,
        SelectAll,
        Home,
        End,
        ShowCharacterPalette,
        Paste,
        Cut,
        Copy,
        Submit,
        Newline,
        Cancel,
    ]
);

/// `TextInput` が発火するイベント。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum InputEvent {
    /// テキスト内容が変化した (IME 確定・文字入力・削除・貼り付け)。
    Changed(String),
    /// `Enter` で確定・送信された。
    Submit(String),
    /// `Esc` でキャンセルされた。
    Cancel,
}

/// アプリ起動時に `TextInput` 用のキーバインドを登録する。
pub fn bind_input_keys(cx: &mut App) {
    let ctx = Some("TextInput");
    cx.bind_keys([
        KeyBinding::new("backspace", Backspace, ctx),
        KeyBinding::new("delete", Delete, ctx),
        KeyBinding::new("left", Left, ctx),
        KeyBinding::new("right", Right, ctx),
        KeyBinding::new("up", Up, ctx),
        KeyBinding::new("down", Down, ctx),
        KeyBinding::new("shift-left", SelectLeft, ctx),
        KeyBinding::new("shift-right", SelectRight, ctx),
        KeyBinding::new("home", Home, ctx),
        KeyBinding::new("end", End, ctx),
        KeyBinding::new("enter", Submit, ctx),
        KeyBinding::new("shift-enter", Newline, ctx),
        KeyBinding::new("alt-enter", Newline, ctx),
        KeyBinding::new("escape", Cancel, ctx),
        // macOS ショートカット
        KeyBinding::new("cmd-a", SelectAll, ctx),
        KeyBinding::new("cmd-v", Paste, ctx),
        KeyBinding::new("cmd-c", Copy, ctx),
        KeyBinding::new("cmd-x", Cut, ctx),
        KeyBinding::new("cmd-left", Home, ctx),
        KeyBinding::new("cmd-right", End, ctx),
        KeyBinding::new("ctrl-cmd-space", ShowCharacterPalette, ctx),
        // Windows / Linux ショートカット
        KeyBinding::new("ctrl-a", SelectAll, ctx),
        KeyBinding::new("ctrl-v", Paste, ctx),
        KeyBinding::new("ctrl-c", Copy, ctx),
        KeyBinding::new("ctrl-x", Cut, ctx),
    ]);
}

const MAX_VISIBLE_LINES: usize = 8;

#[derive(Clone)]
struct ShapedRow {
    start: usize,
    end: usize,
    line: ShapedLine,
}

pub struct TextInput {
    focus_handle: FocusHandle,
    content: String,
    placeholder: SharedString,
    multiline: bool,
    selected_range: Range<usize>,
    selection_reversed: bool,
    marked_range: Option<Range<usize>>,
    last_rows: Vec<ShapedRow>,
    last_scroll_row: usize,
    last_scroll_x: Pixels,
    last_line_height: Pixels,
    last_bounds: Option<Bounds<Pixels>>,
    is_selecting: bool,
}

impl EventEmitter<InputEvent> for TextInput {}

impl Focusable for TextInput {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus_handle.clone()
    }
}

impl TextInput {
    pub fn new_single_line(cx: &mut Context<Self>, placeholder: impl Into<SharedString>) -> Self {
        Self::new(cx, placeholder, false)
    }

    pub fn new_multiline(cx: &mut Context<Self>, placeholder: impl Into<SharedString>) -> Self {
        Self::new(cx, placeholder, true)
    }

    fn new(cx: &mut Context<Self>, placeholder: impl Into<SharedString>, multiline: bool) -> Self {
        Self {
            focus_handle: cx.focus_handle(),
            content: String::new(),
            placeholder: placeholder.into(),
            multiline,
            selected_range: 0..0,
            selection_reversed: false,
            marked_range: None,
            last_rows: Vec::new(),
            last_scroll_row: 0,
            last_scroll_x: px(0.0),
            last_line_height: px(20.0),
            last_bounds: None,
            is_selecting: false,
        }
    }

    pub fn text(&self) -> &str {
        &self.content
    }

    pub fn is_empty(&self) -> bool {
        self.content.is_empty()
    }

    pub fn set_placeholder(
        &mut self,
        placeholder: impl Into<SharedString>,
        cx: &mut Context<Self>,
    ) {
        self.placeholder = placeholder.into();
        cx.notify();
    }

    pub fn set_text(&mut self, text: impl Into<String>, cx: &mut Context<Self>) {
        let text = normalize_newlines(&text.into(), self.multiline);
        let len = text.len();
        self.content = text;
        self.selected_range = len..len;
        self.selection_reversed = false;
        self.marked_range = None;
        self.last_scroll_row = 0;
        self.last_scroll_x = px(0.0);
        cx.notify();
    }

    pub fn clear(&mut self, cx: &mut Context<Self>) {
        self.content.clear();
        self.selected_range = 0..0;
        self.selection_reversed = false;
        self.marked_range = None;
        self.last_rows.clear();
        self.last_scroll_row = 0;
        self.last_scroll_x = px(0.0);
        self.is_selecting = false;
        cx.notify();
    }

    fn cursor_offset(&self) -> usize {
        if self.selection_reversed {
            self.selected_range.start
        } else {
            self.selected_range.end
        }
    }

    fn move_to(&mut self, offset: usize, cx: &mut Context<Self>) {
        let offset = offset.min(self.content.len());
        self.selected_range = offset..offset;
        self.selection_reversed = false;
        cx.notify();
    }

    fn select_to(&mut self, offset: usize, cx: &mut Context<Self>) {
        let offset = offset.min(self.content.len());
        if self.selection_reversed {
            self.selected_range.start = offset;
        } else {
            self.selected_range.end = offset;
        }
        if self.selected_range.end < self.selected_range.start {
            self.selection_reversed = !self.selection_reversed;
            self.selected_range = self.selected_range.end..self.selected_range.start;
        }
        cx.notify();
    }

    fn previous_boundary(&self, offset: usize) -> usize {
        previous_grapheme_boundary(&self.content, offset)
    }

    fn next_boundary(&self, offset: usize) -> usize {
        next_grapheme_boundary(&self.content, offset)
    }

    fn left(&mut self, _: &Left, _: &mut Window, cx: &mut Context<Self>) {
        if self.selected_range.is_empty() {
            self.move_to(self.previous_boundary(self.cursor_offset()), cx);
        } else {
            self.move_to(self.selected_range.start, cx);
        }
    }

    fn right(&mut self, _: &Right, _: &mut Window, cx: &mut Context<Self>) {
        if self.selected_range.is_empty() {
            self.move_to(self.next_boundary(self.selected_range.end), cx);
        } else {
            self.move_to(self.selected_range.end, cx);
        }
    }

    fn up(&mut self, _: &Up, _: &mut Window, cx: &mut Context<Self>) {
        let target = self.vertical_move_target(-1);
        self.move_to(target, cx);
    }

    fn down(&mut self, _: &Down, _: &mut Window, cx: &mut Context<Self>) {
        let target = self.vertical_move_target(1);
        self.move_to(target, cx);
    }

    fn vertical_move_target(&self, delta_rows: isize) -> usize {
        let cursor = self.cursor_offset();
        let ranges = line_byte_ranges(&self.content);
        let row_idx = row_index_for_offset(&ranges, cursor);
        if delta_rows < 0 && row_idx == 0 {
            return 0;
        }
        if delta_rows > 0 && row_idx + 1 >= ranges.len() {
            return self.content.len();
        }
        let target_row_idx = (row_idx as isize + delta_rows) as usize;
        if let (Some(cur_row), Some(target_row)) = (
            self.last_rows.get(row_idx),
            self.last_rows.get(target_row_idx),
        ) && cur_row.start == ranges[row_idx].start
            && cur_row.end == ranges[row_idx].end
            && target_row.start == ranges[target_row_idx].start
            && target_row.end == ranges[target_row_idx].end
            && cur_row.line.width > px(0.0)
        {
            let local_cursor = cursor.saturating_sub(cur_row.start);
            let x = cur_row.line.x_for_index(local_cursor);
            let target_local = target_row.line.closest_index_for_x(x);
            return (target_row.start + target_local).min(target_row.end);
        }
        move_offset_vertically(&self.content, cursor, delta_rows)
    }

    fn select_left(&mut self, _: &SelectLeft, _: &mut Window, cx: &mut Context<Self>) {
        self.select_to(self.previous_boundary(self.cursor_offset()), cx);
    }

    fn select_right(&mut self, _: &SelectRight, _: &mut Window, cx: &mut Context<Self>) {
        self.select_to(self.next_boundary(self.cursor_offset()), cx);
    }

    fn select_all(&mut self, _: &SelectAll, _: &mut Window, cx: &mut Context<Self>) {
        self.move_to(0, cx);
        self.select_to(self.content.len(), cx);
    }

    fn home(&mut self, _: &Home, _: &mut Window, cx: &mut Context<Self>) {
        let cursor = self.cursor_offset();
        let ranges = line_byte_ranges(&self.content);
        let row_idx = row_index_for_offset(&ranges, cursor);
        self.move_to(ranges[row_idx].start, cx);
    }

    fn end(&mut self, _: &End, _: &mut Window, cx: &mut Context<Self>) {
        let cursor = self.cursor_offset();
        let ranges = line_byte_ranges(&self.content);
        let row_idx = row_index_for_offset(&ranges, cursor);
        self.move_to(ranges[row_idx].end, cx);
    }

    fn backspace(&mut self, _: &Backspace, window: &mut Window, cx: &mut Context<Self>) {
        if self.selected_range.is_empty() {
            self.select_to(self.previous_boundary(self.cursor_offset()), cx);
        }
        self.replace_text_in_range(None, "", window, cx);
    }

    fn delete(&mut self, _: &Delete, window: &mut Window, cx: &mut Context<Self>) {
        if self.selected_range.is_empty() {
            self.select_to(self.next_boundary(self.cursor_offset()), cx);
        }
        self.replace_text_in_range(None, "", window, cx);
    }

    fn newline(&mut self, _: &Newline, window: &mut Window, cx: &mut Context<Self>) {
        if self.multiline {
            self.replace_text_in_range(None, "\n", window, cx);
        } else {
            self.submit(&Submit, window, cx);
        }
    }

    /// IME 変換中でなければ `InputEvent::Submit` を発火する。
    pub fn request_submit(&mut self, cx: &mut Context<Self>) {
        // IME 変換中なら確定を優先し、送信しない。
        if self.marked_range.is_some() {
            return;
        }
        cx.emit(InputEvent::Submit(self.content.clone()));
    }

    fn submit(&mut self, _: &Submit, _window: &mut Window, cx: &mut Context<Self>) {
        self.request_submit(cx);
    }

    fn cancel(&mut self, _: &Cancel, _window: &mut Window, cx: &mut Context<Self>) {
        if self.marked_range.is_some() {
            self.marked_range = None;
            cx.notify();
            return;
        }
        cx.emit(InputEvent::Cancel);
    }

    fn show_character_palette(
        &mut self,
        _: &ShowCharacterPalette,
        window: &mut Window,
        _: &mut Context<Self>,
    ) {
        window.show_character_palette();
    }

    fn paste(&mut self, _: &Paste, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(text) = cx.read_from_clipboard().and_then(|item| item.text()) {
            let normalized = normalize_newlines(&text, self.multiline);
            self.replace_text_in_range(None, &normalized, window, cx);
        }
    }

    fn copy(&mut self, _: &Copy, _: &mut Window, cx: &mut Context<Self>) {
        if !self.selected_range.is_empty() {
            cx.write_to_clipboard(ClipboardItem::new_string(
                self.content[self.selected_range.clone()].to_string(),
            ));
        }
    }

    fn cut(&mut self, _: &Cut, window: &mut Window, cx: &mut Context<Self>) {
        if !self.selected_range.is_empty() {
            cx.write_to_clipboard(ClipboardItem::new_string(
                self.content[self.selected_range.clone()].to_string(),
            ));
            self.replace_text_in_range(None, "", window, cx);
        }
    }

    fn on_mouse_down(
        &mut self,
        event: &MouseDownEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        window.focus(&self.focus_handle);
        self.is_selecting = true;
        let idx = self.index_for_mouse_position(event.position);
        if event.modifiers.shift {
            self.select_to(idx, cx);
        } else {
            self.move_to(idx, cx);
        }
    }

    fn on_mouse_up(&mut self, _: &MouseUpEvent, _window: &mut Window, _: &mut Context<Self>) {
        self.is_selecting = false;
    }

    fn on_mouse_move(&mut self, event: &MouseMoveEvent, _: &mut Window, cx: &mut Context<Self>) {
        if self.is_selecting {
            let idx = self.index_for_mouse_position(event.position);
            self.select_to(idx, cx);
        }
    }

    fn index_for_mouse_position(&self, position: Point<Pixels>) -> usize {
        if self.content.is_empty() || self.last_rows.is_empty() {
            return 0;
        }
        let Some(bounds) = self.last_bounds.as_ref() else {
            return 0;
        };
        let visible_rows = if self.multiline {
            self.last_rows
                .len()
                .saturating_sub(self.last_scroll_row)
                .clamp(1, MAX_VISIBLE_LINES)
        } else {
            1
        };
        if position.y <= bounds.top() {
            return self.last_rows[self.last_scroll_row].start;
        }
        let rel_y = (position.y - bounds.top()).max(px(0.0));
        let line_h = self.last_line_height.max(px(1.0));
        let vis_idx = ((rel_y / line_h) as usize).min(visible_rows.saturating_sub(1));
        let row_idx = (self.last_scroll_row + vis_idx).min(self.last_rows.len().saturating_sub(1));
        let row = &self.last_rows[row_idx];
        let local_x = (position.x - bounds.left() + self.last_scroll_x).max(px(0.0));
        let local_offset = row.line.closest_index_for_x(local_x);
        (row.start + local_offset).min(row.end)
    }
}

impl EntityInputHandler for TextInput {
    fn text_for_range(
        &mut self,
        range_utf16: Range<usize>,
        actual_range: &mut Option<Range<usize>>,
        _window: &mut Window,
        _cx: &mut Context<Self>,
    ) -> Option<String> {
        let range = range_from_utf16(&self.content, &range_utf16);
        actual_range.replace(range_to_utf16(&self.content, &range));
        Some(self.content[range].to_string())
    }

    fn selected_text_range(
        &mut self,
        _ignore_disabled_input: bool,
        _window: &mut Window,
        _cx: &mut Context<Self>,
    ) -> Option<UTF16Selection> {
        Some(UTF16Selection {
            range: range_to_utf16(&self.content, &self.selected_range),
            reversed: self.selection_reversed,
        })
    }

    fn marked_text_range(
        &self,
        _window: &mut Window,
        _cx: &mut Context<Self>,
    ) -> Option<Range<usize>> {
        self.marked_range
            .as_ref()
            .map(|range| range_to_utf16(&self.content, range))
    }

    fn unmark_text(&mut self, _window: &mut Window, _cx: &mut Context<Self>) {
        self.marked_range = None;
    }

    fn replace_text_in_range(
        &mut self,
        range_utf16: Option<Range<usize>>,
        new_text: &str,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let range = range_utf16
            .as_ref()
            .map(|r| range_from_utf16(&self.content, r))
            .or_else(|| self.marked_range.clone())
            .unwrap_or_else(|| self.selected_range.clone());

        let normalized = normalize_newlines(new_text, self.multiline);
        self.content = format!(
            "{}{}{}",
            &self.content[0..range.start],
            normalized,
            &self.content[range.end..]
        );
        let new_cursor = range.start + normalized.len();
        self.selected_range = new_cursor..new_cursor;
        self.marked_range = None;
        cx.emit(InputEvent::Changed(self.content.clone()));
        cx.notify();
    }

    fn replace_and_mark_text_in_range(
        &mut self,
        range_utf16: Option<Range<usize>>,
        new_text: &str,
        new_selected_range_utf16: Option<Range<usize>>,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let range = range_utf16
            .as_ref()
            .map(|r| range_from_utf16(&self.content, r))
            .or_else(|| self.marked_range.clone())
            .unwrap_or_else(|| self.selected_range.clone());

        let normalized = normalize_newlines(new_text, self.multiline);
        self.content = format!(
            "{}{}{}",
            &self.content[0..range.start],
            normalized,
            &self.content[range.end..]
        );
        if !normalized.is_empty() {
            self.marked_range = Some(range.start..range.start + normalized.len());
        } else {
            self.marked_range = None;
        }
        self.selected_range = new_selected_range_utf16
            .as_ref()
            .map(|r| {
                let local = range_from_utf16(&normalized, r);
                (range.start + local.start)..(range.start + local.end)
            })
            .unwrap_or_else(|| {
                let end = range.start + normalized.len();
                end..end
            });

        cx.emit(InputEvent::Changed(self.content.clone()));
        cx.notify();
    }

    fn bounds_for_range(
        &mut self,
        range_utf16: Range<usize>,
        bounds: Bounds<Pixels>,
        _window: &mut Window,
        _cx: &mut Context<Self>,
    ) -> Option<Bounds<Pixels>> {
        if self.last_rows.is_empty() {
            return None;
        }
        let range = range_from_utf16(&self.content, &range_utf16);
        let row_idx = self
            .last_rows
            .iter()
            .position(|row| range.start >= row.start && range.start <= row.end)
            .unwrap_or(self.last_rows.len() - 1);
        let row = &self.last_rows[row_idx];
        let local_start = range.start.saturating_sub(row.start);
        let local_end = range.end.min(row.end).saturating_sub(row.start);
        let vis_row = row_idx.saturating_sub(self.last_scroll_row);
        let top = bounds.top() + self.last_line_height * (vis_row as f32);
        let bottom = top + self.last_line_height;
        let x1 = (bounds.left() + row.line.x_for_index(local_start) - self.last_scroll_x)
            .clamp(bounds.left(), bounds.right());
        let x2 = (bounds.left() + row.line.x_for_index(local_end) - self.last_scroll_x)
            .clamp(bounds.left(), bounds.right());
        Some(Bounds::from_corners(point(x1, top), point(x2, bottom)))
    }

    fn character_index_for_point(
        &mut self,
        point: Point<Pixels>,
        _window: &mut Window,
        _cx: &mut Context<Self>,
    ) -> Option<usize> {
        let utf8_idx = self.index_for_mouse_position(point);
        Some(offset_to_utf16(&self.content, utf8_idx))
    }
}

struct TextElement {
    input: Entity<TextInput>,
}

struct PrepaintState {
    rows: Vec<ShapedRow>,
    scroll_row: usize,
    scroll_x: Pixels,
    visible_rows: usize,
    line_height: Pixels,
    cursor: Option<PaintQuad>,
    selections: Vec<PaintQuad>,
}

impl IntoElement for TextElement {
    type Element = Self;

    fn into_element(self) -> Self::Element {
        self
    }
}

impl Element for TextElement {
    type RequestLayoutState = ();
    type PrepaintState = PrepaintState;

    fn id(&self) -> Option<ElementId> {
        None
    }

    fn source_location(&self) -> Option<&'static core::panic::Location<'static>> {
        None
    }

    fn request_layout(
        &mut self,
        _id: Option<&GlobalElementId>,
        _inspector_id: Option<&InspectorElementId>,
        window: &mut Window,
        cx: &mut App,
    ) -> (LayoutId, Self::RequestLayoutState) {
        let input = self.input.read(cx);
        let line_count = if input.multiline {
            input
                .content
                .split('\n')
                .count()
                .clamp(1, MAX_VISIBLE_LINES)
        } else {
            1
        };
        let mut style = Style::default();
        style.size.width = relative(1.0).into();
        style.size.height = (window.line_height() * (line_count as f32)).into();
        (window.request_layout(style, [], cx), ())
    }

    fn prepaint(
        &mut self,
        _id: Option<&GlobalElementId>,
        _inspector_id: Option<&InspectorElementId>,
        bounds: Bounds<Pixels>,
        _request_layout: &mut Self::RequestLayoutState,
        window: &mut Window,
        cx: &mut App,
    ) -> Self::PrepaintState {
        let input = self.input.read(cx);
        let content = input.content.clone();
        let multiline = input.multiline;
        let selected_range = input.selected_range.clone();
        let marked_range = input.marked_range.clone();
        let cursor = input.cursor_offset();
        let last_scroll_row = input.last_scroll_row;
        let last_scroll_x = input.last_scroll_x;
        let style = window.text_style();
        let line_height = window.line_height();
        let font_size = style.font_size.to_pixels(window.rem_size());

        let mut rows = Vec::new();
        if content.is_empty() {
            let placeholder = input.placeholder.clone();
            let run = TextRun {
                len: placeholder.len(),
                font: style.font(),
                color: theme::placeholder_color(),
                background_color: None,
                underline: None,
                strikethrough: None,
            };
            let line = window
                .text_system()
                .shape_line(placeholder, font_size, &[run], None);
            rows.push(ShapedRow {
                start: 0,
                end: 0,
                line,
            });
        } else {
            for range in line_byte_ranges(&content) {
                let slice = &content[range.clone()];
                let base_run = TextRun {
                    len: slice.len(),
                    font: style.font(),
                    color: style.color,
                    background_color: None,
                    underline: None,
                    strikethrough: None,
                };
                let runs = if let Some(marked) = marked_range.as_ref() {
                    let overlap_start = marked.start.max(range.start).min(range.end) - range.start;
                    let overlap_end = marked.end.max(range.start).min(range.end) - range.start;
                    if overlap_start < overlap_end {
                        vec![
                            TextRun {
                                len: overlap_start,
                                ..base_run.clone()
                            },
                            TextRun {
                                len: overlap_end - overlap_start,
                                underline: Some(UnderlineStyle {
                                    color: Some(theme::ACCENT.into()),
                                    thickness: px(1.5),
                                    wavy: false,
                                }),
                                ..base_run.clone()
                            },
                            TextRun {
                                len: slice.len() - overlap_end,
                                ..base_run
                            },
                        ]
                        .into_iter()
                        .filter(|r| r.len > 0)
                        .collect()
                    } else if slice.is_empty() {
                        Vec::new()
                    } else {
                        vec![base_run]
                    }
                } else if slice.is_empty() {
                    Vec::new()
                } else {
                    vec![base_run]
                };

                let shaped = window.text_system().shape_line(
                    SharedString::from(slice.to_string()),
                    font_size,
                    &runs,
                    None,
                );
                rows.push(ShapedRow {
                    start: range.start,
                    end: range.end,
                    line: shaped,
                });
            }
        }

        let cursor_row = rows
            .iter()
            .position(|r| cursor >= r.start && cursor <= r.end)
            .unwrap_or(rows.len().saturating_sub(1));
        let max_vis = if multiline { MAX_VISIBLE_LINES } else { 1 };
        let visible_rows = rows.len().clamp(1, max_vis);
        let scroll_row =
            compute_visible_scroll_row(rows.len(), cursor_row, last_scroll_row, visible_rows);

        let (cursor_x_in_row, row_width) = rows
            .get(cursor_row)
            .map(|r| {
                (
                    r.line.x_for_index(cursor.saturating_sub(r.start)),
                    r.line.width,
                )
            })
            .unwrap_or((px(0.0), px(0.0)));
        let scroll_x =
            compute_visible_scroll_x(cursor_x_in_row, row_width, last_scroll_x, bounds.size.width);

        let mut selections = Vec::new();
        let mut cursor_quad = None;

        if selected_range.is_empty() {
            if cursor_row >= scroll_row
                && cursor_row < scroll_row + visible_rows
                && let Some(row) = rows.get(cursor_row)
            {
                let vis_idx = cursor_row - scroll_row;
                let local_cursor = cursor.saturating_sub(row.start);
                let x = bounds.left() + row.line.x_for_index(local_cursor) - scroll_x;
                let y = bounds.top() + line_height * (vis_idx as f32);
                cursor_quad = Some(fill(
                    Bounds::new(point(x, y), size(px(2.0), line_height)),
                    theme::ACCENT,
                ));
            }
        } else {
            for (vis_idx, row) in rows.iter().skip(scroll_row).take(visible_rows).enumerate() {
                let sel_start = selected_range.start.max(row.start).min(row.end);
                let sel_end = selected_range.end.max(row.start).min(row.end);
                if sel_start < sel_end {
                    let x1 = bounds.left() + row.line.x_for_index(sel_start - row.start) - scroll_x;
                    let x2 = bounds.left() + row.line.x_for_index(sel_end - row.start) - scroll_x;
                    let y = bounds.top() + line_height * (vis_idx as f32);
                    selections.push(fill(
                        Bounds::from_corners(point(x1, y), point(x2, y + line_height)),
                        theme::selection_bg(),
                    ));
                }
            }
        }

        PrepaintState {
            rows,
            scroll_row,
            scroll_x,
            visible_rows,
            line_height,
            cursor: cursor_quad,
            selections,
        }
    }

    fn paint(
        &mut self,
        _id: Option<&GlobalElementId>,
        _inspector_id: Option<&InspectorElementId>,
        bounds: Bounds<Pixels>,
        _request_layout: &mut Self::RequestLayoutState,
        prepaint: &mut Self::PrepaintState,
        window: &mut Window,
        cx: &mut App,
    ) {
        let focus_handle = self.input.read(cx).focus_handle.clone();
        window.handle_input(
            &focus_handle,
            ElementInputHandler::new(bounds, self.input.clone()),
            cx,
        );
        let line_height = prepaint.line_height;
        let scroll_row = prepaint.scroll_row;
        let scroll_x = prepaint.scroll_x;
        let visible_rows = prepaint.visible_rows;
        window.with_content_mask(Some(ContentMask { bounds }), |window| {
            for selection in prepaint.selections.drain(..) {
                window.paint_quad(selection);
            }
            for (vis_idx, row) in prepaint
                .rows
                .iter()
                .skip(scroll_row)
                .take(visible_rows)
                .enumerate()
            {
                let origin = point(
                    bounds.left() - scroll_x,
                    bounds.top() + line_height * (vis_idx as f32),
                );
                let _ = row.line.paint(origin, line_height, window, cx);
            }
            if focus_handle.is_focused(window)
                && let Some(cursor) = prepaint.cursor.take()
            {
                window.paint_quad(cursor);
            }
        });
        let rows = std::mem::take(&mut prepaint.rows);
        self.input.update(cx, |input, _cx| {
            input.last_rows = rows;
            input.last_scroll_row = scroll_row;
            input.last_scroll_x = scroll_x;
            input.last_line_height = line_height;
            input.last_bounds = Some(bounds);
        });
    }
}

impl Render for TextInput {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let focused = self.focus_handle.is_focused(window);
        let border_color = if focused {
            theme::BORDER_ACCENT
        } else {
            theme::BORDER
        };

        div()
            .flex()
            .w_full()
            .key_context("TextInput")
            .track_focus(&self.focus_handle(cx))
            .cursor(CursorStyle::IBeam)
            .on_action(cx.listener(Self::backspace))
            .on_action(cx.listener(Self::delete))
            .on_action(cx.listener(Self::left))
            .on_action(cx.listener(Self::right))
            .on_action(cx.listener(Self::up))
            .on_action(cx.listener(Self::down))
            .on_action(cx.listener(Self::select_left))
            .on_action(cx.listener(Self::select_right))
            .on_action(cx.listener(Self::select_all))
            .on_action(cx.listener(Self::home))
            .on_action(cx.listener(Self::end))
            .on_action(cx.listener(Self::show_character_palette))
            .on_action(cx.listener(Self::paste))
            .on_action(cx.listener(Self::cut))
            .on_action(cx.listener(Self::copy))
            .on_action(cx.listener(Self::submit))
            .on_action(cx.listener(Self::newline))
            .on_action(cx.listener(Self::cancel))
            .on_mouse_down(MouseButton::Left, cx.listener(Self::on_mouse_down))
            .on_mouse_up(MouseButton::Left, cx.listener(Self::on_mouse_up))
            .on_mouse_up_out(MouseButton::Left, cx.listener(Self::on_mouse_up))
            .on_mouse_move(cx.listener(Self::on_mouse_move))
            .bg(theme::BG_SURFACE)
            .border_1()
            .border_color(border_color)
            .rounded_md()
            .px_3()
            .py_2()
            .text_sm()
            .line_height(px(20.0))
            .text_color(theme::TEXT_PRIMARY)
            .child(TextElement { input: cx.entity() })
    }
}

/// 改行コードを正規化する。単一行モードでは改行を空白に置き換える。
pub fn normalize_newlines(text: &str, multiline: bool) -> String {
    let unified = text.replace("\r\n", "\n").replace('\r', "\n");
    if multiline {
        unified
    } else {
        unified.replace('\n', " ")
    }
}

/// `\n` で区切られた各行の UTF-8 バイト範囲を返す。
pub fn line_byte_ranges(text: &str) -> Vec<Range<usize>> {
    let mut ranges = Vec::new();
    let mut start = 0;
    for (idx, ch) in text.char_indices() {
        if ch == '\n' {
            ranges.push(start..idx);
            start = idx + 1;
        }
    }
    ranges.push(start..text.len());
    ranges
}

pub fn row_index_for_offset(ranges: &[Range<usize>], offset: usize) -> usize {
    ranges
        .iter()
        .position(|r| offset >= r.start && offset <= r.end)
        .unwrap_or_else(|| ranges.len().saturating_sub(1))
}

/// 複数行入力で表示ウィンドウ (最大 `visible_rows` 行) の先頭行インデックスを求める。
pub fn compute_visible_scroll_row(
    total_rows: usize,
    cursor_row: usize,
    last_scroll_row: usize,
    visible_rows: usize,
) -> usize {
    let visible_rows = visible_rows.max(1);
    if total_rows <= visible_rows {
        return 0;
    }
    let max_scroll = total_rows - visible_rows;
    let mut scroll = last_scroll_row.min(max_scroll);
    if cursor_row < scroll {
        scroll = cursor_row;
    } else if cursor_row >= scroll + visible_rows {
        scroll = cursor_row + 1 - visible_rows;
    }
    scroll.min(max_scroll)
}

/// 1行の幅が入力欄の幅を超えるときに、カーソル位置を表示範囲に収める横スクロール量を求める。
/// 行自体が短縮・置換されて `row_width <= usable_width` になった場合は `0` に戻す。
pub fn compute_visible_scroll_x(
    cursor_x: Pixels,
    row_width: Pixels,
    last_scroll_x: Pixels,
    available_width: Pixels,
) -> Pixels {
    let margin = px(6.0);
    let usable_width = (available_width - margin).max(px(1.0));
    let max_scroll_x = (row_width.max(cursor_x) - usable_width).max(px(0.0));
    let mut scroll_x = last_scroll_x.clamp(px(0.0), max_scroll_x);
    if cursor_x < scroll_x {
        scroll_x = cursor_x;
    } else if cursor_x > scroll_x + usable_width {
        scroll_x = cursor_x - usable_width;
    }
    scroll_x.clamp(px(0.0), max_scroll_x)
}

/// 複数行テキストで上下に移動したときのバイトオフセットを書記素 (grapheme) 列インデックス基準で求める。
pub fn move_offset_vertically(text: &str, cursor: usize, delta_rows: isize) -> usize {
    let ranges = line_byte_ranges(text);
    let row_idx = row_index_for_offset(&ranges, cursor);
    if delta_rows < 0 && row_idx == 0 {
        return 0;
    }
    if delta_rows > 0 && row_idx + 1 >= ranges.len() {
        return text.len();
    }
    let cur_range = &ranges[row_idx];
    let clamped_cursor = clamp_to_char_boundary(text, cursor.clamp(cur_range.start, cur_range.end));
    let grapheme_col = text[cur_range.start..clamped_cursor]
        .graphemes(true)
        .count();
    let target_row_idx = (row_idx as isize + delta_rows) as usize;
    let target_range = &ranges[target_row_idx];
    let target_slice = &text[target_range.clone()];
    target_slice
        .grapheme_indices(true)
        .nth(grapheme_col)
        .map(|(i, _)| target_range.start + i)
        .unwrap_or(target_range.end)
}

pub fn clamp_to_char_boundary(text: &str, mut offset: usize) -> usize {
    offset = offset.min(text.len());
    while offset > 0 && !text.is_char_boundary(offset) {
        offset -= 1;
    }
    offset
}

pub fn previous_grapheme_boundary(text: &str, offset: usize) -> usize {
    text.grapheme_indices(true)
        .rev()
        .find_map(|(idx, _)| (idx < offset).then_some(idx))
        .unwrap_or(0)
}

pub fn next_grapheme_boundary(text: &str, offset: usize) -> usize {
    text.grapheme_indices(true)
        .find_map(|(idx, _)| (idx > offset).then_some(idx))
        .unwrap_or(text.len())
}

pub fn offset_from_utf16(text: &str, utf16_offset: usize) -> usize {
    let mut utf8_offset = 0;
    let mut utf16_count = 0;
    for ch in text.chars() {
        if utf16_count >= utf16_offset {
            break;
        }
        utf16_count += ch.len_utf16();
        utf8_offset += ch.len_utf8();
    }
    utf8_offset
}

pub fn offset_to_utf16(text: &str, utf8_offset: usize) -> usize {
    let mut utf16_offset = 0;
    let mut utf8_count = 0;
    for ch in text.chars() {
        if utf8_count >= utf8_offset {
            break;
        }
        utf8_count += ch.len_utf8();
        utf16_offset += ch.len_utf16();
    }
    utf16_offset
}

pub fn range_to_utf16(text: &str, range: &Range<usize>) -> Range<usize> {
    offset_to_utf16(text, range.start)..offset_to_utf16(text, range.end)
}

pub fn range_from_utf16(text: &str, range_utf16: &Range<usize>) -> Range<usize> {
    offset_from_utf16(text, range_utf16.start)..offset_from_utf16(text, range_utf16.end)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn converts_utf16_and_utf8_offsets_with_japanese_and_emoji() {
        let text = "あ👍b";
        // 'あ': 3 bytes UTF-8, 1 code unit UTF-16
        // '👍': 4 bytes UTF-8, 2 code units UTF-16
        // 'b': 1 byte UTF-8, 1 code unit UTF-16
        assert_eq!(offset_to_utf16(text, 0), 0);
        assert_eq!(offset_to_utf16(text, 3), 1);
        assert_eq!(offset_to_utf16(text, 7), 3);
        assert_eq!(offset_to_utf16(text, 8), 4);

        assert_eq!(offset_from_utf16(text, 0), 0);
        assert_eq!(offset_from_utf16(text, 1), 3);
        assert_eq!(offset_from_utf16(text, 3), 7);
        assert_eq!(offset_from_utf16(text, 4), 8);
    }

    #[test]
    fn splits_line_byte_ranges_and_normalizes_newlines() {
        assert_eq!(
            line_byte_ranges("1行目\n2行目\n"),
            vec![0..7, 8..15, 16..16]
        );
        assert_eq!(normalize_newlines("a\r\nb\rc", true), "a\nb\nc".to_string());
        assert_eq!(normalize_newlines("a\r\nb\rc", false), "a b c".to_string());
    }

    #[test]
    fn navigates_grapheme_boundaries_safely() {
        let text = "a🇯🇵b";
        let end = text.len();
        let before_b = previous_grapheme_boundary(text, end);
        assert_eq!(&text[before_b..], "b");
        let before_flag = previous_grapheme_boundary(text, before_b);
        assert_eq!(before_flag, 1);
        assert_eq!(next_grapheme_boundary(text, 1), before_b);

        // 日本語や絵文字を含む行と ASCII 行の間での上下移動は書記素列を維持する
        let multiline = "あ\nabcdef";
        // 1行目末尾 ("あ" の直後, 3 bytes, 1書記素目) から下へ移動すると "a" の直後 (offset 5) になる
        assert_eq!(move_offset_vertically(multiline, 3, 1), 5);
        assert_eq!(
            &multiline[..move_offset_vertically(multiline, 3, 1)],
            "あ\na"
        );
        // 2行目の "a" の直後 (offset 5) から上へ移動すると "あ" の直後 (offset 3) に戻る
        assert_eq!(move_offset_vertically(multiline, 5, -1), 3);
    }

    #[test]
    fn scrolls_visible_window_for_multiline_input_exceeding_max_lines() {
        assert_eq!(compute_visible_scroll_row(5, 4, 0, 8), 0);
        // 10行でカーソルが末尾(9行目)に移動したときは 2..10 行目を表示する
        assert_eq!(compute_visible_scroll_row(10, 9, 0, 8), 2);
        // カーソルが表示範囲内(5行目)にある間はスクロール位置(2)を維持する
        assert_eq!(compute_visible_scroll_row(10, 5, 2, 8), 2);
        // カーソルが上端より上(1行目)へ移動したときはカーソル行まで戻す
        assert_eq!(compute_visible_scroll_row(10, 1, 2, 8), 1);

        // 横方向のスクロール追従
        assert_eq!(
            compute_visible_scroll_x(px(50.0), px(50.0), px(0.0), px(100.0)),
            px(0.0)
        );
        assert_eq!(
            compute_visible_scroll_x(px(140.0), px(140.0), px(0.0), px(100.0)),
            px(46.0)
        );
        assert_eq!(
            compute_visible_scroll_x(px(80.0), px(140.0), px(46.0), px(100.0)),
            px(46.0)
        );
        assert_eq!(
            compute_visible_scroll_x(px(20.0), px(140.0), px(46.0), px(100.0)),
            px(20.0)
        );
        // 短い行に置換された場合は前回の横スクロール量が残っていても 0 に戻る
        assert_eq!(
            compute_visible_scroll_x(px(40.0), px(40.0), px(120.0), px(100.0)),
            px(0.0)
        );
    }
}
