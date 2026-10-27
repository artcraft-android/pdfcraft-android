//! Edit a PDF ▸ Edit text: the text lines already on a page, and replacing a line's text in place.
//!
//! A *line* is a run of compatible text-showing operators on the same baseline, read
//! from one content stream. Replacing it rewrites only that stream (as a new object, since content
//! streams may be shared): the first operator of the line shows the new text, the others are
//! dropped, and every positioning operator stays, so the lines after it keep their places
//! (`Td`, `T*` and friends move from the line matrix, which showing text doesn't change).
//!
//! Text a page draws inside form XObjects (Chrome puts whole pages in one) is read and edited as
//! if the forms were drawn inline: the first rewrite of such a page inlines them for good, with
//! the same rendering (see `inline_forms`).
//!
//! The new text uses the line's own font when every character has a code and a glyph in it;
//! otherwise it is set in Helvetica (WinAnsi), and the result says the font was substituted.
//! Reflowing a paragraph to a new width is not done here (see `edit.text-reflow`).

use std::collections::HashMap;
use std::rc::Rc;

use pdfcraft_content::{Matrix, Op, Pieces, parse, serialize_ops};
use pdfcraft_cos::{Dict, Document, Object, PdfString, Stream};
use pdfcraft_fonts::pdf::Metrics;
use pdfcraft_fonts::{CraftFont, GlyphError, japanese_glyph_from};

use crate::EditError;

#[path = "text_reading.rs"]
mod reading;
#[path = "text_regions.rs"]
mod regions;
use regions::{group_blocks, rewrite_styled_block};

/// One line of existing text.
#[derive(Clone, Debug, PartialEq)]
pub struct TextLine {
    /// The line's text as shown.
    pub text: String,
    /// Its box in user space.
    pub rect: [f64; 4],
    /// The font's resource name and `/BaseFont` (or Type 3 descriptor's `/FontName`), and its size in text space.
    pub font: String,
    pub base_font: String,
    pub size: f64,
    pub bold: bool,
    pub italic: bool,
    /// The current fill colour as RGB.
    pub color: [f64; 3],
    /// Whether new text in this font can only be shown by substituting another font (no
    /// Unicode mapping for the line, so nothing could be reused).
    pub decodable: bool,
    /// The Japanese or Hebrew fallback face the font subsets, when the line is in one (see [`same_font`]).
    face: Option<String>,
    /// The content stream piece the line starts in, and its operators (indices into the page's
    /// streams parsed joined).
    stream: usize,
    ops: Vec<usize>,
    /// Where the line starts: text matrix (text space), the state there, and its `BT`.
    origin: Origin,
    /// Explicit marked-content scope (when present); distinct scopes must not be guessed into one paragraph.
    scope: Option<usize>,
    /// Whether the line paints anything visible (see [`shown_visible`]). Invisible text (a
    /// selectable copy over a picture, an OCR layer, text clipped away) is never a paragraph.
    visible: bool,
    /// The fill alpha it is drawn with: a rewrite draws a paragraph in one graphics state.
    alpha: f64,
    /// The bounds of the clip it is drawn in (see `Ts::clip`).
    clip: Option<[f64; 4]>,
}

impl TextLine {
    /// Whether text drawn in this line's graphics state shows where `other` is: a rewrite draws
    /// a whole paragraph in the state of its first line, so its other lines must lie inside that
    /// line's clip.
    pub(crate) fn reaches(&self, other: &TextLine) -> bool {
        let r = other.rect;
        self.clip.is_none_or(|c| c[0] <= r[0] + 1.0 && c[1] <= r[1] + 1.0 && c[2] >= r[2] - 1.0 && c[3] >= r[3] - 1.0)
    }
}

impl TextLine {
    /// Origin of the source baseline in PDF user space, independent of font ascent/descent.
    pub fn baseline(&self) -> [f64; 2] {
        [self.origin.x, self.origin.baseline]
    }

    /// Whether the line can be seen (it is drawn with some opacity, not invisibly, and isn't
    /// clipped away). Paragraphs ([`text_blocks`]) hold only visible lines.
    pub fn is_visible(&self) -> bool {
        self.visible
    }

    /// Leading used when a single source run wraps into additional rows.
    pub fn wrap_leading(&self) -> f64 {
        if self.origin.state.leading > 0.0 { self.origin.state.leading * self.origin.k } else { self.size * 1.2 }
    }
}

/// An embedded sfnt source face, read only for the inline editor. Bytes stay in memory and
/// are never installed, copied into assets, or used as a font for another document.
#[derive(Clone, Debug)]
pub struct TextEditorFont {
    pub resource: String,
    pub face: String,
    pub bytes: Vec<u8>,
}

pub fn text_editor_fonts(doc: &Document, page: usize) -> Result<Vec<TextEditorFont>, EditError> {
    // The fonts of text drawn in forms too, by the names `text_lines` gives them.
    let view = crate::inline_forms::view(doc, page)?;
    let doc: &Document = &view;
    let p = page_dict(doc, page)?;
    let resources = p.dict.get(b"Resources").and_then(|o| doc.dict(o)).unwrap_or_default();
    let fonts = resources.get(b"Font").and_then(|o| doc.dict(o)).unwrap_or_default();
    let mut budget = 32_usize << 20;
    let mut result = Vec::new();
    for (name, font) in fonts.iter().take(128) {
        let Some(font) = doc.dict(font) else { continue };
        let Some(bytes) = pdfcraft_fonts::pdf::embedded_sfnt(doc, &font) else { continue };
        if bytes.len() > budget {
            continue;
        }
        budget -= bytes.len();
        result.push(TextEditorFont {
            resource: String::from_utf8_lossy(name).into_owned(),
            face: Metrics::from_dict(doc, &font).base_font.rsplit('+').next().unwrap_or_default().to_owned(),
            bytes,
        });
    }
    // The approved full face follows all source subsets in the editor's font stack. It can
    // display newly inserted glyphs before Apply without changing the source text's face.
    let source_faces: Vec<_> = result.iter().map(|p| p.face.clone()).collect();
    for face in source_faces {
        for full in pdfcraft_fonts::embedding::matching_faces(&face) {
            if full.bytes.len() > budget || result.iter().any(|p| p.resource == format!("PCApproved{}", full.name())) {
                continue;
            }
            budget -= full.bytes.len();
            result.push(TextEditorFont { resource: format!("PCApproved{}", full.name()), face: face.clone(), bytes: full.bytes.to_vec() });
        }
    }
    Ok(result)
}

/// Load an explicit full face into this page's resources. Nothing on the page is replaced;
/// the new resource can complete matching source subsets or be chosen in a text style.
pub fn register_text_font(doc: &mut Document, page: usize, bytes: &[u8]) -> Result<String, EditError> {
    pdfcraft_fonts::embedding::validate_document_input(bytes).map_err(EditError::Invalid)?;
    let (_, postscript) = pdfcraft_fonts::embedding::names(bytes).map_err(EditError::Invalid)?;
    let p = page_dict(doc, page)?;
    let mut resources = p.dict.get(b"Resources").and_then(|o| doc.dict(o)).unwrap_or_default();
    let mut fonts = resources.get(b"Font").and_then(|o| doc.dict(o)).unwrap_or_default();
    let name = pdfcraft_fonts::embedding::embed(doc, &mut fonts, bytes, &postscript).map_err(EditError::Invalid)?;
    resources.set(b"Font".to_vec(), Object::Dict(fonts));
    doc.update_dict(p.obj, |d| d.set(b"Resources".to_vec(), Object::Dict(resources)))?;
    Ok(name)
}

/// Read-only draft bounds, measured with the same PDF rewrite used by Apply/Save.
#[derive(Clone, Debug, PartialEq)]
pub struct TextEditLayout {
    pub source_rect: [f64; 4],
    pub draft_rect: Option<[f64; 4]>,
    pub overflow: bool,
}

pub fn text_edit_layout(doc: &Document, page: usize, block: usize, text: &str, style: &BlockStyle) -> Result<TextEditLayout, EditError> {
    let original = text_blocks(doc, page)?;
    let source = original.get(block).ok_or(EditError::Invalid("text block does not exist".into()))?;
    if text == source.editor_text && *style == BlockStyle::default() {
        return Ok(TextEditLayout { source_rect: source.rect, draft_rect: Some(source.rect), overflow: false });
    }
    // Subtract the untouched runs by value, rather than by operator offsets: a rewrite may
    // split BT/ET or add font resources. A multiset also handles coincident repeated text.
    let key =
        |line: &TextLine| (line.text.clone(), line.font.clone(), line.rect.map(f64::to_bits), line.size.to_bits(), line.color.map(f64::to_bits));
    let mut untouched = std::collections::BTreeMap::new();
    for (index, line) in text_lines(doc, page)?.iter().enumerate() {
        if !source.lines.contains(&index) {
            *untouched.entry(key(line)).or_insert(0_usize) += 1;
        }
    }
    let mut draft = doc.clone();
    rewrite_block(&mut draft, page, block, Some(text), style)?;
    let mut rect: Option<[f64; 4]> = None;
    for line in text_lines(&draft, page)? {
        if let Some(count) = untouched.get_mut(&key(&line))
            && *count > 0
        {
            *count -= 1;
            continue;
        }
        rect = Some(rect.map_or(line.rect, |r| [r[0].min(line.rect[0]), r[1].min(line.rect[1]), r[2].max(line.rect[2]), r[3].max(line.rect[3])]));
    }
    let overflow = rect
        .is_some_and(|r| r[0] < source.rect[0] - 0.5 || r[1] < source.rect[1] - 0.5 || r[2] > source.rect[2] + 0.5 || r[3] > source.rect[3] + 0.5);
    Ok(TextEditLayout { source_rect: source.rect, draft_rect: rect, overflow })
}

/// Text and graphics state captured for rewriting.
#[derive(Clone, Debug, PartialEq)]
struct TextState {
    font: Option<(String, f64)>,
    char_spacing: f64,
    word_spacing: f64,
    scale: f64,
    leading: f64,
    rise: f64,
    fill: Vec<Op>,
}

impl TextState {
    fn of(ts: &Ts) -> Self {
        TextState {
            font: ts.font.as_ref().map(|(n, _)| (String::from_utf8_lossy(n).into_owned(), ts.size)),
            char_spacing: ts.char_spacing,
            word_spacing: ts.word_spacing,
            scale: ts.scale,
            leading: ts.leading,
            rise: ts.rise,
            fill: ts.fill.clone(),
        }
    }

    /// Operators that set this state (inside a text object).
    fn ops(&self) -> Vec<Op> {
        let n = pdfcraft_content::num;
        let mut v = Vec::new();
        if let Some((f, size)) = &self.font {
            v.push(Op::new("Tf", vec![Object::name(f), n(*size)]));
        }
        v.push(Op::new("Tc", vec![n(self.char_spacing)]));
        v.push(Op::new("Tw", vec![n(self.word_spacing)]));
        v.push(Op::new("Tz", vec![n(self.scale * 100.0)]));
        v.push(Op::new("TL", vec![n(self.leading)]));
        v.push(Op::new("Ts", vec![n(self.rise)]));
        v.extend(self.fill.iter().cloned());
        v
    }
}

#[derive(Clone, Debug, PartialEq)]
struct Origin {
    tm: [f64; 6],
    /// The text line matrix there (where the next line's Td/T* starts from).
    tlm: [f64; 6],
    /// Text-space units → user space (the scale of the text rendering matrix's y axis).
    k: f64,
    /// The CTM there (current space → page space), for moving the paragraph in page space.
    ctm: [f64; 6],
    baseline: f64,
    x: f64,
    state: TextState,
    bt_op: usize,
    bt_state: TextState,
}

/// An editable text region: adjacent runs on visual rows, followed by aligned rows.
/// A region may contain several source fonts, sizes, colours and weights.
#[derive(Clone, Debug, PartialEq)]
pub struct TextBlock {
    /// The lines' text joined with spaces (lines that ended in a hyphen are joined directly).
    pub text: String,
    pub rect: [f64; 4],
    pub base_font: String,
    pub size: f64,
    pub bold: bool,
    pub italic: bool,
    /// The text fill colour as RGB, used by the visual editor and preserved when no colour
    /// override is requested.
    pub color: [f64; 3],
    /// Counterclockwise baseline angle in page user space, in degrees.
    pub rotation: f64,
    /// Indexes into [`text_lines`].
    pub lines: Vec<usize>,
    /// Visual rows; each contains indexes into [`text_lines`], including mixed-style runs.
    pub rows: Vec<Vec<usize>>,
    /// Text for the inline editor, with a newline only between visual rows.
    pub editor_text: String,
    /// Byte ranges in `editor_text` and their source line indexes.
    pub spans: Vec<(std::ops::Range<usize>, usize)>,
    /// Each visual row's baseline height in user space, top row first.
    pub baselines: Vec<f64>,
}

impl TextBlock {
    /// Preserve styles outside the changed range; inserted characters inherit the surrounding
    /// source run. Byte ranges are always UTF-8 boundaries. Linear time, including large edits.
    pub fn edited_spans(&self, draft: &str) -> Vec<(std::ops::Range<usize>, usize)> {
        if self.editor_text.len() > 4_000_000 || draft.len() > 4_000_000 {
            return vec![(0..draft.len(), self.lines.first().copied().unwrap_or(0))];
        }
        let old: Vec<_> = self.editor_text.chars().collect();
        let new: Vec<_> = draft.char_indices().collect();
        // A row break typed as a space (text written on one line) is the same separator: the
        // words after it keep their own style instead of taking the one before the break.
        let same = |a: &char, b: &char| a == b || (matches!(a, '\n' | ' ') && matches!(b, '\n' | ' '));
        let prefix = old.iter().zip(&new).take_while(|(a, (_, b))| same(a, b)).count();
        let suffix =
            old.iter().rev().zip(new.iter().rev()).take(old.len().min(new.len()).saturating_sub(prefix)).take_while(|(a, (_, b))| same(a, b)).count();
        let mut original = Vec::with_capacity(old.len());
        let mut span = 0;
        for (byte, _) in self.editor_text.char_indices() {
            while self.spans.get(span).is_some_and(|(r, _)| byte >= r.end) {
                span += 1;
            }
            original.push(self.spans.get(span).or_else(|| self.spans.last()).map_or(0, |(_, i)| *i));
        }
        let inherited = original.get(prefix.saturating_sub(1)).or_else(|| original.get(prefix)).copied().unwrap_or(0);
        let mut result: Vec<(std::ops::Range<usize>, usize)> = Vec::new();
        for (i, (byte, ch)) in new.iter().enumerate() {
            let source = if i < prefix {
                original.get(i).copied()
            } else if i >= new.len().saturating_sub(suffix) {
                original.get(old.len().saturating_sub(new.len() - i)).copied()
            } else {
                Some(inherited)
            }
            .unwrap_or(inherited);
            let end = byte.saturating_add(ch.len_utf8());
            if let Some((range, _)) = result.last_mut().filter(|(_, prev)| *prev == source) {
                range.end = end;
            } else {
                result.push((*byte..end, source));
            }
        }
        result
    }
}

impl TextLine {
    /// The baseline's height in user space.
    pub fn origin_baseline(&self) -> f64 {
        self.origin.baseline
    }

    /// Paint position of the line's last text-showing operator: `(content-stream index,
    /// operator index in the page's joined stream)`. Lines paint in order of this tuple.
    /// (`text_lines` reads form XObjects' text as inlined into the page's streams, so `stream`
    /// is always a page content-stream index and `ops` are indices into the joined operator list.)
    pub fn paint_pos(&self) -> (usize, usize) {
        (self.stream, self.ops.iter().max().copied().unwrap_or(0))
    }

    fn rotation(&self) -> f64 {
        let m = Matrix(self.origin.tm).then(&Matrix(self.origin.ctm)).0;
        m[1].atan2(m[0]).to_degrees()
    }
}

/// What replacing a line did.
#[derive(Clone, Debug, PartialEq)]
pub struct LineEdit {
    /// `Some(font)` when the line's font couldn't show the new text and Helvetica was used.
    pub substituted: Option<String>,
}

#[derive(Clone)]
struct Ts {
    ctm: Matrix,
    /// The operators that set the current fill colour (`g`, `rg`, `k`, or `cs` + `sc`/`scn`).
    fill: Vec<Op>,
    font: Option<(Vec<u8>, Rc<Metrics>)>,
    /// The Japanese or Hebrew fallback face the font subsets, when the font is one (see [`is_type3_fallback`]).
    face: Option<String>,
    size: f64,
    char_spacing: f64,
    word_spacing: f64,
    scale: f64,
    leading: f64,
    rise: f64,
    render_mode: i64,
    /// The fill alpha (`ca` of the last `gs`); text drawn at 0 is invisible.
    alpha: f64,
    /// The stroke alpha (`CA`), for stroked text (rendering modes 1, 2, 5, 6).
    stroke_alpha: f64,
    /// The clipping area's bounds in user space (`None`: unclipped). A bound, not the exact
    /// area: text outside it is certainly clipped away.
    clip: Option<[f64; 4]>,
}

/// Whether a run paints anything a reader can see: not drawn at zero alpha, not invisible
/// (rendering mode 3) or clip-only (mode 7), and not clipped away entirely.
fn shown_visible(s: &Shown) -> bool {
    let st = &s.state;
    let painted = match st.render_mode {
        0 | 4 => st.alpha > 0.0,
        1 | 5 => st.stroke_alpha > 0.0,
        2 | 6 => st.alpha > 0.0 || st.stroke_alpha > 0.0,
        3 | 7 => false,
        _ => true,
    };
    let r = s.rect;
    painted && st.clip.is_none_or(|c| c[0] < c[2] && c[1] < c[3] && c[0] < r[2] && c[2] > r[0] && c[1] < r[3] && c[3] > r[1])
}

pub(crate) fn content_streams(doc: &Document, page: &Dict) -> Vec<(Object, Vec<u8>)> {
    let list: Vec<Object> = match page.get(b"Contents") {
        None => Vec::new(),
        Some(c) => match &*doc.resolve(c) {
            Object::Array(a) => a.clone(),
            _ => vec![c.clone()],
        },
    };
    list.into_iter()
        .map(|o| {
            let data = match &*doc.resolve(&o) {
                Object::Stream(s) => s.decoded().unwrap_or_default(),
                _ => Vec::new(),
            };
            (o, data)
        })
        .collect()
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

fn page_dict(doc: &Document, page: usize) -> Result<pdfcraft_model::Page, EditError> {
    pdfcraft_model::pages(doc).into_iter().nth(page).ok_or(EditError::NoSuchPage(page))
}

fn fill_color(fill: &[Op]) -> [f64; 3] {
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
fn fill_rgb(doc: &Document, colour_spaces: &Dict, fill: &[Op]) -> Option<[f64; 3]> {
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

/// The text-showing operators of one stream with their text, font, box and baseline.
#[derive(Clone)]
struct Shown {
    op: usize,
    extra_ops: Vec<usize>,
    /// Advance in text space, including spacing and TJ adjustments.
    advance: f64,
    /// The text matrix where it starts (text space), and the state there and at its `BT`.
    tm: Matrix,
    tlm: Matrix,
    state: Ts,
    bt_op: usize,
    bt_state: Ts,
    text: String,
    rect: [f64; 4],
    baseline: f64,
    start_x: f64,
    end_x: f64,
    bt: usize,
    font: Vec<u8>,
    base_font: String,
    face: Option<String>,
    size: f64,
    bold: bool,
    italic: bool,
    decodable: bool,
    scope: Option<usize>,
    /// Each decoded glyph's text and where it starts and ends along the run, in user space x
    /// (its own advance, without character or word spacing).
    glyphs: Vec<(String, f64, f64)>,
}

/// The graphics state carried from one of a page's content streams to the next: the streams
/// are one stream in pieces (§7.8.2), so a `cm` (AutoCAD scales the whole page in the first
/// stream), an unbalanced `q`, the font and the colour still apply in the streams after it.
struct Carry {
    ts: Ts,
    stack: Vec<Ts>,
    marked: Vec<Option<usize>>,
    /// The `ExtGState` resources `gs` names (only reading tracks fill alpha).
    states: Option<Dict>,
}

impl Carry {
    fn new() -> Self {
        let ts = Ts {
            ctm: Matrix::IDENTITY,
            fill: Vec::new(),
            font: None,
            face: None,
            size: 0.0,
            char_spacing: 0.0,
            word_spacing: 0.0,
            scale: 1.0,
            leading: 0.0,
            rise: 0.0,
            render_mode: 0,
            alpha: 1.0,
            stroke_alpha: 1.0,
            clip: None,
        };
        Carry { ts, stack: Vec::new(), marked: Vec::new(), states: None }
    }
}

/// A `Do` met while interpreting: its operator index, XObject name and the state there.
type Invocation = (usize, Vec<u8>, Ts);

fn interpret(
    doc: &Document,
    ops: &[Op],
    fonts_res: &Dict,
    cache: &mut HashMap<Vec<u8>, (Rc<Metrics>, Option<String>)>,
    carry: &mut Carry,
    mut invoked: Option<&mut Vec<Invocation>>,
    logical_text: bool,
) -> Vec<Shown> {
    let ext_gstate = carry.states.clone();
    let ext_gstate = ext_gstate.as_ref();
    let mut out: Vec<Shown> = Vec::new();
    let mut ts = carry.ts.clone();
    let mut at_bt = (0usize, ts.clone());
    let mut stack: Vec<Ts> = std::mem::take(&mut carry.stack);
    let (mut tm, mut tlm) = (Matrix::IDENTITY, Matrix::IDENTITY);
    let mut bt = 0usize;
    let mut marked = std::mem::take(&mut carry.marked);
    let mut actual: Vec<Option<(usize, usize, String)>> = Vec::new();
    let mut path_box: Option<[f64; 4]> = None;
    let mut clip_pending = false;
    for (i, op) in ops.iter().enumerate() {
        match op.op.as_slice() {
            b"BMC" | b"BDC" => {
                if marked.len() < 256 {
                    // Marked text paragraphs/headings are authoritative boundaries. Ignore
                    // artifacts and inner spans, which inherit their enclosing region.
                    let region = matches!(op.name(0), Some(b"P" | b"H" | b"H1" | b"H2" | b"H3" | b"H4" | b"H5" | b"H6" | b"LI" | b"TD" | b"TH"));
                    marked.push(if region { Some(i) } else { marked.last().copied().flatten() });
                    let logical = op.operands.get(1).and_then(Object::as_dict).and_then(|d| d.get(b"ActualText")).and_then(|s| match s {
                        Object::String(s) if s.bytes.len() <= 4_000_000 => Some(s.to_text()),
                        _ => None,
                    });
                    actual.push(if logical_text { logical.map(|s| (out.len(), i, s)) } else { None });
                }
            }
            b"EMC" => {
                marked.pop();
                if let Some(Some((start, open, logical))) = actual.pop() {
                    let tail = out.split_off(start.min(out.len()));
                    let mut items = tail.into_iter();
                    if let Some(mut first) = items.next() {
                        for shown in items {
                            first.extra_ops.push(shown.op);
                            first.extra_ops.extend(shown.extra_ops);
                            first.rect = [
                                first.rect[0].min(shown.rect[0]),
                                first.rect[1].min(shown.rect[1]),
                                first.rect[2].max(shown.rect[2]),
                                first.rect[3].max(shown.rect[3]),
                            ];
                        }
                        first.extra_ops.extend([open, i]);
                        first.glyphs = vec![(logical.clone(), first.rect[0], first.rect[2])];
                        first.text = logical;
                        first.start_x = first.rect[0];
                        first.end_x = first.rect[2];
                        first.advance = (first.end_x - first.start_x) / first.tm.then(&first.state.ctm).0[0].abs().max(1e-6);
                        out.push(first);
                    }
                }
            }
            b"q" => stack.push(ts.clone()),
            b"Q" => {
                if let Some(s) = stack.pop() {
                    ts = s;
                }
            }
            b"cm" => {
                if let Some(m) = op.nums::<6>() {
                    ts.ctm = Matrix(m).then(&ts.ctm);
                }
            }
            b"BT" => {
                tm = Matrix::IDENTITY;
                tlm = Matrix::IDENTITY;
                bt += 1;
                at_bt = (i, ts.clone());
            }
            b"g" | b"rg" | b"k" | b"cs" => ts.fill = vec![op.clone()],
            b"sc" | b"scn" => {
                ts.fill.retain(|o| o.is("cs"));
                ts.fill.push(op.clone());
            }
            b"Tf" => {
                ts.size = op.num(1).unwrap_or(ts.size);
                if let Some(name) = op.name(0) {
                    let (m, face) = cache.entry(name.to_vec()).or_insert_with(|| {
                        let dict = fonts_res.get(name).and_then(|f| doc.resolve(f).as_dict().cloned());
                        let metrics = dict.as_ref().map_or_else(Metrics::fallback, |d| Metrics::from_dict(doc, d));
                        let face = dict.filter(is_type3_fallback).map(|_| metrics.base_font.clone());
                        (Rc::new(metrics), face)
                    });
                    ts.font = Some((name.to_vec(), m.clone()));
                    ts.face = face.clone();
                }
            }
            b"Tc" => ts.char_spacing = op.num(0).unwrap_or(0.0),
            b"Tw" => ts.word_spacing = op.num(0).unwrap_or(0.0),
            b"Tz" => ts.scale = op.num(0).unwrap_or(100.0) / 100.0,
            b"TL" => ts.leading = op.num(0).unwrap_or(0.0),
            b"Ts" => ts.rise = op.num(0).unwrap_or(0.0),
            b"Tr" => ts.render_mode = op.operands.first().and_then(Object::as_int).unwrap_or(-1),
            b"gs" => {
                let state = ext_gstate.zip(op.name(0)).and_then(|(states, name)| states.get(name)).and_then(|s| doc.dict(s));
                let alpha = |key: &[u8]| state.as_ref().and_then(|s| s.get(key).and_then(|a| doc.resolve(a).as_f64())).filter(|a| a.is_finite());
                if let Some(a) = alpha(b"ca") {
                    ts.alpha = a.clamp(0.0, 1.0);
                }
                if let Some(a) = alpha(b"CA") {
                    ts.stroke_alpha = a.clamp(0.0, 1.0);
                }
            }
            // Path construction, for the clipping bound (`W`/`W*` take effect at the painting op).
            b"m" | b"l" | b"c" | b"v" | b"y" => {
                for [x, y] in op.operands.as_chunks::<2>().0 {
                    if let (Some(x), Some(y)) = (x.as_f64(), y.as_f64()) {
                        let (x, y) = ts.ctm.apply(x, y);
                        let b = path_box.get_or_insert([x, y, x, y]);
                        *b = [b[0].min(x), b[1].min(y), b[2].max(x), b[3].max(y)];
                    }
                }
            }
            b"re" => {
                if let Some([x, y, w, h]) = op.nums::<4>() {
                    let r = ts.ctm.bbox([x.min(x + w), y.min(y + h), x.max(x + w), y.max(y + h)]);
                    let b = path_box.get_or_insert(r);
                    *b = [b[0].min(r[0]), b[1].min(r[1]), b[2].max(r[2]), b[3].max(r[3])];
                }
            }
            b"W" | b"W*" => clip_pending = true,
            b"n" | b"S" | b"s" | b"f" | b"F" | b"f*" | b"B" | b"B*" | b"b" | b"b*" => {
                if std::mem::take(&mut clip_pending) {
                    let area = path_box.unwrap_or([0.0; 4]);
                    let clip = match ts.clip {
                        Some(c) => [c[0].max(area[0]), c[1].max(area[1]), c[2].min(area[2]), c[3].min(area[3])],
                        None => area,
                    };
                    // An empty or unreadable intersection clips everything away.
                    ts.clip = Some(if clip.iter().all(|v| v.is_finite()) { clip } else { [0.0; 4] });
                }
                path_box = None;
            }
            b"Do" => {
                if let (Some(list), Some(name)) = (invoked.as_deref_mut(), op.name(0)) {
                    list.push((i, name.to_vec(), ts.clone()));
                }
            }
            b"Td" | b"TD" => {
                if let Some([x, y]) = op.nums::<2>() {
                    if op.is("TD") {
                        ts.leading = -y;
                    }
                    tlm = Matrix([1.0, 0.0, 0.0, 1.0, x, y]).then(&tlm);
                    tm = tlm;
                }
            }
            b"Tm" => {
                if let Some(m) = op.nums::<6>() {
                    tlm = Matrix(m);
                    tm = tlm;
                }
            }
            b"T*" => {
                tlm = Matrix([1.0, 0.0, 0.0, 1.0, 0.0, -ts.leading]).then(&tlm);
                tm = tlm;
            }
            b"Tj" | b"TJ" | b"'" | b"\"" => {
                if matches!(op.op.as_slice(), b"'" | b"\"") {
                    if op.is("\"") {
                        ts.word_spacing = op.num(0).unwrap_or(ts.word_spacing);
                        ts.char_spacing = op.num(1).unwrap_or(ts.char_spacing);
                    }
                    tlm = Matrix([1.0, 0.0, 0.0, 1.0, 0.0, -ts.leading]).then(&tlm);
                    tm = tlm;
                }
                let Some((name, m)) = ts.font.clone() else { continue };
                // Pieces: strings, and TJ number adjustments.
                let pieces: Vec<Object> = match op.op.as_slice() {
                    b"TJ" => op.operands.first().and_then(Object::as_array).cloned().unwrap_or_default(),
                    _ => op.operands.last().cloned().into_iter().collect(),
                };
                let trm0 = tm.then(&ts.ctm);
                let start = trm0.apply(0.0, ts.rise);
                let mut text = String::new();
                let mut glyphs = Vec::new();
                let mut decodable = true;
                let mut x_text = 0.0;
                for p in &pieces {
                    match p {
                        Object::String(s) => {
                            for (code, len) in m.codes(&s.bytes) {
                                match m.text_of(code) {
                                    Some(t) => {
                                        let own = m.width(code) * ts.size * ts.scale;
                                        if glyphs.len() < MAX_RUN_GLYPHS {
                                            glyphs.push((t.to_string(), trm0.apply(x_text, ts.rise).0, trm0.apply(x_text + own, ts.rise).0));
                                        }
                                        text.push_str(&t);
                                    }
                                    None => decodable = false,
                                }
                                let w = m.width(code) * ts.size + ts.char_spacing + if m.is_space(code, len) { ts.word_spacing } else { 0.0 };
                                x_text += w * ts.scale;
                            }
                        }
                        other => {
                            if let Some(n) = other.as_f64() {
                                let dx = -n / 1000.0 * ts.size * ts.scale;
                                // A large gap inside TJ reads as a space.
                                if dx > ts.size * 0.2 && !text.ends_with(' ') {
                                    text.push(' ');
                                }
                                x_text += dx;
                            }
                        }
                    }
                }
                tm = Matrix([1.0, 0.0, 0.0, 1.0, x_text, 0.0]).then(&tm);
                let end = tm.then(&ts.ctm).apply(0.0, ts.rise);
                // The box: from descent to ascent along the run.
                let corners = [
                    trm0.apply(0.0, ts.rise + m.descent * ts.size),
                    trm0.apply(x_text, ts.rise + m.descent * ts.size),
                    trm0.apply(0.0, ts.rise + m.ascent * ts.size),
                    trm0.apply(x_text, ts.rise + m.ascent * ts.size),
                ];
                let rect = corners
                    .iter()
                    .fold([f64::MAX, f64::MAX, f64::MIN, f64::MIN], |b, p| [b[0].min(p.0), b[1].min(p.1), b[2].max(p.0), b[3].max(p.1)]);
                let size_user = (trm0.0[2].powi(2) + trm0.0[3].powi(2)).sqrt() * ts.size;
                // A numeric-only TJ adjusts the current text matrix but paints no glyphs.
                // It must not create a phantom editable line or enlarge its neighbours' boxes.
                if !pieces.iter().any(|p| matches!(p, Object::String(s) if !s.bytes.is_empty())) {
                    continue;
                }
                out.push(Shown {
                    op: i,
                    extra_ops: Vec::new(),
                    advance: x_text,
                    tm: Matrix([tm.0[0], tm.0[1], tm.0[2], tm.0[3], tm.0[4] - x_text * tm.0[0], tm.0[5] - x_text * tm.0[1]]),
                    state: ts.clone(),
                    tlm,
                    bt_op: at_bt.0,
                    bt_state: at_bt.1.clone(),
                    text,
                    rect,
                    baseline: start.1,
                    start_x: start.0,
                    end_x: end.0,
                    bt,
                    font: name,
                    base_font: m.base_font.clone(),
                    face: ts.face.clone(),
                    size: if size_user > 0.0 { size_user } else { ts.size },
                    bold: m.bold,
                    italic: m.italic,
                    decodable,
                    scope: marked.last().copied().flatten(),
                    glyphs,
                });
            }
            _ => {}
        }
    }
    carry.ts = ts;
    carry.stack = stack;
    carry.marked = marked;
    out
}

/// The lines of text on a page (0-based), in content order. Text the page draws in form
/// XObjects is included as if the forms were drawn inline, which is how every rewrite here
/// writes such a page (see `inline_forms`).
pub fn text_lines(doc: &Document, page: usize) -> Result<Vec<TextLine>, EditError> {
    text_lines_in(crate::inline_forms::view(doc, page)?.as_ref(), page)
}

/// [`text_lines`] of the page's own content streams.
fn text_lines_in(doc: &Document, page: usize) -> Result<Vec<TextLine>, EditError> {
    let p = page_dict(doc, page)?;
    let res = p.dict.get(b"Resources").map(|r| doc.resolve(r)).and_then(|r| r.as_dict().cloned()).unwrap_or_default();
    let fonts_res = res.get(b"Font").map(|f| doc.resolve(f)).and_then(|f| f.as_dict().cloned()).unwrap_or_default();
    let mut cache = HashMap::new();
    let mut lines: Vec<TextLine> = Vec::new();
    let mut carry = Carry::new();
    // Opacity decides which text can be seen.
    carry.states = res.get(b"ExtGState").and_then(|s| doc.dict(s));
    // The streams are one stream in pieces (§7.8.2), split between any two tokens: parse them
    // joined, so an operator whose operands end one piece still has them.
    let pieces = Pieces::join(&content_streams(doc, &p.dict).into_iter().map(|(_, d)| d).collect::<Vec<_>>());
    let ops = pieces.parse().ops;
    let shown = interpret(doc, &ops, &fonts_res, &mut cache, &mut carry, None, true);
    let mut last: Option<(usize, usize, f64, f64, f64)> = None; // (piece, bt, baseline, end_x, size)
    for s in shown {
        // A line belongs to one stream piece. Within it, PDF producers often put every word
        // in its own BT/ET: geometry and text state, not object boundaries, define a line.
        let si = ops.get(s.op).map_or(0, |op| pieces.pieces_of(op).0);
        let visible = shown_visible(&s);
        let joins = last.is_some_and(|(piece, bt, base, end, size)| {
            piece == si
                && lines.last().is_some_and(|l| l.scope == s.scope && l.visible == visible && l.alpha == s.state.alpha)
                && (bt == s.bt || lines.last().is_some_and(|l| compatible_word(l, &s)))
                && (s.text.trim().is_empty() || lines.last().is_some_and(|l| same_font(l, &s.font, s.face.as_deref())))
                && (s.text.trim().is_empty()
                    || lines.last().is_some_and(|l| {
                        (s.size - l.size).abs() < 0.01 && s.bold == l.bold && s.italic == l.italic && fill_color(&s.state.fill) == l.color
                    }))
                && (s.baseline - base).abs() < size * 0.3
                && s.start_x > end - size
                && s.start_x - end < size * if bt == s.bt { 3.0 } else { 1.5 }
        });
        if joins && let Some(l) = lines.last_mut() {
            let gap = s.start_x - last.map_or(s.start_x, |x| x.3);
            if gap > l.size * 0.2 && !l.text.ends_with(' ') && !s.text.starts_with(' ') {
                l.text.push(' ');
            }
            l.text.push_str(&s.text);
            l.rect = [l.rect[0].min(s.rect[0]), l.rect[1].min(s.rect[1]), l.rect[2].max(s.rect[2]), l.rect[3].max(s.rect[3])];
            l.ops.push(s.op);
            l.ops.extend(s.extra_ops.iter().copied());
            l.decodable &= s.decodable;
            // Spaces may use a separate subset font; they do not change the visible style.
        } else {
            lines.push(TextLine {
                text: s.text.clone(),
                rect: s.rect,
                font: String::from_utf8_lossy(&s.font).into_owned(),
                base_font: s.base_font.clone(),
                size: s.size,
                bold: s.bold,
                italic: s.italic,
                color: fill_color(&s.state.fill),
                decodable: s.decodable,
                face: s.face.clone(),
                stream: si,
                scope: s.scope,
                visible,
                alpha: s.state.alpha,
                clip: s.state.clip,
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
        last = Some((si, s.bt, s.baseline, s.end_x, s.size.max(1.0)));
    }
    // Content streams draw right-to-left lines in visual order; the editor works in typing order.
    let colour_spaces = res.get(b"ColorSpace").map(|c| doc.resolve(c)).and_then(|c| c.as_dict().cloned()).unwrap_or_default();
    for l in &mut lines {
        l.text = crate::bidi::to_logical(&l.text);
        // A colour set through a colour space (Chrome writes ICC-based sRGB) reads as itself.
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
    let same = |a: &[f64], b: &[f64]| a.iter().zip(b).all(|(a, b)| (a - b).abs() < 1e-6);
    let (a, b) = (&line.origin.state, TextState::of(&shown.state));
    line.scope == shown.scope
        && same(&line.origin.tm[..4], &shown.tm.0[..4])
        && same(&line.origin.ctm, &shown.state.ctm.0)
        && same(&[a.char_spacing, a.word_spacing, a.scale, a.rise], &[b.char_spacing, b.word_spacing, b.scale, b.rise])
        && a.fill.len() == b.fill.len()
        && a.fill.iter().zip(&b.fill).all(|(a, b)| a.op == b.op && a.operands == b.operands)
}

/// The line `s` continues, or a new one: `last` is the previous run of the same stream
/// (`bt`, baseline, end x, size).
fn add_shown(lines: &mut Vec<TextLine>, last: &mut Option<(usize, f64, f64, f64)>, s: Shown, stream: usize) {
    let joins = last.is_some_and(|(bt, base, end, size)| {
        lines.last().is_some_and(|l| l.scope == s.scope)
            && (bt == s.bt || lines.last().is_some_and(|l| compatible_word(l, &s)))
            && (s.text.trim().is_empty() || lines.last().is_some_and(|l| same_font(l, &s.font, s.face.as_deref())))
            && (s.text.trim().is_empty()
                || lines.last().is_some_and(|l| {
                    (s.size - l.size).abs() < 0.01 && s.bold == l.bold && s.italic == l.italic && fill_color(&s.state.fill) == l.color
                }))
            && (s.baseline - base).abs() < size * 0.3
            && s.start_x > end - size
            && s.start_x - end < size * if bt == s.bt { 3.0 } else { 1.5 }
    });
    if joins && let Some(l) = lines.last_mut() {
        let gap = s.start_x - last.map_or(s.start_x, |x| x.2);
        if gap > l.size * 0.2 && !l.text.ends_with(' ') && !s.text.starts_with(' ') {
            l.text.push(' ');
        }
        l.text.push_str(&s.text);
        l.rect = [l.rect[0].min(s.rect[0]), l.rect[1].min(s.rect[1]), l.rect[2].max(s.rect[2]), l.rect[3].max(s.rect[3])];
        l.ops.push(s.op);
        l.ops.extend(s.extra_ops.iter().copied());
        l.decodable &= s.decodable;
    } else {
        lines.push(TextLine {
            text: s.text.clone(),
            rect: s.rect,
            font: String::from_utf8_lossy(&s.font).into_owned(),
            base_font: s.base_font.clone(),
            size: s.size,
            bold: s.bold,
            italic: s.italic,
            color: fill_color(&s.state.fill),
            decodable: s.decodable,
            face: s.face.clone(),
            stream,
            scope: s.scope,
            visible: shown_visible(&s),
            alpha: s.state.alpha,
            clip: s.state.clip,
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
    *last = Some((s.bt, s.baseline, s.end_x, s.size.max(1.0)));
}

/// How deep form XObjects nest before reading stops (they may also refer to themselves).
pub(crate) const MAX_FORM_DEPTH: usize = 12;

/// The most glyph positions one text-showing operator records (reading rebuilds lines from them).
const MAX_RUN_GLYPHS: usize = 100_000;

/// The most form XObjects one page's walk enters, at any depth. The depth cap alone still lets
/// a form that draws another form many times fan out exponentially (100 `Do`s on each of 12
/// levels), which would hang export on a hostile PDF.
pub(crate) const MAX_FORM_VISITS: usize = 4096;

/// A form XObject a page draws: its object, content, resources and its matrix composed with the
/// CTM at the `Do`.
pub(crate) struct FormCall {
    pub(crate) obj: Option<pdfcraft_cos::ObjRef>,
    pub(crate) data: Vec<u8>,
    pub(crate) resources: Dict,
    pub(crate) ctm: Matrix,
}

/// The form XObject `name` in `resources`, drawn with `ctm`. `None` for images, missing names
/// and forms already being read (`path`), nested too deep, or past the page's `visits` budget
/// ([`MAX_FORM_VISITS`]); each form entered counts one visit.
pub(crate) fn form_call(
    doc: &Document,
    resources: &Dict,
    name: &[u8],
    ctm: Matrix,
    path: &[pdfcraft_cos::ObjRef],
    visits: &mut usize,
) -> Option<FormCall> {
    if path.len() >= MAX_FORM_DEPTH || *visits >= MAX_FORM_VISITS {
        return None;
    }
    let xobjects = resources.get(b"XObject").map(|x| doc.resolve(x)).and_then(|x| x.as_dict().cloned())?;
    let o = xobjects.get(name)?;
    let obj = o.as_ref();
    if obj.is_some_and(|r| path.contains(&r)) {
        return None;
    }
    let Object::Stream(s) = &*doc.resolve(o) else { return None };
    if s.dict.name(b"Subtype") != Some(b"Form") {
        return None;
    }
    let m = s.dict.get(b"Matrix").map(|m| doc.resolve(m)).and_then(|m| {
        let a = m.as_array()?;
        let v: Vec<f64> = a.iter().filter_map(|x| doc.resolve(x).as_f64()).collect();
        <[f64; 6]>::try_from(v).ok().filter(|v| v.iter().all(|x| x.is_finite()))
    });
    // A form without its own resources uses the ones it was drawn with (PDF 1.1 files).
    let own = s.dict.get(b"Resources").map(|r| doc.resolve(r)).and_then(|r| r.as_dict().cloned());
    *visits = visits.saturating_add(1);
    Some(FormCall {
        obj,
        data: s.decoded().unwrap_or_default(),
        resources: own.unwrap_or_else(|| resources.clone()),
        ctm: m.map_or(ctm, |m| Matrix(m).then(&ctm)),
    })
}

/// The lines a page shows as it reads, including text drawn by form XObjects (which
/// [`text_lines`] leaves out because it can only rewrite the page's own streams). Lines from a
/// form carry a stream number past the page's streams, one per form drawn.
fn reading_lines(doc: &Document, page: usize) -> Result<Vec<TextLine>, EditError> {
    reading::reading_lines(doc, page)
}

pub use reading::remove_all_text;

/// Text → the runs that show it: each run's font resource name and the bytes shown in that font.
type Encoder = Box<dyn Fn(&str) -> Option<Vec<(String, Vec<u8>)>>>;

fn is_win_ansi_char(c: char) -> bool {
    c == '\t' || pdfcraft_fonts::win_ansi_byte(c).is_some()
}

fn needs_type3(text: &str) -> bool {
    text.chars().any(|c| !is_win_ansi_char(c))
}

/// The `/Name` of the Type 3 fonts generated for Japanese text.
const JAPANESE_FONT_NAME: &str = "PCJapanese";
/// The `/Name` of the Type 3 fonts generated for Hebrew text.
const HEBREW_FONT_NAME: &str = "PCHebrew";
/// The `/Name` of the Type 3 fonts generated from Inter, for Latin, Greek and Cyrillic text the
/// standard fonts can't hold (Turkish ş, Polish ł, …).
const INTER_FONT_NAME: &str = "PCInter";

/// Whether `font` is one of those generated fonts (Japanese, Hebrew or Inter): a replacement longer
/// than one of them is split across several, which read back as one line (see [`same_font`]).
fn is_type3_fallback(font: &Dict) -> bool {
    font.name(b"Subtype") == Some(b"Type3")
        && font.name(b"Name").is_some_and(|n| [JAPANESE_FONT_NAME, HEBREW_FONT_NAME, INTER_FONT_NAME].iter().any(|name| n == name.as_bytes()))
}

/// Whether `face` is the bundled Inter rather than a craft-fonts face.
fn is_inter(face: &CraftFont) -> bool {
    std::ptr::eq(face, &pdfcraft_fonts::INTER)
}

/// Whether a run shown in `font` (a resource name, and the Japanese or Hebrew fallback face it subsets, if
/// any) continues `line`: the same font resource, or another subset of the same face.
fn same_font(line: &TextLine, font: &[u8], face: Option<&str>) -> bool {
    line.font.as_bytes() == font || (line.face.is_some() && line.face.as_deref() == face)
}

fn source_family(base_font: &str) -> crate::added::Family {
    let name = base_font.to_ascii_lowercase();
    if ["courier", "mono", "consolas", "menlo", "monaco", "lucida console"].iter().any(|s| name.contains(s)) {
        crate::added::Family::Courier
    } else if !name.contains("sans")
        && ["times", "serif", "roman", "mincho", "cambria", "georgia", "palatino", "garamond"].iter().any(|s| name.contains(s))
    {
        crate::added::Family::Times
    } else {
        crate::added::Family::Helvetica
    }
}

const SUBSTITUTE_NAME: &str = "PCEdHelv";
const SUBSTITUTE: &[u8] = SUBSTITUTE_NAME.as_bytes();
const MAX_TYPE3_GLYPHS: usize = 240;

/// The generated fonts for one replacement: one per run of at most `max_glyphs` unique characters,
/// in order of first appearance.
#[derive(Clone)]
struct Type3Fallback {
    /// The family its glyphs come from ("Shippori Mincho").
    family: &'static str,
    /// The fonts' resource names in order; the first is `PCJp` (`PCHe` for Hebrew text).
    names: Vec<String>,
    /// Every unique character with its advance (em), in order of first appearance.
    widths: Vec<(char, f64)>,
    /// Where each character is shown: the index of its font in `names`, and its code there.
    codes: HashMap<char, (usize, u8)>,
}

/// One glyph of a generated font: its code there, its character, its glyph procedure and advance.
struct Type3Glyph {
    code: u8,
    ch: char,
    path: Vec<u8>,
    width: f64,
}

fn pdf_num(v: f64) -> String {
    if v.fract() == 0.0 { format!("{v:.0}") } else { format!("{v:.4}").trim_end_matches('0').trim_end_matches('.').to_string() }
}

fn type3_path(face: &CraftFont, ch: char) -> Result<(Vec<u8>, f64), EditError> {
    let glyph = japanese_glyph_from(face, ch).map_err(|e| match e {
        GlyphError::NoFont => no_japanese_font(),
        GlyphError::Missing => EditError::Invalid(format!("the {} fallback font has no glyph for U+{:04X}", face.family, ch as u32)),
        GlyphError::TooComplex => EditError::Invalid(format!("the {} fallback glyph U+{:04X} is too complex", face.family, ch as u32)),
    })?;
    // Inter's Greek and Cyrillic are proportional already.
    let (dx, width) = if is_inter(face) { (0.0, glyph.width) } else { proportional(ch, &glyph) };
    let scale = 1000.0;
    // d1 is `wx wy llx lly urx ury` (ISO 32000-2 §9.6.4) with a box enclosing the glyph;
    // Acrobat draws a bullet in place of a glyph whose d1 is malformed.
    let b = if glyph.contours.is_empty() { [0.0; 4] } else { glyph.bbox };
    let mut out = format!(
        "{} 0 {} {} {} {} d1\n",
        pdf_num(width * scale),
        pdf_num(((b[0] + dx) * scale).floor()),
        pdf_num((b[1] * scale).floor()),
        pdf_num(((b[2] + dx) * scale).ceil()),
        pdf_num((b[3] * scale).ceil())
    )
    .into_bytes();
    for contour in glyph.contours {
        let Some(first) = contour.first() else { continue };
        out.extend_from_slice(format!("{} {} m\n", pdf_num((first[0] + dx) * scale), pdf_num(first[1] * scale)).as_bytes());
        for p in contour.iter().skip(1) {
            out.extend_from_slice(format!("{} {} l\n", pdf_num((p[0] + dx) * scale), pdf_num(p[1] * scale)).as_bytes());
        }
        out.extend_from_slice(b"h\n");
    }
    out.extend_from_slice(b"f\n");
    Ok((out, width))
}

/// Side bearing (em) given to a respaced glyph on each side.
const SIDE_BEARING: f64 = 0.05;

/// Shift and advance for a fallback glyph. Japanese faces draw Cyrillic and Greek full-width
/// (one em each, as in JIS X 0208) and have no proportional (`palt`) metrics for them, so a
/// word set that way reads as letter-spaced. Those letters get their ink width plus a side
/// bearing instead; everything else keeps the face's own advance.
fn proportional(ch: char, glyph: &pdfcraft_fonts::GlyphOutline) -> (f64, f64) {
    let respace = matches!(ch, '\u{0370}'..='\u{03FF}' | '\u{0400}'..='\u{052F}' | '\u{1F00}'..='\u{1FFF}');
    let ink = glyph.bbox[2] - glyph.bbox[0];
    if !respace || glyph.contours.is_empty() || !(ink > 0.0 && ink < glyph.width) {
        return (0.0, glyph.width);
    }
    (SIDE_BEARING - glyph.bbox[0], ink + 2.0 * SIDE_BEARING)
}

fn unicode_hex(ch: char) -> String {
    let mut units = [0u16; 2];
    let encoded = ch.encode_utf16(&mut units);
    encoded.iter().map(|u| format!("{u:04X}")).collect()
}

/// This build has no Japanese face to draw replacement text with.
fn no_japanese_font() -> EditError {
    EditError::Invalid(
        "this text needs PdfCraft's Japanese fallback font, which this build doesn't include \
         (official releases do; to build it in, set CRAFT_FONTS_DIR to a craft-fonts checkout)"
            .into(),
    )
}

/// The first of `faces` (best match first) that has a glyph for every one of `chars`, else the first.
fn fallback_face<'a>(faces: &[&'a CraftFont], chars: &[char], has_glyph: impl Fn(&CraftFont, char) -> bool) -> Option<&'a CraftFont> {
    faces.iter().copied().find(|face| chars.iter().all(|ch| has_glyph(face, *ch))).or_else(|| faces.first().copied())
}

/// This build has no Hebrew face to draw replacement text with.
fn no_hebrew_font() -> EditError {
    EditError::Invalid(
        "this text needs PdfCraft's Hebrew fallback font (Noto Sans Hebrew), which this build doesn't include \
         (to build it in, set CRAFT_FONTS_DIR to a craft-fonts checkout that has it)"
            .into(),
    )
}

/// The faces a Type 3 replacement may draw `text` with, best first. Text with Hebrew takes the
/// Hebrew face first (its Latin and digits too, so a line keeps one look), then the Japanese
/// faces for anything it lacks. Text Inter can draw all of (Turkish ş, Polish ł, Greek, Cyrillic, …)
/// takes Inter, which every build has; other text takes the Japanese faces, as before.
fn type3_faces(text: &str, family: crate::added::Family, bold: bool) -> Result<(Vec<&'static CraftFont>, bool), EditError> {
    let japanese = pdfcraft_fonts::document_japanese_fonts_for_style(family == crate::added::Family::Times, bold);
    if !text.chars().any(crate::bidi::is_hebrew) {
        if text.chars().all(|c| is_win_ansi_char(c) || pdfcraft_fonts::inter_has(c)) {
            return Ok((std::iter::once(&pdfcraft_fonts::INTER).chain(japanese).collect(), false));
        }
        return Ok((japanese, false));
    }
    // Points are drawn on their letter with the font's positioning, which a one-glyph-per-
    // character Type 3 font can't do; refuse rather than print them beside the letter.
    if text.chars().any(crate::bidi::is_hebrew_mark) {
        return Err(EditError::Invalid("Hebrew points (niqqud) can't be written into a PDF yet; type the text without them".into()));
    }
    let hebrew = pdfcraft_fonts::document_hebrew_font(bold).ok_or_else(no_hebrew_font)?;
    Ok((std::iter::once(hebrew).chain(japanese).collect(), true))
}

fn type3_font(
    doc: &mut Document,
    fonts_res: &mut Dict,
    text: &str,
    family: crate::added::Family,
    bold: bool,
    max_glyphs: usize,
) -> Result<Type3Fallback, EditError> {
    let (faces, hebrew) = type3_faces(text, family, bold)?;
    // Each unique character once, in order of first appearance.
    let mut chars = Vec::new();
    let mut seen = std::collections::HashSet::new();
    for ch in text.chars() {
        if seen.insert(ch) {
            chars.push(ch);
        }
    }
    if chars.is_empty() {
        return Err(EditError::Invalid("replacement text is empty".into()));
    }
    // The faces differ in coverage (e.g. of Cyrillic), so use the best face that has every
    // character. When none has them all, the best face reports the character it lacks. Hebrew
    // text keeps the Hebrew face and takes each character it lacks from the next face that has it.
    let face = if hebrew { faces.first().copied() } else { fallback_face(&faces, &chars, |face, ch| japanese_glyph_from(face, ch).is_ok()) }
        .ok_or_else(no_japanese_font)?;
    let others: &[&CraftFont] = if hebrew { faces.get(1..).unwrap_or_default() } else { &[] };
    // Every outline is read before the document is touched, so a character that fails leaves it
    // as it was. Each font takes the next `per_font` characters; codes restart at 1 in each.
    let per_font = max_glyphs.clamp(1, MAX_TYPE3_GLYPHS);
    let mut glyphs = Vec::with_capacity(chars.len());
    for (i, ch) in chars.into_iter().enumerate() {
        // Hebrew text: the first face with the glyph, else the Hebrew face's error.
        let (path, width) = others.iter().fold(type3_path(face, ch), |found, f| found.or_else(|e| type3_path(f, ch).map_err(|_| e)))?;
        let code = u8::try_from((i % per_font).saturating_add(1))
            .map_err(|_| EditError::Invalid("replacement text has too many unique characters".into()))?;
        glyphs.push(Type3Glyph { code, ch, path, width });
    }
    let mut codes = HashMap::with_capacity(glyphs.len());
    let mut widths = Vec::with_capacity(glyphs.len());
    let mut names = Vec::new();
    let mut glyphs = glyphs.into_iter();
    loop {
        let group: Vec<Type3Glyph> = glyphs.by_ref().take(per_font).collect();
        if group.is_empty() {
            break;
        }
        for glyph in &group {
            codes.insert(glyph.ch, (names.len(), glyph.code));
            widths.push((glyph.ch, glyph.width));
        }
        names.push(add_type3_font(doc, fonts_res, face, hebrew, group));
    }
    Ok(Type3Fallback { family: face.family, names, widths, codes })
}

/// Adds one generated font whose codes 1 to `glyphs.len()` show `glyphs`, and returns its resource
/// name (`PCJp`, then `PCJp1`, ...; `PCHe`, ... for Hebrew text). Nothing here can fail: the
/// outlines were read already.
fn add_type3_font(doc: &mut Document, fonts_res: &mut Dict, face: &CraftFont, hebrew: bool, glyphs: Vec<Type3Glyph>) -> String {
    let last_char = glyphs.len();
    let mut charprocs = Dict::new();
    let mut widths = Vec::with_capacity(last_char);
    let mut differences = vec![Object::Int(1)];
    let mut cmap = String::from(
        "/CIDInit /ProcSet findresource begin\n12 dict begin\nbegincmap\n/CMapType 2 def\n1 begincodespacerange\n<01> <FF>\nendcodespacerange\n",
    );
    cmap.push_str(&format!("{last_char} beginbfchar\n"));
    for glyph in glyphs {
        let glyph_name = format!("g{:02X}", glyph.code);
        let mut pd = Dict::new();
        pd.set(b"Length".to_vec(), glyph.path.len() as i64);
        let proc_ref = doc.add(Object::Stream(Stream::from_raw(pd, glyph.path)));
        charprocs.set(glyph_name.as_bytes().to_vec(), Object::Ref(proc_ref));
        differences.push(Object::name(&glyph_name));
        widths.push(Object::Real((glyph.width * 1000.0).round()));
        cmap.push_str(&format!("<{:02X}> <{}>\n", glyph.code, unicode_hex(glyph.ch)));
    }
    cmap.push_str("endbfchar\nendcmap\nCMapName currentdict /CMap defineresource pop\nend\nend\n");
    let mut cmap_dict = Dict::new();
    cmap_dict.set(b"Length".to_vec(), cmap.len() as i64);
    let cmap_ref = doc.add(Object::Stream(Stream::from_raw(cmap_dict, cmap.into_bytes())));
    let mut encoding = Dict::new();
    encoding.set(b"Type".to_vec(), Object::name("Encoding"));
    encoding.set(b"Differences".to_vec(), Object::Array(differences));
    let (font_name, prefix) = if hebrew {
        (HEBREW_FONT_NAME, "PCHe")
    } else if is_inter(face) {
        (INTER_FONT_NAME, "PCIn")
    } else {
        (JAPANESE_FONT_NAME, "PCJp")
    };
    let mut font = Dict::new();
    font.set(b"Type".to_vec(), Object::name("Font"));
    font.set(b"Subtype".to_vec(), Object::name("Type3"));
    font.set(b"Name".to_vec(), Object::name(font_name));
    let mut descriptor = Dict::new();
    descriptor.set(b"Type".to_vec(), Object::name("FontDescriptor"));
    descriptor.set(b"FontName".to_vec(), Object::name(&format!("{}-{}", face.family, face.style).replace(' ', "")));
    descriptor.set(b"FontFamily".to_vec(), Object::String(PdfString::literal(face.family.as_bytes().to_vec())));
    descriptor.set(b"Flags".to_vec(), Object::Int(if face.family.contains("Mincho") { 6 } else { 4 }));
    descriptor.set(b"ItalicAngle".to_vec(), Object::Int(0));
    // PDF 1.7 tables 5.9 and 5.19: a Type 3 descriptor is indirect; Ascent/Descent may be omitted.
    font.set(b"FontDescriptor".to_vec(), Object::Ref(doc.add(Object::Dict(descriptor))));
    font.set(b"FontBBox".to_vec(), Object::Array(vec![Object::Int(0), Object::Int(-300), Object::Int(1000), Object::Int(1000)]));
    font.set(
        b"FontMatrix".to_vec(),
        Object::Array(vec![Object::Real(0.001), Object::Int(0), Object::Int(0), Object::Real(0.001), Object::Int(0), Object::Int(0)]),
    );
    font.set(b"FirstChar".to_vec(), Object::Int(1));
    font.set(b"LastChar".to_vec(), Object::Int(last_char as i64));
    font.set(b"Widths".to_vec(), Object::Array(widths));
    font.set(b"Encoding".to_vec(), Object::Dict(encoding));
    font.set(b"CharProcs".to_vec(), Object::Dict(charprocs));
    font.set(b"ToUnicode".to_vec(), Object::Ref(cmap_ref));
    let mut name = String::from(prefix);
    let mut suffix = 0usize;
    while fonts_res.contains(name.as_bytes()) {
        suffix = suffix.saturating_add(1);
        name = format!("{prefix}{suffix}");
    }
    fonts_res.set(name.as_bytes().to_vec(), Object::Dict(font));
    name
}

/// The runs that show `text` in the fallback: consecutive characters in one font form one run, so
/// a font is switched only where the text moves to another font. `None` if a character has no code.
fn type3_encode(fallback: &Type3Fallback, text: &str) -> Option<Vec<(String, Vec<u8>)>> {
    let mut runs: Vec<(usize, Vec<u8>)> = Vec::new();
    for ch in text.chars() {
        let (font, code) = *fallback.codes.get(&ch)?;
        match runs.last_mut() {
            Some((last, bytes)) if *last == font => bytes.push(code),
            _ => runs.push((font, vec![code])),
        }
    }
    runs.into_iter().map(|(font, bytes)| Some((fallback.names.get(font)?.clone(), bytes))).collect()
}

/// A subset may contain all the letters but no space glyph: producers use another subset
/// or a positioning operator for spaces. Do the same instead of substituting the whole font.
fn source_encodes(metrics: &Metrics, text: &str) -> bool {
    text.split(' ').all(|word| metrics.encode(word).is_some())
}

fn source_space(doc: &Document, fonts: &Dict, metrics: &Metrics) -> f64 {
    let width = |m: &Metrics| m.encode(" ").map(|b| m.codes(&b).iter().map(|(c, _)| m.width(*c)).sum::<f64>());
    if let Some(w) = width(metrics) {
        return w;
    }
    let family = metrics.base_font.rsplit('+').next().unwrap_or(&metrics.base_font);
    for (_, font) in fonts.iter() {
        if let Some(dict) = doc.resolve(font).as_dict() {
            let m = Metrics::from_dict(doc, dict);
            if m.base_font.rsplit('+').next() == Some(family)
                && let Some(w) = width(&m)
            {
                return w;
            }
        }
    }
    0.33
}

fn source_width(metrics: &Metrics, text: &str, size: f64, space: f64, char_spacing: f64, word_spacing: f64) -> f64 {
    if let Some(bytes) = metrics.encode(text) {
        // PDF word spacing applies only to single-byte code 32, even for a Unicode space.
        return metrics
            .codes(&bytes)
            .iter()
            .map(|(c, len)| metrics.width(*c) * size + char_spacing + if metrics.is_space(*c, *len) { word_spacing } else { 0.0 })
            .sum();
    }
    let words: f64 = text
        .split(' ')
        .filter_map(|w| metrics.encode(w))
        .map(|b| metrics.codes(&b).iter().map(|(c, _)| metrics.width(*c) * size + char_spacing).sum::<f64>())
        .sum();
    words + text.matches(' ').count() as f64 * (space * size + char_spacing + word_spacing)
}

fn source_show(metrics: &Metrics, text: &str, size: f64, space: f64, char_spacing: f64, word_spacing: f64) -> Result<Vec<Op>, EditError> {
    if let Some(bytes) = metrics.encode(text) {
        return Ok(vec![Op::new("Tj", vec![Object::String(PdfString::literal(bytes))])]);
    }
    let adjustment = -1000.0 * (space + (char_spacing + word_spacing) / size);
    if !adjustment.is_finite() {
        return Err(EditError::Invalid("the source font's space advance is invalid".into()));
    }
    let mut out = Vec::new();
    for (i, word) in text.split(' ').enumerate() {
        if i > 0 {
            out.push(Op::new("TJ", vec![Object::Array(vec![pdfcraft_content::num(adjustment)])]));
        }
        if !word.is_empty() {
            let bytes = metrics.encode(word).ok_or_else(|| EditError::Invalid("the source font cannot show the replacement text".into()))?;
            out.push(Op::new("Tj", vec![Object::String(PdfString::literal(bytes))]));
        }
    }
    Ok(out)
}

/// Replace the text of line `line` (an index into [`text_lines`]) on `page` with `text`.
pub fn replace_line(doc: &mut Document, page: usize, line: usize, text: &str) -> Result<LineEdit, EditError> {
    replace_line_with_limit(doc, page, line, text, MAX_TYPE3_GLYPHS)
}

/// [`replace_line`] with the most unique characters one generated font may hold (tests lower it).
pub(crate) fn replace_line_with_limit(doc: &mut Document, page: usize, line: usize, text: &str, max_glyphs: usize) -> Result<LineEdit, EditError> {
    crate::inline_forms::edit_inlined(doc, page, |doc| {
        // A visible line stays visible; an invisible one (an OCR layer) may be edited as it is.
        if text_lines(doc, page)?.get(line).is_some_and(|l| l.visible) {
            stays_visible(doc, page, |doc| replace_line_inlined(doc, page, line, text, max_glyphs))
        } else {
            replace_line_inlined(doc, page, line, text, max_glyphs)
        }
    })
}

/// [`replace_line_with_limit`] on a page whose text-bearing forms are already inlined.
fn replace_line_inlined(doc: &mut Document, page: usize, line: usize, text: &str, max_glyphs: usize) -> Result<LineEdit, EditError> {
    let text = text.replace(['\n', '\r'], " ");
    let lines = text_lines(doc, page)?;
    let target = lines.get(line).cloned().ok_or_else(|| EditError::Invalid(format!("page {} has no line {}", page + 1, line + 1)))?;
    let p = page_dict(doc, page)?;
    let mut res = p.dict.get(b"Resources").map(|r| doc.resolve(r)).and_then(|r| r.as_dict().cloned()).unwrap_or_default();
    let mut fonts_res = res.get(b"Font").map(|f| doc.resolve(f)).and_then(|f| f.as_dict().cloned()).unwrap_or_default();
    let streams = content_streams(doc, &p.dict);
    let pieces = Pieces::join(&streams.iter().map(|(_, d)| d.as_slice()).collect::<Vec<_>>());
    let ops = pieces.parse().ops;
    let first = target.ops.first().copied().filter(|i| *i < ops.len()).ok_or_else(|| EditError::Invalid("the page's content changed".into()))?;
    // The line's own font, when it can show every character.
    let font = fonts_res.get(target.font.as_bytes()).and_then(|f| doc.resolve(f).as_dict().cloned()).map(|d| Metrics::from_dict(doc, &d));
    // Typed in logical order; drawn left to right (right-to-left runs reversed).
    let shown = crate::bidi::to_visual(&text);
    let reused = font.as_ref().filter(|m| source_encodes(m, &shown));
    let mut substituted = None;
    let mut replacement: Vec<Op> = Vec::new();
    // ' and " also move to the next line; keep that.
    match ops[first].op.as_slice() {
        b"'" => replacement.push(Op::new("T*", vec![])),
        b"\"" => {
            replacement.push(Op::new("Tw", vec![ops[first].operands.first().cloned().unwrap_or(Object::Int(0))]));
            replacement.push(Op::new("Tc", vec![ops[first].operands.get(1).cloned().unwrap_or(Object::Int(0))]));
            replacement.push(Op::new("T*", vec![]));
        }
        _ => {}
    }
    match reused {
        Some(metrics) => replacement.extend(source_show(
            metrics,
            &shown,
            target.origin.state.font.as_ref().map_or(target.size, |(_, size)| *size),
            source_space(doc, &fonts_res, metrics),
            target.origin.state.char_spacing,
            target.origin.state.word_spacing,
        )?),
        None => {
            if needs_type3(&text) {
                let fallback = type3_font(doc, &mut fonts_res, &text, source_family(&target.base_font), target.bold, max_glyphs)?;
                let runs = type3_encode(&fallback, &shown)
                    .ok_or_else(|| EditError::Invalid(format!("\"{text}\" can't be shown by the {} fallback", fallback.family)))?;
                let size = font_size_before(&ops, first).unwrap_or(target.size);
                for (name, bytes) in runs {
                    replacement.push(Op::new("Tf", vec![Object::name(&name), pdfcraft_content::num(size)]));
                    replacement.push(Op::new("Tj", vec![Object::String(PdfString::literal(bytes))]));
                }
                replacement.push(Op::new("Tf", vec![Object::name(&target.font), pdfcraft_content::num(size)]));
                substituted = Some(format!("{} Type3", fallback.family));
            } else {
                let win = pdfcraft_fonts::win_ansi(&shown);
                // WinAnsi turns what it can't show into '?'; refuse rather than print the wrong thing.
                let back: String = win.iter().map(|b| char::from_u32(u32::from(*b)).unwrap_or('?')).collect();
                if shown.chars().zip(back.chars()).any(|(a, b)| b == '?' && a != '?') {
                    return Err(EditError::Invalid(format!("\"{text}\" has characters neither {} nor Helvetica can show", target.base_font)));
                }
                // The size in text space: the current Tf's size.
                let size = font_size_before(&ops, first).unwrap_or(target.size);
                let family = source_family(&target.base_font);
                let base = family.base_font(target.bold, target.italic);
                let substitute_name = format!("PCEd{}", base.replace('-', ""));
                replacement.push(Op::new("Tf", vec![Object::name(&substitute_name), pdfcraft_content::num(size)]));
                replacement.push(Op::new("Tj", vec![Object::String(PdfString::literal(win))]));
                replacement.push(Op::new("Tf", vec![Object::name(&target.font), pdfcraft_content::num(size)]));
                substituted = Some(base.to_string());
                let mut f = Dict::new();
                f.set(b"Type".to_vec(), Object::name("Font"));
                f.set(b"Subtype".to_vec(), Object::name("Type1"));
                f.set(b"BaseFont".to_vec(), Object::name(base));
                f.set(b"Encoding".to_vec(), Object::name("WinAnsiEncoding"));
                fonts_res.set(substitute_name.into_bytes(), Object::Dict(f));
            }
        }
    }
    // Rebuild: the line's first operator becomes the replacement, its others go. Copies of the
    // line drawn in the same area go too, so the replacement is all that shows.
    let drops = coincident_ops(&lines, std::slice::from_ref(&target.rect));
    let new_data = splice(&pieces, &ops, |i| if i == first { (std::mem::take(&mut replacement), false) } else { (Vec::new(), !drops.contains(&i)) });
    let contents = rewritten_contents(doc, &streams, new_data);
    let page_ref = p.obj;
    if substituted.is_some() {
        res.set(b"Font".to_vec(), Object::Dict(fonts_res));
    }
    doc.update_dict(page_ref, |d| {
        d.set(b"Contents".to_vec(), contents);
        if substituted.is_some() {
            d.set(b"Resources".to_vec(), Object::Dict(res));
        }
    })?;
    Ok(LineEdit { substituted })
}

/// The size of the last `Tf` before operator `at`.
fn font_size_before(ops: &[Op], at: usize) -> Option<f64> {
    ops[..at].iter().rev().find(|o| o.is("Tf")).and_then(|o| o.num(1))
}

/// The operators of every line drawn in the same area as one of `rects`. Documents sometimes
/// draw a line more than once (fake bold, an invisible text layer), and a surviving copy would
/// show the old text under the replaced line. Copies overlap a member's rect by more than half of
/// the smaller rect; neighbouring lines share no area.
fn coincident_ops(lines: &[TextLine], rects: &[[f64; 4]]) -> std::collections::HashSet<usize> {
    let area = |r: [f64; 4]| ((r[2] - r[0]) * (r[3] - r[1])).max(0.0);
    let mut drop = std::collections::HashSet::new();
    for l in lines {
        let covered = rects.iter().any(|m| {
            let ix = (m[2].min(l.rect[2]) - m[0].max(l.rect[0])).max(0.0);
            let iy = (m[3].min(l.rect[3]) - m[1].max(l.rect[1])).max(0.0);
            let small = area(*m).min(area(l.rect));
            small > 0.0 && ix * iy / small > 0.5
        });
        if covered {
            drop.extend(l.ops.iter().copied());
        }
    }
    drop
}

/// The paragraphs on a page (0-based).
pub fn text_blocks(doc: &Document, page: usize) -> Result<Vec<TextBlock>, EditError> {
    let lines = text_lines(doc, page)?;
    Ok(group_blocks(&lines))
}

/// Remove a paragraph's glyphs for a temporary editor background. Numeric TJ adjustments
/// preserve the text matrix, so unselected text positioned relative to it stays in place.
/// Call this on a clone: it is a preview operation, not a document edit.
pub fn clear_block(doc: &mut Document, page: usize, block: usize) -> Result<(), EditError> {
    crate::inline_forms::edit_inlined(doc, page, |doc| clear_block_inlined(doc, page, block))
}

fn clear_block_inlined(doc: &mut Document, page: usize, block: usize) -> Result<(), EditError> {
    let lines = text_lines(doc, page)?;
    let blocks = group_blocks(&lines);
    let target = blocks.get(block).ok_or_else(|| EditError::Invalid("the paragraph no longer exists".into()))?;
    let rects: Vec<_> = target.lines.iter().filter_map(|i| lines.get(*i)).map(|l| l.rect).collect();
    let mut drops = coincident_ops(&lines, &rects);
    drops.extend(target.lines.iter().filter_map(|i| lines.get(*i)).flat_map(|l| l.ops.iter().copied()));
    let p = page_dict(doc, page)?;
    let res = p.dict.get(b"Resources").and_then(|r| doc.resolve(r).as_dict().cloned()).unwrap_or_default();
    let fonts = res.get(b"Font").and_then(|r| doc.resolve(r).as_dict().cloned()).unwrap_or_default();
    let streams = content_streams(doc, &p.dict);
    let pieces = Pieces::join(&streams.iter().map(|(_, d)| d.as_slice()).collect::<Vec<_>>());
    let ops = pieces.parse().ops;
    let shown = interpret(doc, &ops, &fonts, &mut HashMap::new(), &mut Carry::new(), None, false);
    let mut patches = HashMap::new();
    for s in shown.into_iter().filter(|s| drops.contains(&s.op)) {
        if s.state.render_mode >= 4 {
            return Err(EditError::Invalid("text used as a clipping path cannot be previewed for editing".into()));
        }
        let denominator = s.state.size * s.state.scale;
        let adjustment = if denominator != 0.0 { -1000.0 * s.advance / denominator } else { 0.0 };
        if !adjustment.is_finite() || (denominator == 0.0 && s.advance != 0.0) {
            return Err(EditError::Invalid("the paragraph's text advance cannot be previewed".into()));
        }
        let mut replacement = Vec::new();
        if let Some(op) = ops.get(s.op) {
            if op.is("\"") {
                replacement.push(Op::new("Tw", vec![op.operands.first().cloned().unwrap_or(Object::Int(0))]));
                replacement.push(Op::new("Tc", vec![op.operands.get(1).cloned().unwrap_or(Object::Int(0))]));
            }
            if op.is("'") || op.is("\"") {
                replacement.push(Op::new("T*", vec![]));
            }
        }
        replacement.push(Op::new("TJ", vec![Object::Array(vec![pdfcraft_content::num(adjustment)])]));
        patches.insert(s.op, replacement);
    }
    let data = splice(&pieces, &ops, |i| patches.remove(&i).map_or((Vec::new(), true), |v| (v, false)));
    let contents = rewritten_contents(doc, &streams, data);
    doc.update_dict(p.obj, |d| d.set(b"Contents".to_vec(), contents))?;
    Ok(())
}

/// The paragraphs on a page as a reader sees them, including text drawn by form XObjects
/// (Export to Word, HTML and RTF). Read-only: indexes into this list are not
/// [`text_blocks`] indexes, so they can't be passed to [`replace_block`] or [`rewrite_block`].
pub fn reading_blocks(doc: &Document, page: usize) -> Result<Vec<TextBlock>, EditError> {
    let lines = reading_lines(doc, page)?;
    Ok(regions::group_reading_blocks(&lines))
}

// An ideographic space carries intentional Japanese spacing, including paragraph indentation.
// Keep other whitespace normalization unchanged.
fn paragraph_separator(c: char) -> bool {
    c.is_whitespace() && c != '\u{3000}'
}

/// Greedy word wrapping to `width` with `advance` giving a string's width.
fn wrap(text: &str, width: f64, advance: impl Fn(&str) -> f64) -> Vec<String> {
    let mut out = Vec::new();
    let mut line = String::new();
    for word in text.split(paragraph_separator).filter(|word| !word.is_empty()) {
        let candidate = if line.is_empty() { word.to_string() } else { format!("{line} {word}") };
        if !line.is_empty() && advance(&candidate) > width {
            out.push(std::mem::take(&mut line));
            line = word.to_string();
        } else {
            line = candidate;
        }
    }
    if !line.is_empty() || out.is_empty() {
        out.push(line);
    }
    out
}

/// Replace paragraph `block` (an index into [`text_blocks`]) with `text`, rewrapped to the
/// paragraph's width with its line spacing. The paragraph keeps its first line's position, font
/// (or Helvetica when the font can't show the text), size and colour.
pub fn replace_block(doc: &mut Document, page: usize, block: usize, text: &str) -> Result<LineEdit, EditError> {
    rewrite_block(doc, page, block, Some(text), &BlockStyle::default())
}

/// Formatting for a rewritten paragraph (`None` keeps the paragraph's own).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct BlockStyle {
    /// Disjoint UTF-8 ranges in the replacement, each preserving unspecified source style.
    pub ranges: Vec<RangeStyle>,
    /// Explicit page font resource (from register_text_font or the page font inventory).
    pub resource: Option<String>,
    /// A standard font family, bold, italic.
    pub family: Option<(crate::added::Family, bool, bool)>,
    /// Override weight without choosing a different family (`None` keeps the source weight).
    pub bold: Option<bool>,
    /// Font size in points (user space).
    pub size: Option<f64>,
    /// Fill colour (RGB 0–1).
    pub color: Option<[f64; 3]>,
    pub align: Option<crate::added::Align>,
    /// Draw a line under each line of text.
    pub underline: Option<bool>,
    /// Line spacing as a multiple of the font size (1.2 is ordinary).
    pub line_spacing: Option<f64>,
    /// Character spacing (points) and horizontal scaling (percent).
    pub char_spacing: Option<f64>,
    pub scale: Option<f64>,
    /// Move the paragraph by `[dx, dy]` in user space (dragging its box).
    pub offset: Option<[f64; 2]>,
    /// Rewrap to this width in user space (dragging the box's edge).
    pub width: Option<f64>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct RangeStyle {
    pub range: std::ops::Range<usize>,
    pub family: Option<(crate::added::Family, bool, bool)>,
    pub resource: Option<String>,
    pub size: Option<f64>,
    pub color: Option<[f64; 3]>,
    pub underline: Option<bool>,
    pub char_spacing: Option<f64>,
    pub scale: Option<f64>,
}

/// Rewrite paragraph `block` with new text (or its own) and formatting, rewrapped to its width.
pub fn rewrite_block(doc: &mut Document, page: usize, block: usize, text: Option<&str>, style: &BlockStyle) -> Result<LineEdit, EditError> {
    rewrite_block_with_limit(doc, page, block, text, style, MAX_TYPE3_GLYPHS)
}

/// [`rewrite_block`] with the most unique characters one generated font may hold (tests lower it).
pub(crate) fn rewrite_block_with_limit(
    doc: &mut Document,
    page: usize,
    block: usize,
    text: Option<&str>,
    style: &BlockStyle,
    max_glyphs: usize,
) -> Result<LineEdit, EditError> {
    crate::inline_forms::edit_inlined(doc, page, |doc| {
        stays_visible(doc, page, |doc| rewrite_block_inlined(doc, page, block, text, style, max_glyphs))
    })
}

/// Run a rewrite of visible text on a copy, and keep it only if it adds no invisible text: new
/// text must never end up drawn at zero opacity, invisibly or clipped away (it is placed in the
/// graphics state of the paragraph's first run). Otherwise nothing changes and the edit is refused.
fn stays_visible<T>(doc: &mut Document, page: usize, f: impl FnOnce(&mut Document) -> Result<T, EditError>) -> Result<T, EditError> {
    let key = |l: &TextLine| (l.text.clone(), l.rect.map(f64::to_bits));
    let mut hidden = std::collections::BTreeMap::new();
    for l in text_lines(doc, page)?.iter().filter(|l| !l.visible) {
        *hidden.entry(key(l)).or_insert(0_usize) += 1;
    }
    let mut work = doc.clone();
    let out = f(&mut work)?;
    for l in text_lines(&work, page)?.iter().filter(|l| !l.visible) {
        match hidden.get_mut(&key(l)) {
            Some(n) if *n > 0 => *n -= 1,
            _ => {
                return Err(EditError::Invalid("the edited text would not be visible where this paragraph is drawn, so nothing was changed".into()));
            }
        }
    }
    *doc = work;
    Ok(out)
}

/// [`rewrite_block_with_limit`] on a page whose text-bearing forms are already inlined.
fn rewrite_block_inlined(
    doc: &mut Document,
    page: usize,
    block: usize,
    text: Option<&str>,
    style: &BlockStyle,
    max_glyphs: usize,
) -> Result<LineEdit, EditError> {
    let lines = text_lines(doc, page)?;
    let blocks = group_blocks(&lines);
    let b = blocks.get(block).cloned().ok_or_else(|| EditError::Invalid(format!("page {} has no paragraph {}", page + 1, block + 1)))?;
    let members: Vec<&TextLine> = b.lines.iter().map(|i| &lines[*i]).collect();
    let first = members.first().copied().ok_or_else(|| EditError::Invalid("the paragraph has no source runs".into()))?;
    let p = page_dict(doc, page)?;
    let mut res = p.dict.get(b"Resources").map(|r| doc.resolve(r)).and_then(|r| r.as_dict().cloned()).unwrap_or_default();
    let mut fonts_res = res.get(b"Font").map(|f| doc.resolve(f)).and_then(|f| f.as_dict().cloned()).unwrap_or_default();
    let registered_face = fonts_res.iter().any(|(name, _)| name.starts_with(b"PCFull"));
    // Hebrew is written back in drawing order (and with the Hebrew fallback face) by the
    // single-style path below; the styled region path does not reorder right-to-left runs.
    let hebrew = text.unwrap_or(&b.text).chars().any(crate::bidi::is_hebrew);
    if hebrew && (!style.ranges.is_empty() || style.resource.is_some()) {
        return Err(EditError::Invalid(
            "formatting part of right-to-left (Hebrew) text, or choosing a document font for it, isn't supported yet".into(),
        ));
    }
    if !hebrew
        && (!style.ranges.is_empty()
            || style.resource.is_some()
            || (text.is_some_and(|draft| {
                draft.contains('\n')
                    || pdfcraft_fonts::source_shaping::required(draft)
                    || !pdfcraft_fonts::embedding::matching_faces(&b.base_font).is_empty()
            }) || registered_face
                || b.rows.iter().any(|row| row.len() > 1)
                || members.iter().any(|line| line.font != first.font || line.size != first.size || line.color != first.color))
                && style.family.is_none()
                && style.bold.is_none())
    {
        return rewrite_styled_block(doc, page, &b, &lines, text, style);
    }
    let streams = content_streams(doc, &p.dict);
    let pieces = Pieces::join(&streams.iter().map(|(_, d)| d.as_slice()).collect::<Vec<_>>());
    let ops = pieces.parse().ops;
    let text = text.unwrap_or(&b.text).split(paragraph_separator).filter(|word| !word.is_empty()).collect::<Vec<_>>().join(" ");
    let o = &first.origin;
    let (font_name, old_size) = o.state.font.clone().ok_or_else(|| EditError::Invalid("the paragraph has no font".into()))?;
    let k = o.k.max(1e-6);
    // The size in text space: points ÷ the text-to-user scale.
    let size = style.size.map_or(old_size, |pt| (pt / k).max(0.1));
    // Spacing and scaling (text state), with the chosen values.
    let mut ts_state = o.state.clone();
    if let Some(c) = style.char_spacing {
        ts_state.char_spacing = c / k;
    }
    if let Some(sc) = style.scale {
        ts_state.scale = (sc / 100.0).clamp(0.1, 10.0);
    }
    let o_state = ts_state.clone();
    let metrics = fonts_res.get(font_name.as_bytes()).and_then(|f| doc.resolve(f).as_dict().cloned()).map(|d| Metrics::from_dict(doc, &d));
    let reuse = style.family.is_none() && style.bold.is_none() && metrics.as_ref().is_some_and(|m| source_encodes(m, &text));
    let space = metrics.as_ref().map_or(0.33, |m| source_space(doc, &fonts_res, m));
    // The chosen style also determines the real Japanese fallback outlines and advances.
    let (family, bold, italic) = style.family.unwrap_or((source_family(&b.base_font), b.bold, b.italic));
    let bold = style.bold.unwrap_or(bold);
    let std_width = move |s: &str, size: f64| -> f64 {
        match family {
            crate::added::Family::Courier => s.chars().count() as f64 * 0.6 * size,
            crate::added::Family::Times => pdfcraft_fonts::helvetica_width(s, size) * 0.92,
            crate::added::Family::Helvetica => pdfcraft_fonts::helvetica_width(s, size) * if bold { 1.05 } else { 1.0 },
        }
    };
    let [dx, dy] = style.offset.unwrap_or([0.0, 0.0]);
    if !(dx.is_finite() && dy.is_finite()) {
        return Err(EditError::Invalid("the paragraph can't be moved that far".into()));
    }
    // Right-to-left text (more Hebrew or Arabic letters than Latin) is right-aligned unless an
    // alignment is asked for, and a single right-to-left line keeps its right edge as it grows.
    let rtl = crate::bidi::rtl_base(&text);
    // One visual row, however many runs draw it (Chrome positions every word, even every glyph).
    let single_row = b.rows.len() <= 1;
    let anchored_right = rtl && style.width.is_none() && single_row;
    // A paragraph keeps its width (or takes the one asked for); a single line grows to the right
    // (to the left when right-to-left), up to the page's margin.
    let width = match style.width {
        // Capped at the largest page PDF allows (14 400 pt).
        Some(w) if w.is_finite() => w.clamp(size * k, 14_400.0),
        Some(_) => return Err(EditError::Invalid("the paragraph's width must be a number".into())),
        None if anchored_right => (b.rect[2] - b.rect[0]).max(size * k).max(b.rect[2] + dx - (p.crop(doc)[0] + 36.0)),
        None if single_row => (b.rect[2] - b.rect[0]).max(size * k).max(p.crop(doc)[2] - 36.0 - (b.rect[0] + dx)),
        None => (b.rect[2] - b.rect[0]).max(size * k),
    };
    if !width.is_finite() {
        return Err(EditError::Invalid("the paragraph's width must be a number".into()));
    }
    let align = style.align.or(rtl.then_some(crate::added::Align::Right));
    // Where a right-aligned line ends, from the paragraph's left edge (user space).
    let right_edge = if anchored_right { (b.rect[2] - b.rect[0]).max(0.0) } else { width };
    // The move and the width are checked here, before the fallback fonts are written: a refused
    // paragraph leaves the document as it was. The move is in page space, so it is taken back through
    // the CTM's linear part into the space the paragraph is drawn in.
    let movement = if dx != 0.0 || dy != 0.0 {
        let c = o.ctm;
        let back = Matrix([c[0], c[1], c[2], c[3], 0.0, 0.0])
            .invert()
            .ok_or_else(|| EditError::Invalid("the paragraph is drawn in a space it can't be moved in".into()))?;
        Some(back.apply(dx, dy))
    } else {
        None
    };
    let type3 = if !reuse && needs_type3(&text) { Some(type3_font(doc, &mut fonts_res, &text, family, bold, max_glyphs)?) } else { None };
    let advance = |s: &str| -> f64 {
        let t = match (&metrics, reuse) {
            (Some(m), true) => source_width(m, s, size, space, o_state.char_spacing, o_state.word_spacing),
            _ if let Some(fallback) = &type3 => {
                // Each character's count in `s`, from one pass over `s`.
                let mut counts: HashMap<char, usize> = HashMap::new();
                for c in s.chars() {
                    let count = counts.entry(c).or_insert(0);
                    *count = count.saturating_add(1);
                }
                fallback.widths.iter().map(|(ch, width)| counts.get(ch).copied().unwrap_or(0) as f64 * width * size).sum::<f64>()
                    + s.chars().count() as f64 * o_state.char_spacing
            }
            _ => std_width(s, size) + s.chars().count() as f64 * o_state.char_spacing,
        };
        t * o_state.scale * k
    };
    let wrapped = wrap(&text, width + 0.5, advance);
    let mut substituted = None;
    let new_font = !reuse;
    let (show_font, encode): (String, Encoder) = if let Some(m) = metrics.clone().filter(|_| reuse) {
        let name = font_name.clone();
        (font_name.clone(), Box::new(move |s: &str| m.encode(s).map(|bytes| vec![(name.clone(), bytes)])))
    } else if let Some(fallback) = type3.clone() {
        substituted = Some(format!("{} Type3", fallback.family));
        let first = fallback.names.first().cloned().ok_or_else(no_japanese_font)?;
        (first, Box::new(move |s: &str| type3_encode(&fallback, s)))
    } else {
        let win = pdfcraft_fonts::win_ansi(&text);
        let back: String = win.iter().map(|c| char::from_u32(u32::from(*c)).unwrap_or('?')).collect();
        let base = family.base_font(bold, italic);
        if text.chars().zip(back.chars()).any(|(a, c)| c == '?' && a != '?') {
            return Err(EditError::Invalid(format!("\"{text}\" has characters neither {} nor {base} can show", b.base_font)));
        }
        let mut f = Dict::new();
        f.set(b"Type".to_vec(), Object::name("Font"));
        f.set(b"Subtype".to_vec(), Object::name("Type1"));
        f.set(b"BaseFont".to_vec(), Object::name(base));
        f.set(b"Encoding".to_vec(), Object::name("WinAnsiEncoding"));
        let name = if style.family.is_none() { String::from_utf8_lossy(SUBSTITUTE).into_owned() } else { format!("PCEd{}", base.replace('-', "")) };
        fonts_res.set(name.clone().into_bytes(), Object::Dict(f));
        if style.family.is_none() {
            substituted = Some(base.to_string());
        }
        let shown = name.clone();
        (name, Box::new(move |s: &str| Some(vec![(shown.clone(), pdfcraft_fonts::win_ansi(s))])))
    };
    // Line spacing in text space: the paragraph's own (scaled with the size), or 1.2 × the size.
    // The source pitch is between the first runs of its first two visual rows (each row may be
    // many runs, and each may be placed by its own Tm), measured across the baseline.
    let row_start = |r: usize| b.rows.get(r).and_then(|row| row.first()).and_then(|i| lines.get(*i));
    let pitch = match (row_start(0), row_start(1)) {
        (Some(a), Some(c)) => {
            let up = Matrix(a.origin.tm).then(&Matrix(a.origin.ctm)).0;
            let (ux, uy) = (up[2], up[3]);
            let norm = ux.hypot(uy);
            (norm > 1e-9).then(|| ((a.origin.x - c.origin.x) * ux + (a.origin.baseline - c.origin.baseline) * uy) / norm).filter(|d| *d > 1e-6)
        }
        _ => None,
    };
    let lead = match style.line_spacing {
        Some(m) => size * m.clamp(0.5, 5.0),
        None if let Some(d) = pitch => d / k * size / old_size,
        None if o_state.leading > 0.0 => o_state.leading,
        None => size * 1.2,
    };
    let n = pdfcraft_content::num;
    // In its own graphics state, so a new colour (or anything else) stops at the paragraph.
    let mut block_ops = vec![Op::new("q", vec![])];
    // Moved: a translation inside that state.
    if let Some((ax, ay)) = movement {
        block_ops.push(Op::new("cm", vec![n(1.0), n(0.0), n(0.0), n(1.0), n(ax), n(ay)]));
    }
    block_ops.push(Op::new("BT", vec![]));
    let mut state = o_state.clone();
    state.word_spacing = 0.0;
    // The font the next run is shown in: the one the BT sets, until a run of another fallback subset.
    let mut shown_font = show_font.clone();
    state.font = Some((show_font, size));
    if let Some([r, g, bl]) = style.color.filter(|c| !same_colour(*c, b.color)) {
        state.fill = vec![Op::new("rg", vec![n(r), n(g), n(bl)])];
    }
    block_ops.extend(state.ops());
    block_ops.push(Op::new("Tm", o.tm.iter().map(|v| n(*v)).collect()));
    // Alignment: each line's offset from the left edge, in text space.
    let offset = |line: &str| -> f64 {
        let free = (width - advance(line)).max(0.0) / k;
        match align {
            Some(crate::added::Align::Center) => free / 2.0,
            // A right-to-left line anchored at its right edge may start left of the old box.
            Some(crate::added::Align::Right) if anchored_right => (right_edge - advance(line)) / k,
            Some(crate::added::Align::Right) => free,
            // A justified right-to-left paragraph ends its last line at the right edge.
            Some(crate::added::Align::Justify) if rtl => free,
            _ => 0.0,
        }
    };
    // Justify: word spacing (single-byte code 32 only) so each line but the last fills the width.
    let single_byte = !reuse || metrics.as_ref().is_some_and(|m| !m.composite);
    let justify = align == Some(crate::added::Align::Justify) && single_byte;
    let mut x = 0.0;
    let mut tw_set = 0.0;
    let mut underlines: Vec<(f64, f64, f64)> = Vec::new(); // (x0, x1, y) in text space
    for (i, line) in wrapped.iter().enumerate() {
        let spaces = line.matches(' ').count();
        let tw =
            if justify && i + 1 < wrapped.len() && spaces > 0 { (width - advance(line)).max(0.0) / k / o_state.scale / spaces as f64 } else { 0.0 };
        // A stretched line fills the width from the left edge.
        let dx = if tw > 0.0 { 0.0 } else { offset(line) };
        if i > 0 || dx != 0.0 {
            block_ops.push(Op::new("Td", vec![n(dx - x), n(if i > 0 { -lead } else { 0.0 })]));
        }
        x = dx;
        if tw != tw_set {
            block_ops.push(Op::new("Tw", vec![n(tw)]));
            tw_set = tw;
        }
        // Each wrapped line in typing order; drawn left to right (right-to-left runs reversed).
        let visual = crate::bidi::to_visual(line);
        if let Some(metrics) = metrics.as_ref().filter(|_| reuse) {
            block_ops.extend(source_show(metrics, &visual, size, space, o_state.char_spacing, tw)?);
        } else {
            for (font, bytes) in encode(&visual).ok_or_else(|| EditError::Invalid(format!("\"{line}\" can't be shown")))? {
                if font != shown_font {
                    block_ops.push(Op::new("Tf", vec![Object::name(&font), n(size)]));
                    shown_font = font;
                }
                block_ops.push(Op::new("Tj", vec![Object::String(PdfString::literal(bytes))]));
            }
        }
        let w = (advance(line) + tw * spaces as f64 * o_state.scale * k) / k;
        underlines.push((dx, dx + w, -(i as f64) * lead - size * 0.12));
    }
    block_ops.push(Op::new("ET", vec![]));
    if style.underline == Some(true) {
        // In text space (the paragraph's text matrix), in its fill colour.
        let colour: Vec<Op> = state
            .fill
            .iter()
            .map(|f| match f.op.as_slice() {
                b"g" => Op::new("G", f.operands.clone()),
                b"rg" => Op::new("RG", f.operands.clone()),
                b"k" => Op::new("K", f.operands.clone()),
                b"cs" => Op::new("CS", f.operands.clone()),
                b"sc" => Op::new("SC", f.operands.clone()),
                _ => Op::new("SCN", f.operands.clone()),
            })
            .collect();
        block_ops.push(Op::new("q", vec![]));
        block_ops.push(Op::new("cm", o.tm.iter().map(|v| n(*v)).collect()));
        block_ops.extend(colour);
        block_ops.push(Op::new("w", vec![n(size * 0.06)]));
        for (x0, x1, y) in underlines {
            block_ops.push(Op::new("m", vec![n(x0), n(y)]));
            block_ops.push(Op::new("l", vec![n(x1), n(y)]));
        }
        block_ops.push(Op::new("S", vec![]));
        block_ops.push(Op::new("Q", vec![]));
    }
    block_ops.push(Op::new("Q", vec![]));
    // The paragraph's operators go, and so does anything drawn in the same area (a fake-bold
    // second copy, an invisible text layer): the new text is all that may show.
    let member_rects: Vec<[f64; 4]> = members.iter().map(|l| l.rect).collect();
    let mut drop = coincident_ops(&lines, &member_rects);
    drop.extend(members.iter().flat_map(|l| l.ops.iter().copied()));
    // Text shown earlier in the same text object stays first: the paragraph then goes where it
    // was, between two halves of the text object, so it keeps its place in reading order.
    let start = first.ops.first().copied().unwrap_or(o.bt_op);
    let split = ops
        .get(o.bt_op..start)
        .unwrap_or_default()
        .iter()
        .enumerate()
        .any(|(i, op)| matches!(op.op.as_slice(), b"Tj" | b"TJ" | b"'" | b"\"") && !drop.contains(&(o.bt_op + i)));
    // What the text after the paragraph expects: the state at its BT (or where it was split).
    let after = if split { &o.state } else { &o.bt_state };
    block_ops.extend(after.ops().into_iter().filter(|op| !op.is("Tf") || after.font.is_some()));
    if split {
        block_ops.insert(0, Op::new("ET", vec![]));
        block_ops.push(Op::new("BT", vec![]));
        block_ops.push(Op::new("Tm", o.tlm.iter().map(|v| n(*v)).collect()));
    }
    let at = if split { start } else { o.bt_op };
    let new_data = splice(&pieces, &ops, |i| {
        let insert = if i == at { std::mem::take(&mut block_ops) } else { Vec::new() };
        (insert, !drop.contains(&i))
    });
    let contents = rewritten_contents(doc, &streams, new_data);
    if new_font {
        res.set(b"Font".to_vec(), Object::Dict(fonts_res));
    }
    let page_ref = p.obj;
    doc.update_dict(page_ref, |d| {
        d.set(b"Contents".to_vec(), contents);
        if new_font {
            d.set(b"Resources".to_vec(), Object::Dict(res));
        }
    })?;
    Ok(LineEdit { substituted })
}

/// Existing paragraph indexes, excluding streams owned by an added-content item.
pub(crate) fn selectable_blocks(
    doc: &Document,
    page: usize,
    excluded: &std::collections::BTreeSet<pdfcraft_cos::ObjRef>,
) -> Result<Vec<(usize, TextBlock)>, EditError> {
    let p = page_dict(doc, page)?;
    let streams = content_streams(doc, &p.dict);
    let lines = text_lines(doc, page)?;
    Ok(group_blocks(&lines)
        .into_iter()
        .enumerate()
        .filter(|(_, b)| {
            !b.lines
                .iter()
                .filter_map(|i| lines.get(*i))
                .any(|l| streams.get(l.stream).and_then(|(o, _)| o.as_ref()).is_some_and(|r| excluded.contains(&r)))
        })
        .collect())
}

/// Detect vertical CMaps before moving horizontal text. A broken/cyclic embedded CMap is
/// refused rather than guessing which direction TJ advances its text matrix.
fn vertical_encoding(doc: &Document, font: &Dict) -> Result<bool, EditError> {
    if font.name(b"Subtype") != Some(b"Type0") {
        return Ok(false);
    }
    let mut next = font.get(b"Encoding").cloned();
    let mut seen = std::collections::BTreeSet::new();
    for _ in 0..8 {
        let Some(o) = next.take() else { return Ok(false) };
        if let Some(r) = o.as_ref()
            && !seen.insert(r)
        {
            return Err(EditError::Invalid("the text encoding is cyclic".into()));
        }
        match &*doc.resolve(&o) {
            Object::Name(n) => return Ok(n.ends_with(b"-V")),
            Object::Stream(stream) => {
                if let Some(mode) = stream.dict.get(b"WMode") {
                    match mode.as_int() {
                        Some(1) => return Ok(true),
                        Some(0) => {}
                        _ => return Err(EditError::Invalid("the text writing mode is invalid".into())),
                    }
                }
                let bytes = stream
                    .decoded_within(8 * 1024 * 1024)
                    .map_err(|_| EditError::Invalid("the text encoding cannot be decoded within its size limit".into()))?;
                let ops = parse(&bytes).ops;
                if let Some(mode) = ops.iter().rev().find(|op| op.is("def") && op.name(0) == Some(b"WMode")) {
                    return match mode.num(1) {
                        Some(0.0) => Ok(false),
                        Some(1.0) => Ok(true),
                        _ => Err(EditError::Invalid("the text writing mode is invalid".into())),
                    };
                }
                next = stream
                    .dict
                    .get(b"UseCMap")
                    .cloned()
                    .or_else(|| ops.iter().rev().find(|op| op.is("usecmap")).and_then(|op| op.operands.first().cloned()));
            }
            _ => return Err(EditError::Invalid("the text encoding is invalid".into())),
        }
    }
    Err(EditError::Invalid("the text encoding is nested too deeply".into()))
}

/// Translate paragraphs together without re-encoding glyphs, changing fonts or reflowing.
/// Numeric TJ offsets and text rise temporarily move each selected show operation, then
/// restore both advances and rise. The PDF reader advances the original glyph bytes itself:
/// no approximate font metrics are used to position following, unselected text (§9.4.4).
pub(crate) fn translate_blocks(doc: &mut Document, page: usize, indexes: &[usize], offset: [f64; 2]) -> Result<(), EditError> {
    if indexes.is_empty() {
        return Ok(());
    }
    crate::inline_forms::edit_inlined(doc, page, |doc| translate_blocks_inlined(doc, page, indexes, offset))
}

fn translate_blocks_inlined(doc: &mut Document, page: usize, indexes: &[usize], offset: [f64; 2]) -> Result<(), EditError> {
    let p = page_dict(doc, page)?;
    let streams = content_streams(doc, &p.dict);
    let lines = text_lines(doc, page)?;
    let blocks = group_blocks(&lines);
    let mut selected = std::collections::BTreeSet::new();
    for index in indexes {
        let b = blocks.get(*index).ok_or_else(|| EditError::Invalid("a selected paragraph no longer exists".into()))?;
        for line in &b.lines {
            let l = lines.get(*line).ok_or_else(|| EditError::Invalid("a selected text line no longer exists".into()))?;
            selected.extend(l.ops.iter().copied());
        }
    }
    let res = p.dict.get(b"Resources").map(|r| doc.resolve(r)).and_then(|r| r.as_dict().cloned()).unwrap_or_default();
    let fonts = res.get(b"Font").map(|r| doc.resolve(r)).and_then(|r| r.as_dict().cloned()).unwrap_or_default();
    let pieces = Pieces::join(&streams.iter().map(|(_, d)| d.as_slice()).collect::<Vec<_>>());
    let ops = pieces.parse().ops;
    let shown = interpret(doc, &ops, &fonts, &mut HashMap::new(), &mut Carry::new(), None, false);
    let mut patches = HashMap::new();
    // Keep full precision: a tiny rounded text-space change can be magnified by a CTM,
    // and restoring rise/spacing must not alter following unselected text.
    let n = Object::Real;
    for s in shown.into_iter().filter(|s| selected.contains(&s.op)) {
        if !(0..=3).contains(&s.state.render_mode) {
            return Err(EditError::Invalid("text used as a clipping path cannot be moved in a group".into()));
        }
        let font = fonts
            .get(&s.font)
            .map(|f| doc.resolve(f))
            .and_then(|f| f.as_dict().cloned())
            .ok_or_else(|| EditError::Invalid("a selected paragraph has no font".into()))?;
        if vertical_encoding(doc, &font)? {
            return Err(EditError::Invalid("vertical text cannot be moved in a group".into()));
        }
        let inverse = s.tm.then(&s.state.ctm).invert().ok_or_else(|| EditError::Invalid("a selected paragraph has no area".into()))?;
        let [a, b, c, d, _, _] = inverse.0;
        let (dx, dy) = (a * offset[0] + c * offset[1], b * offset[0] + d * offset[1]);
        let denominator = s.state.size * s.state.scale;
        let shift = if dx == 0.0 {
            0.0
        } else if denominator.abs() > 1e-12 {
            -1000.0 * dx / denominator
        } else {
            return Err(EditError::Invalid("a selected paragraph has no horizontal advance".into()));
        };
        let rise = s.state.rise + dy;
        if ![shift, rise, dx, dy].iter().all(|v| v.is_finite() && v.abs() <= 1e12) {
            return Err(EditError::Invalid("the paragraph move is too large".into()));
        }
        let original = ops.get(s.op).ok_or_else(|| EditError::Invalid("a selected text operation is missing".into()))?;
        let mut replacement = Vec::new();
        let show = if original.is("'") || original.is("\"") {
            if original.is("\"") {
                let spacing = original
                    .operands
                    .get(..2)
                    .filter(|a| a.iter().all(|o| o.as_f64().is_some()))
                    .ok_or_else(|| EditError::Invalid("a text operation has invalid spacing".into()))?;
                for (operator, operand) in ["Tw", "Tc"].into_iter().zip(spacing) {
                    replacement.push(Op::new(operator, vec![operand.clone()]));
                }
            }
            replacement.push(Op::new("T*", vec![]));
            Op::new("Tj", vec![original.operands.last().cloned().ok_or_else(|| EditError::Invalid("a text operation has no string".into()))?])
        } else {
            original.clone()
        };
        if dy != 0.0 {
            replacement.push(Op::new("Ts", vec![n(rise)]));
        }
        if shift != 0.0 {
            replacement.push(Op::new("TJ", vec![Object::Array(vec![n(shift)])]));
        }
        replacement.push(show);
        if shift != 0.0 {
            replacement.push(Op::new("TJ", vec![Object::Array(vec![n(-shift)])]));
        }
        if dy != 0.0 {
            replacement.push(Op::new("Ts", vec![n(s.state.rise)]));
        }
        patches.insert(s.op, replacement);
    }
    if patches.len() != selected.len() {
        return Err(EditError::Invalid("a selected text operation cannot be moved".into()));
    }
    let data = splice(&pieces, &ops, |i| patches.remove(&i).map_or((Vec::new(), true), |v| (v, false)));
    let contents = rewritten_contents(doc, &streams, data);
    doc.update_dict(p.obj, |d| d.set(b"Contents".to_vec(), contents))?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn paragraph_wrapping_keeps_ideographic_spaces() {
        let width = |s: &str| s.chars().count() as f64;
        assert_eq!(wrap("A　　B C", 4.0, width), ["A　　B", "C"]);
        assert_eq!(wrap("A\u{a0}B C", 3.0, width), ["A B", "C"]);
        assert_eq!(wrap("A\u{2028}B C", 3.0, width), ["A B", "C"]);
        assert_eq!(wrap(" \tA  B\r\nC ", 3.0, width), ["A B", "C"]);
    }

    #[test]
    fn fallback_face_moves_on_when_the_best_face_lacks_a_character() {
        static JPAN: &[&str] = &["Jpan"];
        let mincho = CraftFont { family: "Mincho", style: "Regular", scripts: JPAN, bytes: b"" };
        let gothic = CraftFont { family: "Gothic", style: "Regular", scripts: JPAN, bytes: b"" };
        let faces = [&mincho, &gothic];
        // Only the Gothic face has the Cyrillic letter.
        let has = |face: &CraftFont, ch: char| ch.is_ascii() || face.family == "Gothic";
        assert_eq!(fallback_face(&faces, &['a', 'ф'], has).map(|f| f.family), Some("Gothic"));
        // Characters the best face has keep it.
        assert_eq!(fallback_face(&faces, &['a', 'b'], has).map(|f| f.family), Some("Mincho"));
        // No face has them all: the best face stays, and names the missing glyph.
        assert_eq!(fallback_face(&faces, &['a', 'ф'], |_, ch| ch.is_ascii()).map(|f| f.family), Some("Mincho"));
        assert!(fallback_face(&[], &['a'], has).is_none());
    }

    #[test]
    fn hebrew_and_japanese_fallback_shards_are_recognized_as_ours() {
        static HEBR: &[&str] = &["Hebr"];
        let face = CraftFont { family: "FakeHebrew", style: "Regular", scripts: HEBR, bytes: b"" };
        let glyph = |code, ch| Type3Glyph { code, ch, path: b"0 0 0 0 0 0 d1\nf\n".to_vec(), width: 0.5 };
        let mut doc = Document::new_empty();
        let mut fonts = Dict::new();
        // A replacement split across two Hebrew fonts, then one Japanese font.
        let he = add_type3_font(&mut doc, &mut fonts, &face, true, vec![glyph(1, 'א')]);
        let he1 = add_type3_font(&mut doc, &mut fonts, &face, true, vec![glyph(1, 'ב')]);
        let jp = add_type3_font(&mut doc, &mut fonts, &face, false, vec![glyph(1, 'あ')]);
        assert_eq!([he.as_str(), he1.as_str(), jp.as_str()], ["PCHe", "PCHe1", "PCJp"]);
        for name in [&he, &he1, &jp] {
            let font = fonts.get(name.as_bytes()).and_then(Object::as_dict).cloned().unwrap_or_default();
            assert!(is_type3_fallback(&font), "{name} isn't recognized as a generated fallback font");
        }
        // Someone else's Type 3 font is not ours.
        let mut other = Dict::new();
        other.set(b"Subtype".to_vec(), Object::name("Type3"));
        other.set(b"Name".to_vec(), Object::name("PCHebrewX"));
        assert!(!is_type3_fallback(&other));
    }
}
