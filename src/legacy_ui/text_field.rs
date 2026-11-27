//! Use the same bitmap glyph advances for painting, pointer hits and selection.
use super::canvas::Pixels;
use bevy::prelude::*;
use ra2_formats::giff::BitmapFont;
use std::{collections::VecDeque, ops::Range, time::Duration};
use unicode_segmentation::UnicodeSegmentation;

#[derive(Clone, Debug, Default)]
pub struct Editing {
    pub field: String,
    pub head: usize,
    pub anchor: usize,
    pub scroll: Vec2,
    pub blink_start: Duration,
    pub caret_visible: bool,
    undo: VecDeque<(String, usize, usize)>,
    redo: Vec<(String, usize, usize)>,
}

impl Editing {
    pub fn focus(&mut self, field: &str, value: &str) {
        if self.field != field {
            *self = Self {
                field: field.into(),
                head: value.len(),
                anchor: value.len(),
                ..default()
            };
        }
        self.head = value.floor_char_boundary(self.head.min(value.len()));
        self.anchor = value.floor_char_boundary(self.anchor.min(value.len()));
    }

    pub fn select_to(&mut self, position: usize, extend: bool) {
        self.head = position;
        if !extend {
            self.anchor = position;
        }
    }

    pub fn selection(&self) -> Range<usize> {
        self.anchor.min(self.head)..self.anchor.max(self.head)
    }

    pub fn replace(&mut self, text: &mut String, input: &str, limit: usize, multiline: bool) {
        let range = self.selection();
        let used = text.chars().count() - text[range.clone()].chars().count();
        let normalized = input.replace("\r\n", "\n");
        let insert: String = normalized
            .chars()
            .map(|c| {
                if !multiline && matches!(c, '\r' | '\n') {
                    ' '
                } else {
                    c
                }
            })
            .filter(|&c| !c.is_control() || multiline && c == '\n')
            .take(limit.saturating_sub(used))
            .collect();
        if insert.is_empty() && !input.is_empty() {
            return;
        }
        if range.is_empty() && insert.is_empty() {
            return;
        }
        if self.undo.len() == 64 {
            self.undo.pop_front();
        }
        self.undo.push_back((text.clone(), self.anchor, self.head));
        self.redo.clear();
        text.replace_range(range.clone(), &insert);
        self.select_to(range.start + insert.len(), false);
    }

    pub fn horizontal(&mut self, text: &str, right: bool, shift: bool, word: bool) {
        let range = self.selection();
        let position = if !shift && !range.is_empty() {
            if right { range.end } else { range.start }
        } else if word && right {
            text[self.head..]
                .split_word_bound_indices()
                .skip(1)
                .find(|(_, s)| !s.trim().is_empty())
                .map_or(text.len(), |(offset, _)| self.head + offset)
        } else if word {
            text[..self.head]
                .split_word_bound_indices()
                .rev()
                .find(|(_, s)| !s.trim().is_empty())
                .map_or(0, |(offset, _)| offset)
        } else if right {
            self.head + text[self.head..].graphemes(true).next().map_or(0, str::len)
        } else {
            text[..self.head]
                .grapheme_indices(true)
                .next_back()
                .map_or(0, |(i, _)| i)
        };
        self.select_to(position, shift);
    }

    pub fn restore(&mut self, text: &mut String, redo: bool) {
        let last = if redo {
            self.redo.pop()
        } else {
            self.undo.pop_back()
        };
        if let Some((value, anchor, head)) = last {
            let current = (std::mem::replace(text, value), self.anchor, self.head);
            if redo {
                self.undo.push_back(current);
            } else {
                self.redo.push(current);
            }
            self.anchor = anchor;
            self.head = head;
        }
    }

    pub fn wake(&mut self, now: Duration) {
        self.blink_start = now;
        self.caret_visible = true;
    }
}

#[derive(Clone, Debug)]
struct Row {
    bytes: Range<usize>,
    stops: Vec<(usize, f32)>,
    origin: Vec2,
}

impl Row {
    fn x(&self, byte: usize) -> f32 {
        let i = self
            .stops
            .partition_point(|(b, _)| *b < byte)
            .min(self.stops.len() - 1);
        self.origin.x + self.stops[i].1
    }
    fn hit(&self, x: f32) -> usize {
        let x = x - self.origin.x;
        for w in self.stops.windows(2) {
            if x < (w[0].1 + w[1].1) * 0.5 {
                return w[0].0;
            }
        }
        self.bytes.end
    }
}

#[derive(Clone, Debug)]
pub struct FieldLayout {
    pub value: String,
    pub multiline: bool,
    pub allow_newlines: bool,
    pub scroll: Vec2,
    pub clip: Rect,
    rows: Vec<Row>,
    ascent: f32,
    descent: f32,
}

fn wrap_lines(text: &str, font: &BitmapFont, width: f32, multi: bool) -> Vec<Range<usize>> {
    if !multi {
        return vec![0..text.len()];
    }
    let mut lines = Vec::new();
    let mut start = 0;
    for paragraph in text.split('\n') {
        let end = start + paragraph.len();
        while start < end {
            let mut width_used = 0.;
            let mut word_break = start;
            let mut split = end;
            for (offset, cluster) in text[start..end].grapheme_indices(true) {
                let next = width_used + font.measure(cluster);
                if offset > 0 && next > width {
                    split = if word_break > start {
                        word_break
                    } else {
                        start + offset
                    };
                    break;
                }
                width_used = next;
                if cluster.chars().all(char::is_whitespace) {
                    word_break = start + offset + cluster.len();
                }
            }
            lines.push(start..split);
            start = split;
        }
        if paragraph.is_empty() {
            lines.push(start..end);
        }
        start = end + 1;
    }
    lines
}

pub fn aligned(start: f32, available: f32, used: f32, center: bool, end: bool) -> f32 {
    (start
        + if end {
            available - used
        } else if center {
            (available - used) * 0.5
        } else {
            0.
        })
    .round()
}

pub fn word_at(text: &str, index: usize) -> (usize, usize) {
    text.split_word_bound_indices()
        .find(|(start, part)| index < start + part.len())
        .or_else(|| text.split_word_bound_indices().next_back())
        .map_or((0, 0), |(i, part)| (i, i + part.len()))
}

pub fn fit_label<'a>(font: &BitmapFont, text: &'a str, width: f32) -> std::borrow::Cow<'a, str> {
    if font.measure(text) <= width {
        return text.into();
    }
    let dots = if font.measure("...") <= width {
        "..."
    } else {
        ""
    };
    let mut used = font.measure(dots);
    let end = text
        .char_indices()
        .find_map(|(i, c)| {
            used += font.advance(c);
            (used > width).then_some(i)
        })
        .unwrap_or(text.len());
    format!("{}{dots}", &text[..end]).into()
}

impl FieldLayout {
    pub fn new(
        font: &BitmapFont,
        text: &str,
        rect: Rect,
        align: u32,
        multiline: bool,
        edit: Option<&Editing>,
    ) -> Self {
        let ascent = font.ascent as f32;
        let descent = font.descent as f32;
        let ink = ascent + descent;
        let padding = Vec2::new(
            3.0_f32.min(rect.width() * 0.25),
            ((rect.height() - ink) * 0.5).clamp(0., 2.),
        );
        let clip = Rect::from_corners((rect.min + padding).ceil(), (rect.max - padding).floor());
        let width = (clip.width() - 1.).max(1.);
        let line_height = font.line_height as f32;
        let ranges = wrap_lines(text, font, width, multiline);
        let height = ink + (ranges.len() - 1) as f32 * line_height;
        let top = if multiline {
            clip.min.y
        } else {
            aligned(
                clip.min.y,
                clip.height(),
                ink,
                align & 16 != 0,
                align & 32 != 0,
            )
        };
        let mut rows = Vec::with_capacity(ranges.len());
        for (index, bytes) in ranges.into_iter().enumerate() {
            let mut stops = vec![(bytes.start, 0.)];
            let mut advance = 0.;
            for (i, cluster) in text[bytes.clone()].grapheme_indices(true) {
                advance += font.measure(cluster);
                stops.push((bytes.start + i + cluster.len(), advance));
            }
            let x = if advance >= width {
                clip.min.x
            } else {
                aligned(clip.min.x, width, advance, align & 2 != 0, align & 4 != 0)
            };
            rows.push(Row {
                bytes,
                stops,
                origin: Vec2::new(x, top + ascent + index as f32 * line_height),
            });
        }
        let mut view = Self {
            value: text.into(),
            multiline,
            allow_newlines: multiline,
            rows,
            ascent,
            descent,
            clip,
            scroll: Vec2::ZERO,
        };
        if let Some(edit) = edit {
            let row = &view.rows[view.row_for(edit.head)];
            let x = row.x(edit.head);
            let y = row.origin.y - ascent;
            view.scroll.x = edit
                .scroll
                .x
                .clamp((x - clip.min.x - width).max(0.), (x - clip.min.x).max(0.))
                .min((row.x(row.bytes.end) - clip.min.x - width).max(0.));
            view.scroll.y = edit
                .scroll
                .y
                .clamp(
                    (y + ink - clip.max.y).max(0.),
                    (y - clip.min.y).max((y + ink - clip.max.y).max(0.)),
                )
                .min((height - clip.height()).max(0.));
            for row in &mut view.rows {
                row.origin -= view.scroll;
            }
        }
        view
    }

    fn row_for(&self, byte: usize) -> usize {
        self.rows
            .partition_point(|row| row.bytes.start <= byte)
            .saturating_sub(1)
    }
    pub fn point(&self, byte: usize) -> Vec2 {
        let row = &self.rows[self.row_for(byte)];
        Vec2::new(row.x(byte), row.origin.y)
    }
    pub fn pick(&self, point: Vec2) -> usize {
        let i = self
            .rows
            .windows(2)
            .take_while(|r| {
                point.y >= (r[0].origin.y + self.descent + r[1].origin.y - self.ascent) * 0.5
            })
            .count();
        self.rows[i].hit(point.x)
    }
    pub fn line_edge(&self, byte: usize, end: bool) -> usize {
        let row = &self.rows[self.row_for(byte)];
        if end { row.bytes.end } else { row.bytes.start }
    }
    pub fn vertical(&self, byte: usize, down: bool) -> usize {
        let i = self.row_for(byte);
        let next = if down {
            (i + 1).min(self.rows.len() - 1)
        } else {
            i.saturating_sub(1)
        };
        self.rows[next].hit(self.rows[i].x(byte))
    }
    pub fn draw(
        &self,
        pixels: &mut Pixels,
        font: &BitmapFont,
        color: [f32; 4],
        parent: Rect,
        edit: Option<&Editing>,
    ) {
        let clip = self.clip.intersect(parent);
        for row in &self.rows {
            if row.origin.y + self.descent <= clip.min.y || row.origin.y - self.ascent >= clip.max.y
            {
                continue;
            }
            if let Some(edit) = edit {
                let selection = edit.selection();
                let left = selection.start.max(row.bytes.start);
                let right = selection.end.min(row.bytes.end);
                if left < right {
                    pixels.fill(
                        Rect::new(
                            row.x(left),
                            row.origin.y - self.ascent,
                            row.x(right),
                            row.origin.y + self.descent,
                        ),
                        [0.19, 0.39, 0.66, 0.9],
                        clip,
                    );
                }
            }
            pixels.text_line(
                font,
                &self.value[row.bytes.clone()],
                row.origin,
                color,
                clip,
            );
        }
        if let Some(edit) = edit.filter(|e| e.caret_visible) {
            let point = self.point(edit.head);
            pixels.fill(
                Rect::new(
                    point.x,
                    point.y - self.ascent,
                    point.x + 1.,
                    point.y + self.descent,
                ),
                color,
                clip,
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ra2_formats::giff::Glyph;

    fn font() -> BitmapFont {
        let glyph = |advance| Glyph {
            uv: [0., 0., 1., 1.],
            advance,
            left: 0,
            top: 2,
        };
        BitmapFont {
            line_height: 8,
            baseline: 3,
            ascent: 3,
            descent: 2,
            space: 2,
            width: 1,
            height: 1,
            rgba: vec![255; 4],
            glyphs: [
                ('?', glyph(4)),
                ('i', glyph(2)),
                ('W', glyph(7)),
                (' ', glyph(2)),
                ('.', glyph(1)),
            ]
            .into(),
        }
    }

    #[test]
    fn pasted_text_replaces_the_selection_and_undo_restores_it() {
        let mut edit = Editing::default();
        let mut text = String::from("Бот 34");
        edit.focus("name", &text);
        edit.horizontal(&text, false, true, true);
        edit.replace(&mut text, "Ёж", 6, false);
        assert_eq!(text, "Бот Ёж");
        assert_eq!(edit.head, text.len());
        edit.restore(&mut text, false);
        assert_eq!(&text[edit.selection()], "34");
        edit.restore(&mut text, true);
        assert_eq!(text, "Бот Ёж");
        edit.anchor = 0;
        edit.replace(&mut text, "е\u{301}ж", 16, false);
        edit.select_to(0, false);
        edit.horizontal(&text, true, true, false);
        assert_eq!(&text[edit.selection()], "е\u{301}");
        edit.replace(&mut text, "", 16, false);
        assert_eq!(text, "ж");
        edit.replace(&mut text, "12\r\n34", 5, false);
        assert_eq!(text, "12 3ж");
    }

    #[test]
    fn pointer_and_caret_share_glyph_stops_through_scrolling_and_wrapping() {
        let f = font();
        let rect = Rect::new(3., 6., 30., 22.);
        let mut edit = Editing::default();
        let text = "WiWiWiWi";
        edit.focus("test", text);
        let field = FieldLayout::new(&f, text, rect, 17, false, Some(&edit));
        assert!(field.scroll.x > 0. && field.clip.contains(field.point(edit.head)));
        for pos in [4, 5, 6, 7, 8] {
            assert_eq!(field.pick(field.point(pos)), pos);
        }
        edit.scroll = field.scroll;
        edit.select_to(0, false);
        let field = FieldLayout::new(&f, text, rect, 17, false, Some(&edit));
        assert_eq!(field.scroll, Vec2::ZERO);
        let caret = field.point(0);
        assert_eq!(field.pick(caret + Vec2::X * 5.), 1);
        assert_eq!(field.pick(caret + Vec2::X * 9.), 2);
        let wrapped = "WWW WWW\nii WW";
        edit.focus("motto", wrapped);
        let field = FieldLayout::new(&f, wrapped, rect, 9, true, Some(&edit));
        assert!(field.scroll.y > 0. && field.clip.contains(field.point(edit.head)));
        assert_eq!(field.pick(field.point(edit.head)), wrapped.len());
        assert!(field.vertical(edit.head, false) < edit.head);
        assert_eq!(field.line_edge(edit.head, true), wrapped.len());
        assert_eq!(fit_label(&f, "WWiWi", 19.), "WWi...");
    }
}
