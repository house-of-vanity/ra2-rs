use super::{Cell, State, assets::LegacyAssets, canvas::Pixels};
use bevy::prelude::*;
use ra2_formats::gib::Widget;

#[derive(Clone, Debug)]
pub struct Hit {
    pub name: String,
    pub owner: String,
    pub kind: String,
    pub rect: Rect,
    pub clip: Rect,
    pub editable: bool,
    pub max_chars: usize,
    pub horizontal: bool,
    pub reversed: bool,
}
pub struct Frame {
    pub pixels: Pixels,
    pub hits: Vec<Hit>,
    pub scroll_limits: std::collections::BTreeMap<String, usize>,
    pub regions: std::collections::BTreeMap<String, Rect>,
}
struct Layout<'a> {
    assets: &'a LegacyAssets,
    state: &'a State,
    frame: Frame,
}
fn color(w: &Widget, key: &str, default: [f32; 4]) -> [f32; 4] {
    w.numbers(key).try_into().unwrap_or(default)
}
fn metric(raw: &str, parent: f32) -> f32 {
    let values: Vec<f32> = raw
        .split_whitespace()
        .filter_map(|v| v.parse().ok())
        .collect();
    values.first().copied().unwrap_or(0.0)
        * if values.get(1) == Some(&1.0) {
            parent
        } else {
            1.0
        }
}
fn pair(w: &Widget, key: &str, parent: Vec2) -> Vec2 {
    let v = w.numbers(key);
    if v.len() != 4 {
        return Vec2::ZERO;
    }
    Vec2::new(
        v[0] * if v[1] == 1.0 { parent.x } else { 1.0 },
        v[2] * if v[3] == 1.0 { parent.y } else { 1.0 },
    )
}
fn bounds(w: &Widget, parent: Rect, forced: Option<Rect>) -> Rect {
    if let Some(r) = forced {
        return r;
    }
    let position = pair(w, "Position", parent.size());
    let mut size = pair(w, "Size", parent.size());
    let aspect = w.value("AspectRatio").parse::<f32>().unwrap_or(0.0);
    if aspect > 0.0 {
        match w.value("AspectSetting") {
            "1" => size.y = size.x * aspect,
            "2" => size.x = size.y / aspect,
            _ => (),
        }
    }
    let alignment = if w.kind == "TextBox" {
        0
    } else {
        w.value("Alignment").parse::<u32>().unwrap_or(0)
    };
    let anchor = Vec2::new(
        if alignment & 4 != 0 {
            1.0
        } else if alignment & 2 != 0 {
            0.5
        } else {
            0.0
        },
        if alignment & 32 != 0 {
            1.0
        } else if alignment & 16 != 0 {
            0.5
        } else {
            0.0
        },
    );
    Rect::from_corners(
        parent.min + position - size * anchor,
        parent.min + position + size * (Vec2::ONE - anchor),
    )
}
fn effective_bounds(w: &Widget, parent: Rect, forced: Option<Rect>) -> Rect {
    if !matches!(w.kind.as_str(), "DisplayBox" | "Scroller")
        && let Some(inner) = &w.wrapped
    {
        return effective_bounds(inner, parent, forced);
    }
    bounds(w, parent, forced)
}
impl Layout<'_> {
    fn text(
        &mut self,
        font: &str,
        text: &str,
        position: Vec2,
        tint: [f32; 4],
        clip: Rect,
        wrap: bool,
    ) {
        if let Some(font) = self
            .assets
            .fonts
            .get(font)
            .or_else(|| self.assets.fonts.get("fg_mcn14"))
        {
            self.frame
                .pixels
                .text(font, text, position, tint, clip, wrap);
        }
    }
    fn image(&mut self, path: &str, r: Rect, tint: [f32; 4], alpha: bool, clip: Rect) {
        if let Some(image) = self.assets.image(path) {
            self.frame.pixels.image(image, r, tint, alpha, clip);
        } else if path.is_empty() {
            self.frame.pixels.fill(r, tint, clip);
        }
    }
    fn rich_text(&mut self, text: &str, r: Rect, clip: Rect) {
        let mut pen = r.min;
        let mut remaining = text;
        while let Some(start) = remaining.find("<text") {
            remaining = &remaining[start..];
            let Some(end) = remaining.find('>') else {
                break;
            };
            let head = &remaining[..end];
            let font = head
                .split_whitespace()
                .skip_while(|s| *s != "font")
                .nth(1)
                .unwrap_or("fg_mcn14");
            remaining = &remaining[end + 1..];
            let end = remaining.find("</text>").unwrap_or(remaining.len());
            let part = &remaining[..end];
            let tint = head
                .split_whitespace()
                .skip_while(|s| *s != "color")
                .skip(1)
                .filter_map(|v| v.parse::<f32>().ok())
                .take(4)
                .collect::<Vec<_>>()
                .try_into()
                .unwrap_or([1.0; 4]);
            if let Some(f) = self.assets.fonts.get(font) {
                let origin = pen + Vec2::Y * f.baseline as f32;
                let size = self
                    .frame
                    .pixels
                    .text(f, part, origin, tint, clip.intersect(r), true);
                if part.ends_with("\\n") || part.ends_with('\n') {
                    pen.x = r.min.x;
                    pen.y += size.y;
                } else {
                    pen.x += size.x;
                }
            }
            remaining = remaining.get(end + 7..).unwrap_or("");
        }
        if !text.contains("<text") {
            self.text(
                "fg_mcn14",
                text,
                r.min + Vec2::Y * 12.0,
                [1.0; 4],
                clip.intersect(r),
                true,
            );
        }
    }
    fn cells(&mut self, w: &Widget, r: Rect, clip: Rect, cells: &[Cell]) {
        let name = w.name();
        let requested_scroll = self.state.scroll.get(name).copied().unwrap_or(0);
        let row_height = w.value("RowHeight").parse::<f32>().unwrap_or(28.0);
        let horizontal = w.kind == "Grid" && w.value("Rows") == "1";
        let column_width = if horizontal {
            cells
                .first()
                .and_then(|cell| self.assets.image(&cell.image))
                .map(|image| row_height * image.width as f32 / image.height as f32)
        } else {
            None
        }
        .unwrap_or_else(|| w.numbers("ColWidths").first().copied().unwrap_or(78.0));
        let columns = if horizontal {
            cells.len().max(1)
        } else {
            w.value("Columns").parse::<usize>().unwrap_or(1).max(1)
        };
        let visible = if horizontal {
            (r.width() / column_width).floor() as usize
        } else {
            (r.height() / row_height).floor() as usize
        };
        let length = if horizontal {
            cells.len()
        } else {
            cells.len().div_ceil(columns)
        };
        let limit = length.saturating_sub(visible.max(1));
        self.frame.scroll_limits.insert(name.into(), limit);
        let scroll = requested_scroll.min(limit);
        for (index, cell) in cells.iter().enumerate() {
            let x = if horizontal {
                (index as i32 - scroll as i32) as f32 * column_width
            } else {
                (index % columns) as f32 * column_width
            };
            let y = if horizontal {
                0.0
            } else {
                (index / columns) as f32 * row_height - scroll as f32 * row_height
            };
            let cr = Rect::from_corners(
                r.min + Vec2::new(x, y),
                r.min
                    + Vec2::new(
                        x + if horizontal || columns > 1 {
                            column_width
                        } else {
                            r.width()
                        },
                        y + row_height,
                    ),
            );
            if cr.intersect(r).is_empty() {
                continue;
            }
            let highlighted = self.state.hover == cell.key;
            let style_id = w.value(if cell.selected {
                "Selection"
            } else {
                "Highlight"
            });
            let style = w.children.iter().find(|c| c.value("ID") == style_id);
            let fill = style
                .map(|s| color(s, "FillColor", [0.3, 0.3, 1., 0.5]))
                .unwrap_or([0.3, 0.3, 1., 0.5]);
            if (cell.selected || highlighted) && style.is_none_or(|style| style.flag("Filled")) {
                self.frame.pixels.fill(cr, fill, clip.intersect(r));
            }
            if let Some(template) = &cell.template {
                self.walk(template, r, Some(cr), clip.intersect(r), name, None, 0);
            }
            if !cell.image.is_empty() {
                self.image(
                    &cell.image,
                    cr,
                    if cell.selected {
                        [1.0; 4]
                    } else {
                        [1.0, 1.0, 1.0, 0.5]
                    },
                    true,
                    clip.intersect(r),
                );
            }
            if !cell.label.is_empty() {
                self.text(
                    "fg_mcn14",
                    &cell.label,
                    cr.min + Vec2::new(3.0, 14.0),
                    [1.0; 4],
                    clip.intersect(cr).intersect(r),
                    true,
                );
            }
            let border = if cell.selected || highlighted {
                style
            } else {
                None
            };
            let width = border
                .map(|s| s.value("BorderWidth"))
                .unwrap_or(w.value("BorderWidth"))
                .parse::<f32>()
                .unwrap_or(0.);
            if width > 0. {
                self.frame.pixels.border(
                    cr,
                    width,
                    border
                        .map(|s| color(s, "BorderColor", [1.; 4]))
                        .unwrap_or(color(w, "BorderColor", [0.1, 0.1, 0.1, 1.])),
                    clip.intersect(r),
                );
            }
            self.frame.hits.push(Hit {
                name: cell.key.clone(),
                owner: name.into(),
                kind: "Cell".into(),
                rect: cr,
                clip: clip.intersect(r),
                editable: false,
                max_chars: 0,
                horizontal: true,
                reversed: false,
            });
        }
    }
    #[allow(
        clippy::too_many_arguments,
        reason = "Inherited layout, clipping and widget decoration context"
    )]
    fn walk(
        &mut self,
        w: &Widget,
        parent: Rect,
        forced: Option<Rect>,
        clip: Rect,
        owner: &str,
        text_override: Option<&str>,
        depth: usize,
    ) -> Rect {
        let mut wrapped = Some(w);
        while let Some(widget) = wrapped {
            if self.state.visibility.get(widget.name()) == Some(&false) {
                return Rect::default();
            }
            wrapped = widget.wrapped.as_deref();
        }
        if depth > 128 {
            return Rect::default();
        }
        let r = effective_bounds(w, parent, forced);
        // Source mode 2 lets decorations (captions and side wires) escape the parent clip.
        let clip = if w.value("ClipMode") == "2" {
            self.frame.pixels.bounds()
        } else {
            clip
        };
        if !w.name().is_empty() {
            self.frame.regions.insert(w.name().into(), r);
        }
        if let Some(inner) = &w.wrapped {
            let scroll = if w.kind == "Scroller" {
                self.state.cells.get(inner.name()).map(|cells| {
                    let rect = r;
                    let rows = inner.value("Rows");
                    let row_height = inner.value("RowHeight").parse::<f32>().unwrap_or(28.);
                    let horizontal = inner.kind == "Grid" && rows == "1";
                    let width = cells
                        .first()
                        .and_then(|c| self.assets.image(&c.image))
                        .map(|i| row_height * i.width as f32 / i.height as f32)
                        .unwrap_or_else(|| {
                            inner.numbers("ColWidths").first().copied().unwrap_or(78.)
                        });
                    let columns = inner.value("Columns").parse::<usize>().unwrap_or(1).max(1);
                    let needed = inner.flag("PermScrolls")
                        || if horizontal {
                            cells.len() as f32 * width > rect.width()
                        } else {
                            cells.len().div_ceil(columns) as f32 * row_height > rect.height()
                        };
                    let expanded = Rect::from_corners(
                        r.min,
                        r.max
                            - if needed {
                                if horizontal {
                                    Vec2::Y * 16.
                                } else {
                                    Vec2::X * 16.
                                }
                            } else {
                                Vec2::ZERO
                            },
                    );
                    (needed, expanded, horizontal)
                })
            } else {
                None
            };
            if w.kind == "DisplayBox" {
                let value = self
                    .state
                    .values
                    .get(w.name())
                    .map(String::as_str)
                    .unwrap_or(w.value("Display"));
                self.rich_text(value, r, clip);
                return r;
            }
            if w.kind == "Background" {
                for child in &w.children {
                    self.walk(child, r, Some(r), clip, owner, None, depth + 1);
                }
            }
            if w.kind == "Scroller" {
                self.walk(
                    inner,
                    r,
                    scroll.map(|(_, rect, _)| rect),
                    clip,
                    inner.name(),
                    text_override,
                    depth + 1,
                );
            } else {
                self.walk(inner, parent, forced, clip, owner, text_override, depth + 1);
            }
            if w.kind == "Border" {
                let width = w.value("Width").parse::<f32>().unwrap_or(1.0);
                self.frame
                    .pixels
                    .border(r, width, color(w, "Outer", [0.6, 0.6, 0.6, 1.0]), clip);
                self.frame.pixels.border(
                    Rect::from_corners(r.min + Vec2::splat(width), r.max - Vec2::splat(width)),
                    width,
                    color(w, "Inner", [1.0; 4]),
                    clip,
                );
            }
            if w.kind != "Background" {
                for child in &w.children {
                    if child.kind == "ScrollBar" && scroll.is_some_and(|(needed, _, _)| !needed) {
                        continue;
                    }
                    let bar_rect = (child.kind == "ScrollBar").then(|| {
                        if scroll.is_some_and(|(_, _, horizontal)| horizontal) {
                            Rect::from_corners(Vec2::new(r.min.x, r.max.y - 16.), r.max)
                        } else {
                            Rect::from_corners(Vec2::new(r.max.x - 16., r.min.y), r.max)
                        }
                    });
                    self.walk(
                        child,
                        r,
                        bar_rect,
                        clip,
                        if w.kind == "Scroller" {
                            inner.name()
                        } else {
                            owner
                        },
                        None,
                        depth + 1,
                    );
                }
            }
            return r;
        }
        let clip = if (w.value("ClipMode") == "1" || w.kind == "TextBox")
            && w.name() != "component mod group"
            && r.width() > 0.0
            && r.height() > 0.0
        {
            clip.intersect(r)
        } else {
            clip
        };
        let owned = matches!(
            w.kind.as_str(),
            "TextBox" | "Slider" | "DropDown" | "Grid" | "ListBox"
        );
        let owner = if owned && !w.name().is_empty() {
            w.name()
        } else {
            owner
        };
        let override_text = self
            .state
            .values
            .get(w.name())
            .map(String::as_str)
            .or(text_override);
        if [
            "Button",
            "CheckBox",
            "TextBox",
            "Slider",
            "DropDown",
            "Grid",
            "ListBox",
            "Placeholder",
        ]
        .contains(&w.kind.as_str())
            || self.state.interactive.contains(w.name())
        {
            let synthetic =
                (w.kind == "Slider" && w.name().is_empty() && self.state.cells.contains_key(owner))
                    .then(|| format!("scroll:{owner}"));
            let capture = (!w.name().is_empty() || synthetic.is_some())
                && !self.state.disabled.contains(w.name())
                && (w.kind != "TextBox"
                    || w.flag("Editable")
                    || self.state.interactive.contains(w.name()));
            if capture {
                self.frame.hits.push(Hit {
                    name: synthetic.unwrap_or_else(|| w.name().into()),
                    owner: owner.into(),
                    kind: w.kind.clone(),
                    rect: r,
                    clip,
                    editable: w.flag("Editable"),
                    max_chars: w.value("MaxCharacters").parse().unwrap_or(0),
                    horizontal: w.flag("Horiz"),
                    reversed: {
                        let from = pair(w, "From", r.size());
                        let to = pair(w, "To", r.size());
                        if w.flag("Horiz") {
                            from.x > to.x
                        } else {
                            from.y > to.y
                        }
                    },
                });
            }
        }
        match w.kind.as_str() {
            "Image" => self.image(
                self.state
                    .images
                    .get(w.name())
                    .map(String::as_str)
                    .unwrap_or(w.value("_Filename")),
                r,
                self.state
                    .colors
                    .get(w.name())
                    .copied()
                    .unwrap_or_else(|| color(w, "_ForeColor", [1.0; 4])),
                w.flag("_Alpha"),
                clip,
            ),
            "Rectangle" => {
                if w.flag("Filled") {
                    self.frame
                        .pixels
                        .fill(r, color(w, "FillColor", [1.0; 4]), clip);
                }
                let width = w.value("BorderWidth").parse().unwrap_or(0.0);
                if width > 0.0 {
                    self.frame
                        .pixels
                        .border(r, width, color(w, "BorderColor", [1.0; 4]), clip);
                }
            }
            "Text" => self.text(
                w.value("_Font"),
                override_text.unwrap_or(w.value("_Text")),
                r.min,
                color(w, "_ForeColor", [1.0; 4]),
                clip,
                false,
            ),
            "Placeholder" => {
                if self.state.viewports.contains(w.name()) {
                    self.frame.pixels.clear(r.intersect(clip));
                }
            }
            "Proxy" => {
                if let Ok(proxy) = self.assets.widget(w.value("Filename")) {
                    self.walk(&proxy, r, None, clip, owner, None, depth + 1);
                }
            }
            _ => (),
        }
        if let Some(cells) = self.state.cells.get(w.name()) {
            self.cells(w, r, clip, cells);
            return r;
        }
        let mut children: Vec<_> = w.children.iter().enumerate().collect();
        children.sort_by_key(|(_, child)| child.value("ChildOrder").parse::<i32>().unwrap_or(0));
        let active_state = if self.state.disabled.contains(w.name()) {
            3
        } else if self
            .state
            .checked
            .get(w.name())
            .copied()
            .unwrap_or(w.flag("Checked"))
            || self.state.pressed == w.name()
        {
            2
        } else if self.state.hover == w.name() {
            1
        } else {
            0
        };
        let image_child = w.fields.get("StateImage").and_then(|states| {
            states.iter().find_map(|v| {
                let parts: Vec<_> = v.split_whitespace().collect();
                if parts.first().and_then(|s| s.parse::<i32>().ok()) == Some(active_state)
                    && parts.get(1) == Some(&"true")
                {
                    parts.get(2)?.parse::<usize>().ok()
                } else {
                    None
                }
            })
        });
        for (index, child) in children {
            if w.fields.contains_key("StateImage") {
                if Some(index) != image_child {
                    continue;
                }
            } else if std::iter::successors(Some(child), |w| w.wrapped.as_deref())
                .any(|w| w.value("Visible") == "false")
                && !std::iter::successors(Some(child), |w| w.wrapped.as_deref())
                    .any(|w| self.state.visibility.get(w.name()) == Some(&true))
                && w.kind != "ScrollBar"
            {
                continue;
            }
            if w.kind == "TextBox" && w.flag("MultiLine") && child.kind == "Text" {
                let baseline = self
                    .assets
                    .fonts
                    .get(child.value("_Font"))
                    .map_or(12, |f| f.baseline) as f32;
                self.text(
                    child.value("_Font"),
                    override_text.unwrap_or(child.value("_Text")),
                    r.min + Vec2::Y * baseline,
                    color(child, "_ForeColor", [1.0; 4]),
                    clip.intersect(r),
                    true,
                );
                continue;
            }
            let mut child_forced = None;
            if w.kind == "TextBox"
                && child.kind == "Text"
                && let Some(font) = self.assets.fonts.get(child.value("_Font"))
            {
                let text = override_text.unwrap_or(child.value("_Text"));
                let width = text
                    .chars()
                    .filter_map(|c| font.glyphs.get(&c))
                    .map(|g| g.advance)
                    .sum::<i32>() as f32;
                let align = w.value("Alignment").parse::<u32>().unwrap_or(9);
                let x = if align & 4 != 0 {
                    r.width() - width
                } else if align & 2 != 0 {
                    (r.width() - width) * 0.5
                } else {
                    0.0
                };
                let y = if align & 32 != 0 {
                    r.height() - font.line_height as f32
                } else if align & 16 != 0 {
                    (r.height() - font.line_height as f32) * 0.5
                } else {
                    0.0
                };
                let origin = r.min + Vec2::new(x, y + font.baseline as f32);
                child_forced = Some(Rect::from_corners(origin, origin));
            }
            if w.kind == "DropDown" && child.value("ID") == w.value("DecoratedList") {
                continue;
            }

            if w.kind == "SplitFrame" && w.children.len() == 2 {
                let horizontal = w.flag("Horizontal");
                let master = w.value("Master").parse::<usize>().unwrap_or(0);
                let extent = metric(
                    w.value("MasterSize"),
                    if horizontal { r.width() } else { r.height() },
                );
                let first = if master == 0 {
                    extent
                } else {
                    (if horizontal { r.width() } else { r.height() }) - extent
                };
                let cross = effective_bounds(child, r, None);
                child_forced = Some(if horizontal {
                    Rect::from_corners(
                        Vec2::new(
                            if index == 0 { r.min.x } else { r.min.x + first },
                            cross.min.y,
                        ),
                        Vec2::new(
                            if index == 0 { r.min.x + first } else { r.max.x },
                            cross.max.y,
                        ),
                    )
                } else {
                    Rect::from_corners(
                        Vec2::new(
                            cross.min.x,
                            if index == 0 { r.min.y } else { r.min.y + first },
                        ),
                        Vec2::new(
                            cross.max.x,
                            if index == 0 { r.min.y + first } else { r.max.y },
                        ),
                    )
                });
            }
            if w.kind == "ScrollBar" && w.children.len() >= 3 {
                let horizontal = w.value("Orientation") == "0";
                let axis = if horizontal { Vec2::X } else { Vec2::Y };
                let extent = 16.0;
                child_forced = Some(match index {
                    0 => Rect::from_corners(
                        r.min,
                        if horizontal {
                            Vec2::new(r.min.x + extent, r.max.y)
                        } else {
                            Vec2::new(r.max.x, r.min.y + extent)
                        },
                    ),
                    1 => Rect::from_corners(
                        if horizontal {
                            Vec2::new(r.max.x - extent, r.min.y)
                        } else {
                            Vec2::new(r.min.x, r.max.y - extent)
                        },
                        r.max,
                    ),
                    _ => Rect::from_corners(r.min + axis * extent, r.max - axis * extent),
                });
            }
            if w.kind == "Slider" && index == 0 {
                let mut cr = bounds(child, r, None);
                let scroll_ratio = (w.name().is_empty() && self.state.cells.contains_key(owner))
                    .then(|| {
                        let limit = self.frame.scroll_limits.get(owner).copied().unwrap_or(0);
                        self.state
                            .scroll
                            .get(owner)
                            .copied()
                            .unwrap_or(0)
                            .min(limit) as f32
                            / limit.max(1) as f32
                    });
                let mut value = scroll_ratio
                    .or_else(|| {
                        self.state
                            .values
                            .get(w.name())
                            .and_then(|s| s.parse::<f32>().ok())
                    })
                    .unwrap_or(0.0)
                    .clamp(0.0, 1.0);
                let from = pair(w, "From", r.size());
                let to = pair(w, "To", r.size());
                if if w.flag("Horiz") {
                    from.x > to.x
                } else {
                    from.y > to.y
                } {
                    value = 1.0 - value;
                }
                let offset = if w.flag("Horiz") {
                    Vec2::X * (r.width() - cr.width()) * value
                } else {
                    Vec2::Y * (r.height() - cr.height()) * value
                };
                cr.min += offset;
                cr.max += offset;
                child_forced = Some(cr);
            }
            self.walk(
                child,
                r,
                child_forced,
                clip,
                owner,
                override_text,
                depth + 1,
            );
        }
        if let Some(slot) = self.state.slots.get(w.name()) {
            self.walk(slot, r, None, clip, owner, None, depth + 1);
        }
        r
    }
}
pub fn compose(assets: &LegacyAssets, state: &State, width: usize, height: usize) -> Frame {
    let frame = Frame {
        pixels: Pixels::blank(width, height),
        hits: vec![],
        scroll_limits: Default::default(),
        regions: Default::default(),
    };
    let mut layout = Layout {
        assets,
        state,
        frame,
    };
    let bounds = layout.frame.pixels.bounds();
    if let Some(root) = &state.root {
        layout.walk(root, bounds, Some(bounds), bounds, "", None, 0);
    }
    for (region, widget) in &state.attachments {
        if let Some(parent) = layout.frame.regions.get(region).copied() {
            layout.walk(widget, parent, None, bounds, "", None, 0);
        }
    }
    if !state.dialogs.is_empty() {
        layout.frame.hits.clear();
    }
    for dialog in &state.dialogs {
        layout.walk(dialog, bounds, None, bounds, "", None, 0);
    }
    if let Some(name) = &state.popup
        && let Some(r) = layout.frame.regions.get(name).copied()
        && let Some(items) = state.choices.get(name)
    {
        let height = 18.0 * items.len() as f32;
        let origin = Vec2::new(
            r.min.x,
            if r.max.y + height <= bounds.max.y {
                r.max.y
            } else {
                (r.min.y - height).max(0.0)
            },
        );
        for (index, item) in items.iter().enumerate() {
            let row = Rect::from_corners(
                origin + Vec2::Y * (index as f32 * 18.0),
                origin + Vec2::new(r.width(), (index + 1) as f32 * 18.0),
            );
            let key = format!("choice:{name}:{index}");
            layout.frame.pixels.fill(
                row,
                if state.hover == key {
                    [0.25, 0.2, 0.6, 1.0]
                } else {
                    [0.1, 0.1, 0.1, 1.0]
                },
                bounds,
            );
            layout.text(
                "fg_mcn14",
                item,
                row.min + Vec2::new(2.0, 13.0),
                [1.0; 4],
                row,
                false,
            );
            layout.frame.hits.push(Hit {
                name: key,
                owner: name.clone(),
                kind: "Choice".into(),
                rect: row,
                clip: bounds,
                editable: false,
                max_chars: 0,
                horizontal: true,
                reversed: false,
            });
        }
    }
    if let Some((image, point)) = &state.control_drag {
        if let Some(target) = layout.frame.hits.iter().rev().find(|hit| {
            hit.name.starts_with("control:")
                && hit.rect.contains(*point)
                && hit.clip.contains(*point)
        }) {
            layout
                .frame
                .pixels
                .border(target.rect, 2.0, [0.3, 1.0, 0.3, 1.0], target.clip);
        }
        let size = assets
            .image(image)
            .map(|image| Vec2::new(image.width as f32, image.height as f32))
            .unwrap_or(Vec2::splat(32.0));
        let size = size * (40.0 / size.max_element()).min(1.0);
        layout.image(
            image,
            Rect::from_center_size(*point, size),
            [1.0, 1.0, 1.0, 0.8],
            true,
            bounds,
        );
    }
    layout.frame
}

#[cfg(test)]
mod visibility_tests {
    use super::*;
    #[test]
    fn scroller_hides_unneeded_bar_and_clamps_rows_and_thumb_to_last_page() {
        fn widget(kind: &str, fields: &[(&str, &str)], children: Vec<Widget>) -> Widget {
            Widget {
                kind: kind.into(),
                fields: fields
                    .iter()
                    .map(|(k, v)| (k.to_string(), vec![v.to_string()]))
                    .collect(),
                wrapped: None,
                children,
            }
        }
        let thumb = widget("Image", &[("Name", "thumb"), ("Size", "16 0 10 0")], vec![]);
        let slider = widget(
            "Slider",
            &[("Size", "16 0 60 0"), ("Horiz", "false")],
            vec![thumb],
        );
        let bar = widget(
            "ScrollBar",
            &[("Position", "84 0 0 0"), ("Size", "16 0 60 0")],
            vec![slider],
        );
        let list = widget(
            "ListBox",
            &[
                ("Name", "items"),
                ("Size", "84 0 60 0"),
                ("RowHeight", "20"),
            ],
            vec![],
        );
        let mut root = widget("Scroller", &[("Size", "100 0 60 0")], vec![bar]);
        root.wrapped = Some(Box::new(list));
        let mut state = State {
            root: Some(std::sync::Arc::new(root)),
            ..Default::default()
        };
        state.cells.insert(
            "items".into(),
            (0..6)
                .map(|i| Cell {
                    key: format!("item:{i}"),
                    ..Default::default()
                })
                .collect(),
        );
        state.scroll.insert("items".into(), 999);
        let assets = LegacyAssets::default();
        let frame = compose(&assets, &state, 100, 60);
        assert_eq!(frame.scroll_limits["items"], 3);
        assert!(frame.hits.iter().any(|h| h.name == "scroll:items"));
        assert!(
            frame
                .hits
                .iter()
                .any(|h| h.name == "item:5" && h.rect.max.y == 60.)
        );
        assert_eq!(frame.regions["thumb"].min.y, 50.);
        state.cells.get_mut("items").unwrap().truncate(2);
        let frame = compose(&assets, &state, 100, 60);
        assert_eq!(frame.scroll_limits["items"], 0);
        assert!(!frame.hits.iter().any(|h| h.name == "scroll:items"));
        assert_eq!(
            frame
                .hits
                .iter()
                .find(|h| h.name == "item:0")
                .unwrap()
                .rect
                .width(),
            100.
        );
    }
    #[test]
    fn hiding_wrapped_button_also_hides_border_and_caption() {
        let button = Widget {
            kind: "Button".into(),
            fields: [
                ("Name".into(), vec!["cancel button".into()]),
                ("Position".into(), vec!["2 0 2 0".into()]),
                ("Size".into(), vec!["20 0 16 0".into()]),
            ]
            .into(),
            wrapped: None,
            children: vec![],
        };
        let border = Widget {
            kind: "Border".into(),
            fields: [("Width".into(), vec!["2".into()])].into(),
            wrapped: Some(Box::new(button)),
            children: vec![],
        };
        let assets = LegacyAssets::default();
        let mut state = State {
            root: Some(std::sync::Arc::new(border)),
            ..Default::default()
        };
        let visible = compose(&assets, &state, 32, 24);
        assert!(!visible.hits.is_empty());
        assert!(visible.pixels.rgba.iter().any(|v| *v != 0));
        state.visibility.insert("cancel button".into(), false);
        let hidden = compose(&assets, &state, 32, 24);
        assert!(hidden.hits.is_empty());
        assert!(hidden.pixels.rgba.iter().all(|v| *v == 0));
    }
}
