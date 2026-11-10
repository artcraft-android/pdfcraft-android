use std::sync::Arc;

use super::fallback::{same_font, same_paint};
use super::interp::{Carry, Origin, Pen, Shown, TextState, Ts, direction, interpret, to_frame, union};
use super::{DecodeBudget, Dict, Document, EditError, HashMap, Matrix, Object, Op, Pieces, Stream, TextLine, parse, serialize_ops};

impl TextLine {
    /// The baseline's height in user space.
    pub fn origin_baseline(&self) -> f64 {
        self.origin.baseline
    }

    /// Paint position of the line's first operator: `(content-stream index, operator index in
    /// the page's joined stream)`. A line joins runs painted at different times, so anything
    /// painted after this position may lie on top of part of it.
    /// (Lines from `text_lines` never come from form XObjects, so `stream` is always a
    /// page content-stream index and `ops` are indices into the joined operator list.)
    pub fn first_paint_pos(&self) -> (usize, usize) {
        (self.stream, self.ops.iter().min().copied().unwrap_or(0))
    }

    /// The runs of text the line joins, in content order: each run's text as its glyphs show it,
    /// the boxes of its words (a part of `ink`), and whether joining put a space between it and
    /// the run before (`text` has one there; runs closer together join with none, so one word
    /// can span runs). None when the line's text was reordered for reading (right-to-left or
    /// visual-order Indic text): then only the line's text as a whole says what it reads.
    pub fn runs(&self) -> impl Iterator<Item = (&str, &[[f64; 4]], bool)> {
        let mut start = 0;
        self.runs.iter().map(move |(text, end, spaced)| {
            let ink = self.ink.get(start..*end).unwrap_or_default();
            start = *end;
            (text.as_str(), ink, *spaced)
        })
    }

    pub(crate) fn rotation(&self) -> f64 {
        let m = Matrix(self.origin.tm).then(&Matrix(self.origin.ctm)).0;
        m[1].atan2(m[0]).to_degrees()
    }

    /// The unit direction its baseline runs in, in user space.
    pub(super) fn direction(&self) -> (f64, f64) {
        direction(&Matrix(self.origin.tm).then(&Matrix(self.origin.ctm)))
    }

    /// Whether it reads left to right along the page's x axis (most text; a page drawn
    /// turned a quarter, as landscape pages often are, isn't).
    pub(crate) fn upright(&self) -> bool {
        let (x, y) = self.direction();
        (x - 1.0).abs() < 1e-3 && y.abs() < 1e-3
    }

    /// Where it starts as `(along, across)` its baseline: user space turned to the text's direction.
    pub(crate) fn frame_origin(&self) -> (f64, f64) {
        to_frame(self.direction(), (self.origin.x, self.origin.baseline))
    }

    /// Its box in the same frame, `[along0, across0, along1, across1]` (for upright text, `rect`).
    pub(crate) fn frame_rect(&self) -> [f64; 4] {
        self.frame
    }
}

/// The page's content streams as listed, each with its decoded data (empty when it doesn't
/// decode), within one page's [`DecodeBudget`]. An error when the content runs past it: reading
/// or rewriting a page from part of its content would lose the rest.
pub(crate) fn content_streams(doc: &Document, page: &Dict) -> Result<Vec<(Object, Vec<u8>)>, EditError> {
    content_streams_within(doc, page, &mut DecodeBudget::page())
}

/// [`content_streams`], decoded against `budget`, which a walk of the page's forms then goes on
/// spending.
pub(crate) fn content_streams_within(doc: &Document, page: &Dict, budget: &mut DecodeBudget) -> Result<Vec<(Object, Vec<u8>)>, EditError> {
    let list: Vec<Object> = match page.get(b"Contents") {
        None => Vec::new(),
        Some(c) => match &*doc.resolve(c) {
            Object::Array(a) => a.clone(),
            _ => vec![c.clone()],
        },
    };
    let mut out = Vec::with_capacity(list.len());
    for o in list {
        let data = match &*doc.resolve(&o) {
            Object::Stream(s) => match budget.decode(s) {
                Ok(data) => data,
                Err(e) if budget.remaining() == 0 => return Err(EditError::Invalid(format!("the page content is too large to read: {e}"))),
                Err(_) => Vec::new(),
            },
            _ => Vec::new(),
        };
        out.push((o, data));
    }
    Ok(out)
}

/// Rewrite a page's content (its streams joined, see [`Pieces`]): `edit(i)` gives the operators
/// to insert before operator `i` of `ops` (parsed from the joined pieces) and whether to keep it.
/// Everything else is copied byte for byte. A page's streams are one stream in pieces, split
/// between any two tokens (ISO 32000-2 §7.8.2), so an operator's operands can end one piece and
/// its keyword start the next; removing or replacing it rewrites every piece it spans. Returns
/// each piece's new bytes.
pub(crate) fn splice(pieces: &Pieces, ops: &[Op], mut edit: impl FnMut(usize) -> (Vec<Op>, bool)) -> Vec<Vec<u8>> {
    let data = pieces.data();
    let mut edits = Vec::new();
    for (i, op) in ops.iter().enumerate() {
        let (insert, keep) = edit(i);
        if insert.is_empty() && keep {
            continue;
        }
        let mut bytes = Vec::new();
        if !insert.is_empty() {
            if op.span.start.checked_sub(1).and_then(|p| data.get(p)).is_some_and(|b| !b.is_ascii_whitespace()) {
                bytes.push(b'\n');
            }
            bytes.extend_from_slice(&serialize_ops(&insert));
        }
        let end = if keep { op.span.start } else { op.span.end };
        edits.push((op.span.start..end, bytes));
    }
    pieces.splice(edits)
}

/// The page's `/Contents` after a [`splice`]: pieces whose bytes changed become new streams
/// (keeping their dictionaries), the others stay as they were.
pub(crate) fn rewritten_contents(doc: &mut Document, streams: &[(Object, Vec<u8>)], new: Vec<Vec<u8>>) -> Object {
    let mut contents: Vec<Object> = Vec::with_capacity(streams.len());
    for ((obj, old), data) in streams.iter().zip(new) {
        if *old == data {
            contents.push(obj.clone());
            continue;
        }
        let mut dict = match &*doc.resolve(obj) {
            Object::Stream(s) => s.dict.clone(),
            _ => Dict::new(),
        };
        dict.remove(b"Length");
        contents.push(Object::Ref(doc.add(Object::Stream(Stream::flate(dict, &data)))));
    }
    match <[Object; 1]>::try_from(contents) {
        Ok([one]) => one,
        Err(many) => Object::Array(many),
    }
}

pub(crate) fn page_dict(doc: &Document, page: usize) -> Result<pdfcraft_model::Page, EditError> {
    pdfcraft_model::pages(doc).into_iter().nth(page).ok_or(EditError::NoSuchPage(page))
}

pub(super) fn fill_color(fill: &[Op]) -> [f64; 3] {
    let Some(op) = fill.iter().rev().find(|op| matches!(op.op.as_slice(), b"g" | b"rg" | b"k" | b"sc" | b"scn")) else {
        return [0.0, 0.0, 0.0];
    };
    let space = match op.op.as_slice() {
        b"sc" | b"scn" => match op.operands.len() {
            1 => b"g".as_slice(),
            3 => b"rg".as_slice(),
            4 => b"k".as_slice(),
            _ => return [0.0; 3],
        },
        other => other,
    };
    match space {
        b"g" => {
            let v = op.operands.first().and_then(Object::as_f64).unwrap_or(0.0).clamp(0.0, 1.0);
            [v, v, v]
        }
        b"rg" => [
            op.operands.first().and_then(Object::as_f64).unwrap_or(0.0).clamp(0.0, 1.0),
            op.operands.get(1).and_then(Object::as_f64).unwrap_or(0.0).clamp(0.0, 1.0),
            op.operands.get(2).and_then(Object::as_f64).unwrap_or(0.0).clamp(0.0, 1.0),
        ],
        b"k" => {
            let c = op.operands.first().and_then(Object::as_f64).unwrap_or(0.0);
            let m = op.operands.get(1).and_then(Object::as_f64).unwrap_or(0.0);
            let y = op.operands.get(2).and_then(Object::as_f64).unwrap_or(0.0);
            let k = op.operands.get(3).and_then(Object::as_f64).unwrap_or(0.0);
            [(1.0 - c) * (1.0 - k), (1.0 - m) * (1.0 - k), (1.0 - y) * (1.0 - k)]
        }
        _ => [0.0, 0.0, 0.0],
    }
}

/// Whether a requested colour is the source's own, as a client that read it back as 8-bit hex
/// sends it: then the source's colour operators stay, in whatever colour space they use.
pub(crate) fn same_colour(a: [f64; 3], b: [f64; 3]) -> bool {
    a.iter().zip(b).all(|(a, b)| (a - b).abs() <= 0.6 / 255.0)
}

/// The fill a `cs` … `sc`/`scn` sequence sets, as RGB, through the colour space it names
/// (ICC-based, calibrated, Lab, Indexed, and Separation/DeviceN with an exponential tint
/// transform). `None` when it isn't such a sequence or the space can't be read (patterns).
pub(super) fn fill_rgb(doc: &Document, colour_spaces: &Dict, fill: &[Op]) -> Option<[f64; 3]> {
    let cs = fill.iter().find(|o| o.is("cs"))?;
    let set = fill.iter().rev().find(|o| o.is("sc") || o.is("scn"))?;
    let comps: Vec<f64> = set.operands.iter().map(Object::as_f64).collect::<Option<_>>()?;
    let name = cs.name(0)?;
    let space = match name {
        b"DeviceGray" | b"DeviceRGB" | b"DeviceCMYK" | b"G" | b"RGB" | b"CMYK" => Object::Name(name.to_vec()),
        _ => (*doc.resolve(colour_spaces.get(name)?)).clone(),
    };
    space_rgb(doc, &space, &comps, 0)
}

fn space_rgb(doc: &Document, space: &Object, comps: &[f64], depth: usize) -> Option<[f64; 3]> {
    let c = |i: usize| comps.get(i).copied().unwrap_or(0.0).clamp(0.0, 1.0);
    let by_count = |n: usize| match n {
        1 => Some([c(0); 3]),
        3 => Some([c(0), c(1), c(2)]),
        4 => Some([(1.0 - c(0)) * (1.0 - c(3)), (1.0 - c(1)) * (1.0 - c(3)), (1.0 - c(2)) * (1.0 - c(3))]),
        _ => None,
    };
    if depth > 3 {
        return None;
    }
    let (family, array) = match space {
        Object::Name(n) => (n.as_slice(), &[][..]),
        Object::Array(a) => (a.first().and_then(Object::as_name)?, a.as_slice()),
        _ => return None,
    };
    let arg = |i: usize| array.get(i).map(|o| doc.resolve(o));
    match family {
        b"DeviceGray" | b"G" | b"CalGray" => by_count(1),
        b"DeviceRGB" | b"RGB" | b"CalRGB" => by_count(3),
        b"DeviceCMYK" | b"CMYK" => by_count(4),
        b"ICCBased" => {
            let n = arg(1).and_then(|s| s.as_dict().and_then(|d| d.int(b"N")))?;
            by_count(usize::try_from(n).ok()?)
        }
        b"Lab" => {
            // CIE L*a*b* (D50 assumed) to sRGB, near enough to tell a colour back.
            let l = comps.first().copied().unwrap_or(0.0);
            let (a, b) = (comps.get(1).copied().unwrap_or(0.0), comps.get(2).copied().unwrap_or(0.0));
            let fy = (l + 16.0) / 116.0;
            let f = |t: f64| if t > 6.0 / 29.0 { t * t * t } else { 3.0 * (6.0f64 / 29.0).powi(2) * (t - 4.0 / 29.0) };
            let (x, y, z) = (0.9642 * f(fy + a / 500.0), f(fy), 0.8249 * f(fy - b / 200.0));
            let lin = [3.1339 * x - 1.6169 * y - 0.4906 * z, -0.9788 * x + 1.9161 * y + 0.0335 * z, 0.0719 * x - 0.2290 * y + 1.4052 * z];
            let gamma = |v: f64| {
                let v = v.clamp(0.0, 1.0);
                if v <= 0.003_130_8 { 12.92 * v } else { 1.055 * v.powf(1.0 / 2.4) - 0.055 }
            };
            Some(lin.map(gamma))
        }
        b"Indexed" | b"I" => {
            let base = arg(1)?;
            let n = match &*base {
                Object::Name(b) if matches!(b.as_slice(), b"DeviceGray" | b"G" | b"CalGray") => 1,
                Object::Name(b) if matches!(b.as_slice(), b"DeviceCMYK" | b"CMYK") => 4,
                _ => 3,
            };
            let n = match &*base {
                Object::Array(a) if a.first().and_then(Object::as_name) == Some(b"ICCBased") => {
                    a.get(1).and_then(|s| doc.resolve(s).as_dict().and_then(|d| d.int(b"N"))).and_then(|n| usize::try_from(n).ok()).unwrap_or(n)
                }
                _ => n,
            };
            let table = match &*arg(3)? {
                Object::String(s) => s.bytes.clone(),
                Object::Stream(s) => s.decoded_within(4096).ok()?,
                _ => return None,
            };
            let index = usize::try_from(comps.first().copied()?.round() as i64).ok()?;
            let entry: Vec<f64> =
                table.get(index.checked_mul(n)?..index.checked_mul(n)?.checked_add(n)?)?.iter().map(|b| f64::from(*b) / 255.0).collect();
            space_rgb(doc, &base, &entry, depth + 1)
        }
        b"Separation" | b"DeviceN" => {
            // Exponential (type 2) tint transforms only: C0 + t^N (C1 − C0).
            let function = arg(3)?;
            let f = function.as_dict()?;
            if f.int(b"FunctionType") != Some(2) || comps.len() != 1 {
                return None;
            }
            let nums = |k: &[u8], d: Vec<f64>| {
                f.get(k).and_then(|v| doc.resolve(v).as_array().map(|a| a.iter().filter_map(Object::as_f64).collect())).unwrap_or(d)
            };
            let (c0, c1) = (nums(b"C0", vec![0.0]), nums(b"C1", vec![1.0]));
            let e = f.get(b"N").and_then(Object::as_f64).unwrap_or(1.0);
            let t = c(0).powf(e);
            let out: Vec<f64> = c0.iter().zip(&c1).map(|(a, b)| a + t * (b - a)).collect();
            space_rgb(doc, &*arg(2)?, &out, depth + 1)
        }
        _ => None,
    }
}

/// The lines of text on a page (0-based), in content order.
pub fn text_lines(doc: &Document, page: usize) -> Result<Vec<TextLine>, EditError> {
    // Text drawn in form XObjects is read as if the forms were inline, which is how every rewrite
    // here writes such a page (see `inline_forms`).
    let view = crate::inline_forms::view(doc, page)?;
    let doc: &Document = &view;
    text_lines_in(doc, &page_dict(doc, page)?, true)
}

/// The lines `page`'s glyphs show, in content order, without `/ActualText` replacements: what the
/// page draws, not what its marked-content metadata says the glyphs mean. For checks that must
/// not trust that metadata (the redaction audit). Takes the page itself, so a caller going
/// through every page doesn't walk the page tree again for each.
pub fn glyph_text_lines(doc: &Document, page: &pdfcraft_model::Page) -> Result<Vec<TextLine>, EditError> {
    text_lines_in(doc, page, false)
}

/// [`text_lines`] of `p`; with `actual_text`, `/ActualText` replaces the glyphs it spans.
fn text_lines_in(doc: &Document, p: &pdfcraft_model::Page, actual_text: bool) -> Result<Vec<TextLine>, EditError> {
    let res = p.dict.get(b"Resources").map(|r| doc.resolve(r)).and_then(|r| r.as_dict().cloned()).unwrap_or_default();
    let mut cache = HashMap::new();
    let mut lines: Vec<TextLine> = Vec::new();
    let mut carry = Carry::new();
    // The streams are one stream in pieces (§7.8.2), split between any two tokens: parse them
    // joined, so an operator whose operands end one piece still has them.
    let pieces = Pieces::join(&content_streams(doc, &p.dict)?.into_iter().map(|(_, d)| d).collect::<Vec<_>>());
    let ops = pieces.parse().ops;
    let properties = res.get(b"Properties").and_then(|o| doc.dict(o)).unwrap_or_default();
    let shown = interpret(doc, &ops, &res, &mut cache, &mut carry, None, actual_text.then_some(&properties))?;
    let mut last: Option<(usize, Pen)> = None; // the stream piece and where its last run ended
    for mut s in shown {
        // A single RTL glyph's ActualText (e.g. lam-alef) is an inline ligature, not a
        // separate logical line. Put its letters into visual order before joining the
        // surrounding glyphs; the completed line is recovered to typing order below.
        // Multi-glyph and mixed-direction replacements keep their authoritative order.
        if s.logical_order && s.glyph_count == 1 && s.text.chars().all(crate::bidi::is_rtl) {
            s.text = crate::bidi::to_visual(&s.text);
            s.logical_order = false;
        }
        // A line belongs to one stream piece. Within it, PDF producers often put every word
        // in its own BT/ET: geometry and text state, not object boundaries, define a line.
        let si = ops.get(s.op).map_or(0, |op| pieces.pieces_of(op).0);
        let joins = last.is_some_and(|(piece, pen)| {
            let bt = pen.bt;
            let (near, gap) = pen.continues(&s, if bt == s.bt { 3.0 } else { 1.5 });
            piece == si
                && lines.last().is_some_and(|l| l.scope == s.scope && l.paint == s.paint && l.logical_order == s.logical_order)
                && (bt == s.bt || lines.last().is_some_and(|l| compatible_word(l, &s)))
                && (s.text.trim().is_empty() || lines.last().is_some_and(|l| same_font(l, &s.font, &s.base_font, s.face.as_deref())))
                && (s.text.trim().is_empty()
                    || lines.last().is_some_and(|l| {
                        (s.size - l.size).abs() < 0.01
                            && s.bold == l.bold
                            && s.italic == l.italic
                            && fill_color(&s.state.fill) == l.color
                            && same_paint(l, &s.state)
                    }))
                && near
                // An OCR layer is one text object over the whole scan: its words continue a line
                // only across a word space, not across a table cell or a tab stop.
                && (s.state.render_mode != 3 || gap < pen.size * 0.6)
        });
        if joins && let Some(l) = lines.last_mut() {
            let gap = last.map_or(0.0, |(_, pen)| pen.continues(&s, 0.0).1);
            let spaced = gap - s.tracking > l.size * s.state.scale.abs() * 0.2 && !l.text.ends_with(' ') && !s.text.starts_with(' ');
            if spaced {
                l.text.push(' ');
            }
            l.text.push_str(&s.text);
            l.rect = [l.rect[0].min(s.rect[0]), l.rect[1].min(s.rect[1]), l.rect[2].max(s.rect[2]), l.rect[3].max(s.rect[3])];
            l.frame = union(l.frame, s.frame);
            l.ink.extend(s.ink.iter().copied());
            l.runs.push((s.text.clone(), l.ink.len(), spaced));
            l.ops.push(s.op);
            l.ops.extend(s.extra_ops.iter().copied());
            l.decodable &= s.decodable;
            l.visual_order |= s.visual_order;
            l.actual_text |= s.actual_text;
            l.clips |= s.state.render_mode >= 4;
            if l.unreadable_mode.is_none() {
                l.unreadable_mode = s.state.unreadable.as_deref().map(str::to_owned);
            }
            // Spaces may use a separate subset font; they do not change the visible style.
        } else {
            lines.push(TextLine {
                text: s.text.clone(),
                rect: s.rect,
                frame: s.frame,
                ink: s.ink.clone(),
                font: String::from_utf8_lossy(&s.font).into_owned(),
                base_font: s.base_font.clone(),
                size: s.size,
                bold: s.bold,
                italic: s.italic,
                color: fill_color(&s.state.fill),
                decodable: s.decodable,
                visual_order: s.visual_order,
                face: s.face.clone(),
                stream: si,
                scope: s.scope,
                paint: s.paint.clone(),
                logical_order: s.logical_order,
                actual_text: s.actual_text,
                clips: s.state.render_mode >= 4,
                unreadable_mode: s.state.unreadable.as_deref().map(str::to_owned),
                runs: vec![(s.text.clone(), s.ink.len(), false)],
                soft_break: s.soft_break,
                hard_break: s.hard_break,
                soft_gap: s.soft_gap,
                ops: std::iter::once(s.op).chain(s.extra_ops.iter().copied()).collect(),
                origin: Origin {
                    tm: s.tm.0,
                    tlm: s.tlm.0,
                    k: (s.tm.then(&s.state.ctm).0[2].powi(2) + s.tm.then(&s.state.ctm).0[3].powi(2)).sqrt(),
                    ctm: s.state.ctm.0,
                    baseline: s.baseline,
                    x: s.start_x,
                    state: TextState::of(&s.state),
                    bt_op: s.bt_op,
                    bt_state: TextState::of(&s.bt_state),
                },
            });
        }
        last = Some((si, Pen::after(&s)));
    }
    // Content streams draw right-to-left lines in visual order; the editor works in typing order.
    for l in lines.iter_mut().filter(|l| !l.logical_order) {
        // A visual-order Indic font draws pre-base vowel signs before their consonant.
        let shown = l.text.clone();
        if l.visual_order && !l.actual_text {
            l.text = crate::indic::to_logical(&l.text);
        }
        l.text = crate::bidi::to_logical(&l.text);
        // The runs hold the text in drawing order, which no longer reads as the line does.
        if l.text != shown {
            l.runs.clear();
        }
    }
    // A colour set through a colour space (Chrome writes ICC-based sRGB) reads as itself.
    let colour_spaces = res.get(b"ColorSpace").map(|c| doc.resolve(c)).and_then(|c| c.as_dict().cloned()).unwrap_or_default();
    for l in &mut lines {
        if let Some(rgb) = fill_rgb(doc, &colour_spaces, &l.origin.state.fill) {
            l.color = rgb;
        }
    }
    // Lines of only spaces aren't editable text.
    lines.retain(|l| !l.text.trim().is_empty());
    Ok(lines)
}

/// Joining independent text objects is safe only when they use the same text and graphics
/// transforms and spacing. In particular, a nearby rotated label isn't part of this line.
fn compatible_word(line: &TextLine, shown: &Shown) -> bool {
    let within = |a: &[f64], b: &[f64], tolerance: f64| a.iter().zip(b).all(|(a, b)| (a - b).abs() < tolerance);
    let same = |a: &[f64], b: &[f64]| within(a, b, 1e-6);
    let (a, b) = (&line.origin.state, TextState::of(&shown.state));
    // Spacing that differs by less than a thousandth of an em, the unit glyph widths are given in,
    // can't be seen. Some producers still write it (`0.0001 Tc` on one word of a line).
    let em = shown.state.size.abs() / 1000.0;
    line.scope == shown.scope
        && line.paint == shown.paint
        && same(&line.origin.tm[..4], &shown.tm.0[..4])
        && same(&line.origin.ctm, &shown.state.ctm.0)
        && within(&[a.char_spacing, a.word_spacing], &[b.char_spacing, b.word_spacing], em.max(1e-6))
        && same(&[a.scale, a.rise], &[b.scale, b.rise])
        && a.fill.len() == b.fill.len()
        && a.fill.iter().zip(&b.fill).all(|(a, b)| a.op == b.op && a.operands == b.operands)
}

/// The line `s` continues, or a new one: `last` is the previous run of the same stream
/// (`bt`, baseline, end x, size).
pub(super) fn add_shown(lines: &mut Vec<TextLine>, last: &mut Option<Pen>, s: Shown, stream: usize) {
    let joins = last.is_some_and(|pen| {
        let bt = pen.bt;
        lines.last().is_some_and(|l| l.scope == s.scope && l.paint == s.paint && l.logical_order == s.logical_order)
            && (bt == s.bt || lines.last().is_some_and(|l| compatible_word(l, &s)))
            && (s.text.trim().is_empty() || lines.last().is_some_and(|l| same_font(l, &s.font, &s.base_font, s.face.as_deref())))
            && (s.text.trim().is_empty()
                || lines.last().is_some_and(|l| {
                    (s.size - l.size).abs() < 0.01
                        && s.bold == l.bold
                        && s.italic == l.italic
                        && fill_color(&s.state.fill) == l.color
                        && same_paint(l, &s.state)
                }))
            && pen.continues(&s, if bt == s.bt { 3.0 } else { 1.5 }).0
    });
    if joins && let Some(l) = lines.last_mut() {
        let gap = last.map_or(0.0, |pen| pen.continues(&s, 0.0).1);
        let spaced = gap - s.tracking > l.size * s.state.scale.abs() * 0.2 && !l.text.ends_with(' ') && !s.text.starts_with(' ');
        if spaced {
            l.text.push(' ');
        }
        l.text.push_str(&s.text);
        l.rect = [l.rect[0].min(s.rect[0]), l.rect[1].min(s.rect[1]), l.rect[2].max(s.rect[2]), l.rect[3].max(s.rect[3])];
        l.frame = union(l.frame, s.frame);
        l.ink.extend(s.ink.iter().copied());
        l.runs.push((s.text.clone(), l.ink.len(), spaced));
        l.ops.push(s.op);
        l.ops.extend(s.extra_ops.iter().copied());
        l.decodable &= s.decodable;
        l.visual_order |= s.visual_order;
        l.actual_text |= s.actual_text;
        l.clips |= s.state.render_mode >= 4;
        if l.unreadable_mode.is_none() {
            l.unreadable_mode = s.state.unreadable.as_deref().map(str::to_owned);
        }
    } else {
        lines.push(TextLine {
            text: s.text.clone(),
            rect: s.rect,
            frame: s.frame,
            ink: s.ink.clone(),
            font: String::from_utf8_lossy(&s.font).into_owned(),
            base_font: s.base_font.clone(),
            size: s.size,
            bold: s.bold,
            italic: s.italic,
            color: fill_color(&s.state.fill),
            decodable: s.decodable,
            visual_order: s.visual_order,
            face: s.face.clone(),
            stream,
            scope: s.scope,
            paint: s.paint.clone(),
            logical_order: s.logical_order,
            actual_text: s.actual_text,
            clips: s.state.render_mode >= 4,
            unreadable_mode: s.state.unreadable.as_deref().map(str::to_owned),
            runs: vec![(s.text.clone(), s.ink.len(), false)],
            soft_break: s.soft_break,
            hard_break: s.hard_break,
            soft_gap: s.soft_gap,
            ops: std::iter::once(s.op).chain(s.extra_ops.iter().copied()).collect(),
            origin: Origin {
                tm: s.tm.0,
                tlm: s.tlm.0,
                k: (s.tm.then(&s.state.ctm).0[2].powi(2) + s.tm.then(&s.state.ctm).0[3].powi(2)).sqrt(),
                ctm: s.state.ctm.0,
                baseline: s.baseline,
                x: s.start_x,
                state: TextState::of(&s.state),
                bt_op: s.bt_op,
                bt_state: TextState::of(&s.bt_state),
            },
        });
    }
    *last = Some(Pen::after(&s));
}

/// How deep form XObjects nest before reading stops (they may also refer to themselves). The
/// renderer stops at the same depth, so export reads the text the page shows (#3120).
pub const MAX_FORM_DEPTH: usize = 50;

/// The most form XObjects one page's walk enters, at any depth. The depth cap alone still lets
/// a form that draws another form many times fan out exponentially (100 `Do`s on each of 50
/// levels), which would hang export on a hostile PDF. The renderer paints as many forms nested
/// in forms; this budget also counts the forms the page draws itself, so a page that draws one
/// a million times stays bounded too. A label or table cell drawn as a form a few thousand
/// times is read in full (#3120).
pub const MAX_FORM_VISITS: usize = 50_000;

/// One page's walk through the form XObjects it draws: how many it entered (the page's
/// [`MAX_FORM_VISITS`]), the bytes the page's content and forms may still decode to, and whether
/// a limit left a form unread.
pub(crate) struct FormBudget {
    visits: usize,
    decode: DecodeBudget,
    /// A form was left unread because it nests deeper than [`MAX_FORM_DEPTH`], comes past
    /// [`MAX_FORM_VISITS`] or past the decode budget: what it draws is missing from the walk.
    pub(crate) truncated: bool,
}

impl FormBudget {
    /// The walk of a page's forms, spending what `decode` has left after the page's content.
    pub(crate) fn new(decode: DecodeBudget) -> Self {
        Self { visits: 0, decode, truncated: false }
    }
}

/// A form XObject a page draws: its object, content, resources and its matrix composed with the
/// CTM at the `Do`.
pub(crate) struct FormCall {
    pub(crate) obj: Option<pdfcraft_cos::ObjRef>,
    pub(crate) data: Vec<u8>,
    /// The form, and its `/Resources` when they are an object of their own, which is what
    /// [`FormCall::resources`] reads.
    form: Arc<Object>,
    own: Option<Arc<Object>>,
    pub(crate) ctm: Matrix,
}

impl FormCall {
    /// The resources its content is read with: its own, or if it has none, `drawn_with`, the ones
    /// it was drawn with (PDF 1.1 files). Read in place, as a page can draw a form thousands of times:
    /// a copy for each took time in the square of them.
    pub(crate) fn resources<'a>(&'a self, drawn_with: &'a Dict) -> &'a Dict {
        let direct = || match &*self.form {
            Object::Stream(s) => s.dict.get(b"Resources").and_then(Object::as_dict),
            _ => None,
        };
        self.own.as_deref().and_then(Object::as_dict).or_else(direct).unwrap_or(drawn_with)
    }
}

/// `o` as a dictionary, read in place for the same reason as [`FormCall::resources`]. `held` keeps it
/// when it is an object of its own. Nothing else is one, and [`Document::resolve`] would copy it.
pub(crate) fn dict_in_place<'a>(doc: &Document, o: &'a Object, held: &'a mut Option<Arc<Object>>) -> Option<&'a Dict> {
    match o {
        Object::Dict(dict) => Some(dict),
        Object::Ref(_) => held.insert(doc.resolve(o)).as_dict(),
        _ => None,
    }
}

/// The dictionary at `key` of `resources`, `/XObject` or `/Font`, say, as each form reads these.
pub(crate) fn resource_dict<'a>(doc: &Document, resources: &'a Dict, key: &[u8], held: &'a mut Option<Arc<Object>>) -> Option<&'a Dict> {
    dict_in_place(doc, resources.get(key)?, held)
}

/// The form XObject `name` in `resources`, drawn with `ctm`. `None` for images, missing names
/// and forms already being read (`path`), and for forms nested too deep or past the page's
/// budget, which `budget` records as truncated; each form entered counts one visit.
pub(crate) fn form_call(
    doc: &Document,
    resources: &Dict,
    name: &[u8],
    ctm: Matrix,
    path: &[pdfcraft_cos::ObjRef],
    budget: &mut FormBudget,
) -> Option<FormCall> {
    let limited = path.len() >= MAX_FORM_DEPTH || budget.visits >= MAX_FORM_VISITS || budget.decode.remaining() == 0;
    // Once a limit has cut the page short, refusing the rest costs nothing more.
    if limited && budget.truncated {
        return None;
    }
    let mut held = None;
    let o = resource_dict(doc, resources, b"XObject", &mut held)?.get(name)?;
    // Only a reference can name a stream, so nothing else needs reading, or copying.
    let obj = Some(o.as_ref()?);
    if obj.is_some_and(|r| path.contains(&r)) {
        return None;
    }
    let form = doc.resolve(o);
    let Object::Stream(s) = &*form else { return None };
    if s.dict.name(b"Subtype") != Some(b"Form") {
        return None;
    }
    // Only a form left unread loses content; an image at the limit is still read by its caller.
    if limited {
        budget.truncated = true;
        return None;
    }
    let m = s.dict.get(b"Matrix").map(|m| doc.resolve(m)).and_then(|m| {
        let a = m.as_array()?;
        let v: Vec<f64> = a.iter().filter_map(|x| doc.resolve(x).as_f64()).collect();
        <[f64; 6]>::try_from(v).ok().filter(|v| v.iter().all(|x| x.is_finite()))
    });
    let own = s.dict.get(b"Resources").filter(|r| matches!(r, Object::Ref(_))).map(|r| doc.resolve(r));
    let data = match budget.decode.decode(s) {
        Ok(data) => data,
        Err(_) if budget.decode.remaining() == 0 => {
            budget.truncated = true;
            return None;
        }
        Err(_) => Vec::new(),
    };
    budget.visits = budget.visits.saturating_add(1);
    Some(FormCall { obj, data, form, own, ctm: m.map_or(ctm, |m| Matrix(m).then(&ctm)) })
}

/// The lines a page shows as it reads, including text drawn by form XObjects (which
/// [`text_lines`] leaves out because it can only rewrite the page's own streams). Lines from a
/// form carry a stream number past the page's streams, one per form drawn. The flag is `true`
/// when forms past [`MAX_FORM_DEPTH`] or [`MAX_FORM_VISITS`] were left unread.
pub(super) fn reading_lines(doc: &Document, page: usize) -> Result<(Vec<TextLine>, bool), EditError> {
    struct Walk<'a> {
        doc: &'a Document,
        runs: Vec<super::reading::Run>,
        gradients: Vec<super::reading::Gradient>,
        next_stream: usize,
        path: Vec<pdfcraft_cos::ObjRef>,
        budget: FormBudget,
    }
    /// `stream` gives the stream number of the operator at an index of `ops`.
    fn walk(w: &mut Walk, ops: &[Op], resources: &Dict, carry: &mut Carry, stream: &dyn Fn(usize) -> usize) -> Result<(), EditError> {
        let mut cache = HashMap::new();
        let mut invoked = Vec::new();
        let (mut held, none) = (None, Dict::new());
        let properties = resource_dict(w.doc, resources, b"Properties", &mut held).unwrap_or(&none);
        let shown = interpret(w.doc, ops, resources, &mut cache, carry, Some(&mut invoked), Some(properties))?;
        let mut forms = invoked.into_iter().peekable();
        for s in shown {
            while let Some((_, name, ts)) = forms.next_if(|f| f.0 < s.op) {
                enter(w, resources, &name, ts, &mut carry.text_left)?;
            }
            // Lines are made from the runs in `reading::lines_of`; text after a form, or from
            // another stream, starts a new one there.
            let color = super::reading::run_colour(w.doc, resources, &s.state.fill);
            let invisible = super::reading::invisible(&s);
            let order = w.runs.len();
            w.runs.push(super::reading::Run { stream: stream(s.op), order, color, invisible, shown: s });
        }
        for (_, name, ts) in forms {
            enter(w, resources, &name, ts, &mut carry.text_left)?;
        }
        Ok(())
    }
    /// `text_left`: the page's text budget, shared with the forms the page draws.
    fn enter(w: &mut Walk, resources: &Dict, name: &[u8], ts: Ts, text_left: &mut usize) -> Result<(), EditError> {
        let Some(form) = form_call(w.doc, resources, name, ts.ctm, &w.path, &mut w.budget) else { return Ok(()) };
        let ops = parse(&form.data).ops;
        if let Some(gradient) = super::reading::form_paint(w.doc, &form, resources, &ops) {
            w.gradients.push(gradient);
        }
        // The form starts with the graphics state at its `Do`, its own matrix applied.
        let mut carry = Carry { ts: Ts { ctm: form.ctm, ..ts }, stack: Vec::new(), marked: Vec::new(), layers: Vec::new(), text_left: *text_left };
        let stream = w.next_stream;
        w.next_stream = w.next_stream.saturating_add(1);
        if let Some(r) = form.obj {
            w.path.push(r);
        }
        walk(w, &ops, form.resources(resources), &mut carry, &|_| stream)?;
        *text_left = carry.text_left;
        if form.obj.is_some() {
            w.path.pop();
        }
        Ok(())
    }
    let p = page_dict(doc, page)?;
    let res = p.dict.get(b"Resources").map(|r| doc.resolve(r)).and_then(|r| r.as_dict().cloned()).unwrap_or_default();
    let mut decode = DecodeBudget::page();
    let streams = content_streams_within(doc, &p.dict, &mut decode)?;
    let mut w = Walk { doc, runs: Vec::new(), gradients: Vec::new(), next_stream: streams.len(), path: Vec::new(), budget: FormBudget::new(decode) };
    let mut carry = Carry::new();
    // Read the page's streams as the one stream they are, in one pass, as rendering and editing
    // do: a comment, an operator's operands or a marked-content sequence (its /ActualText too)
    // can run on from one stream into the next. Text still counts as the stream its operator
    // starts in, which keeps lines from different streams apart.
    let pieces = Pieces::join(&streams.into_iter().map(|(_, data)| data).collect::<Vec<_>>());
    let ops = pieces.parse().ops;
    let stream_of = |i: usize| ops.get(i).map_or(0, |op| pieces.pieces_of(op).0);
    walk(&mut w, &ops, &res, &mut carry, &stream_of)?;
    // Upright lines are rebuilt from their glyphs (browsers print glyph by glyph, in several
    // subsets of a font); other text is joined run by run.
    let mut lines = super::reading::lines_of(std::mem::take(&mut w.runs), &w.gradients);
    // A visual-order Indic font draws pre-base vowel signs before their consonant.
    for l in lines.iter_mut().filter(|l| l.visual_order && !l.actual_text) {
        let shown = l.text.clone();
        l.text = crate::indic::to_logical(&l.text);
        if l.text != shown {
            l.runs.clear();
        }
    }
    lines.retain(|l| !l.text.trim().is_empty());
    Ok((lines, w.budget.truncated))
}
