//! pdfcraft-fonts — font metrics and encodings for generated appearances (L2).
//!
//! See the README: the metrics are approximations by character class (no vendor metrics files
//! are bundled). The full font subsystem lands in M2.2/M7.

#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable)]

mod craft;
pub mod embedding;
mod encodings;
pub mod pdf;
mod script;
mod shaping;
pub mod source_shaping;
pub use craft::{
    CRAFT_FONTS, CraftFont, SHIPPORI_MINCHO, document_arabic_font, document_hebrew_font, document_japanese_font, document_japanese_font_for_style,
    document_japanese_fonts_for_style, document_thai_font, ui_arabic_fonts, ui_chinese_fonts, ui_cjk_fonts, ui_hebrew_fonts, ui_japanese_fonts,
    ui_telugu_fonts, ui_thai_fonts,
};
pub use script::{
    GlyphError, GlyphOutline, INTER, INTER_REGULAR, MAX_SIGNATURE_CHARS, ScriptOutline, inter_has, inter_path, inter_width, japanese_glyph,
    japanese_glyph_from, script_missing, script_outline,
};
pub use shaping::{ShapedCluster, ShapedScript, arabic_glyph, arabic_has, shape_arabic, shape_inter, shape_thai, shaped_glyph, thai_has};

/// Approximate advance of `s` in Helvetica (or Arial) at `size` points.
pub fn helvetica_width(s: &str, size: f64) -> f64 {
    let units: f64 = s
        .chars()
        .map(|c| match c {
            ' ' | 'i' | 'j' | 'l' | '\'' | '!' | '|' | '.' | ',' | ':' | ';' | 'I' => 260.0,
            'f' | 't' | 'r' | '(' | ')' | '[' | ']' | '/' | '-' | '"' => 333.0,
            'm' => 833.0,
            'w' => 722.0,
            'M' => 833.0,
            'W' => 944.0,
            'J' | 'c' | 'k' | 's' | 'v' | 'x' | 'y' | 'z' => 500.0,
            '0'..='9' | 'a'..='z' | '$' | '#' | '?' | '_' => 556.0,
            'A'..='Z' => 680.0,
            '@' => 1015.0,
            _ if c.is_whitespace() => 260.0,
            _ => 584.0,
        })
        .sum();
    units * size / 1000.0
}

/// Approximate advance of `s` at `size` points in a CJK font: full-width characters take one
/// em, half-width forms half of one, and everything else is measured as Helvetica.
pub fn cjk_width(s: &str, size: f64) -> f64 {
    let mut buf = [0u8; 4];
    s.chars()
        .map(|c| match c {
            '\u{ff61}'..='\u{ffdc}' | '\u{ffe8}'..='\u{ffee}' => size * 0.5,
            _ if is_wide(c) => size,
            _ => helvetica_width(c.encode_utf8(&mut buf), size),
        })
        .sum()
}

/// East Asian wide and full-width characters (Unicode UAX #11 W and F, by block).
fn is_wide(c: char) -> bool {
    matches!(c,
        '\u{1100}'..='\u{115f}' // Hangul Jamo initials
        | '\u{2e80}'..='\u{303e}' // CJK radicals, Kangxi, ideographic description, CJK symbols
        | '\u{3041}'..='\u{33ff}' // kana, Bopomofo, Hangul compatibility, Kanbun, CJK compatibility
        | '\u{3400}'..='\u{4dbf}' // CJK extension A
        | '\u{4e00}'..='\u{9fff}' // CJK unified ideographs
        | '\u{a000}'..='\u{a4cf}' // Yi
        | '\u{ac00}'..='\u{d7a3}' // Hangul syllables
        | '\u{f900}'..='\u{faff}' // CJK compatibility ideographs
        | '\u{fe30}'..='\u{fe4f}' // CJK compatibility forms
        | '\u{ff00}'..='\u{ff60}' // full-width forms
        | '\u{ffe0}'..='\u{ffe6}'
        | '\u{20000}'..='\u{3fffd}' // supplementary ideographic planes
    )
}

/// Greedy line breaking within `width` points (paragraphs split on newlines; words longer
/// than a line are broken by character).
pub fn wrap(text: &str, size: f64, width: f64) -> Vec<String> {
    wrap_with(text, size, width, helvetica_width)
}

/// [`wrap`], measuring with `measure` (for example [`cjk_width`]).
pub fn wrap_with(text: &str, size: f64, width: f64, measure: impl Fn(&str, f64) -> f64) -> Vec<String> {
    let mut lines = Vec::new();
    for para in text.split(['\n', '\r']) {
        let mut line = String::new();
        for word in para.split(' ') {
            let candidate = if line.is_empty() { word.to_string() } else { format!("{line} {word}") };
            if measure(&candidate, size) <= width || line.is_empty() && measure(word, size) <= width {
                line = candidate;
                continue;
            }
            if !line.is_empty() {
                lines.push(std::mem::take(&mut line));
            }
            for ch in word.chars() {
                if !line.is_empty() && measure(&format!("{line}{ch}"), size) > width {
                    lines.push(std::mem::take(&mut line));
                }
                line.push(ch);
            }
        }
        lines.push(line);
    }
    lines
}

/// The WinAnsiEncoding (ISO 32000-2 Annex D) code for `c`, if the encoding has one.
///
/// Codes 0x80–0x9F come from the same table the text extractor decodes with, so encoding and
/// decoding agree (Š š Ž ž Œ œ Ÿ ƒ † ‡ ˆ ˜ ‰ ‹ › as well as quotes, dashes, € and ™).
pub fn win_ansi_byte(c: char) -> Option<u8> {
    match c {
        '\u{20}'..='\u{7e}' | '\u{a0}'..='\u{ff}' => u8::try_from(u32::from(c)).ok(),
        _ => {
            let high = encodings::WIN_ANSI.get(0x80..0xa0)?;
            let i = high.iter().position(|&u| u != 0 && u == u32::from(c))?;
            u8::try_from(0x80 + i).ok()
        }
    }
}

/// Encode text in WinAnsiEncoding (ISO 32000-2 Annex D); tabs become spaces and unmappable
/// characters become `?`.
pub fn win_ansi(s: &str) -> Vec<u8> {
    s.chars().map(|c| if c == '\t' { b' ' } else { win_ansi_byte(c).unwrap_or(b'?') }).collect()
}

/// The character encoding of a predefined Unicode CMap (`UniJIS-UTF16-H`, `UniGB-UCS2-H`, …;
/// ISO 32000-2 §9.7.5.2), which maps Unicode text straight to codes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum UnicodeCMap {
    Ucs2,
    Utf16,
    Utf8,
    Utf32,
}

impl UnicodeCMap {
    /// The encoding of the predefined CMap `name`, if it is a horizontal Unicode one. Vertical
    /// (`-V`) CMaps are refused: field appearances are laid out horizontally, so their glyphs
    /// would be drawn stacked down the field instead of across it.
    pub fn from_name(name: &[u8]) -> Option<Self> {
        let name = std::str::from_utf8(name).ok()?;
        if !name.starts_with("Uni") || !name.ends_with("-H") {
            return None;
        }
        name.split('-').find_map(|part| match part {
            "UCS2" => Some(Self::Ucs2),
            "UTF16" => Some(Self::Utf16),
            "UTF8" => Some(Self::Utf8),
            "UTF32" => Some(Self::Utf32),
            _ => None,
        })
    }

    /// The first character of `s` this encoding can't hold, which [`Self::encode`] would write as
    /// `?`: a character beyond U+FFFF in UCS-2 (`𠀋`). UTF-16, UTF-8 and UTF-32 hold every one.
    pub fn first_unencodable(self, s: &str) -> Option<char> {
        match self {
            Self::Ucs2 => s.chars().find(|c| u16::try_from(u32::from(*c)).is_err()),
            Self::Utf16 | Self::Utf8 | Self::Utf32 => None,
        }
    }

    /// Encode `s`; tabs become spaces, and characters UCS-2 can't hold become `?` (check them
    /// first with [`Self::first_unencodable`]: PdfCraft refuses such text rather than write `?`).
    pub fn encode(self, s: &str) -> Vec<u8> {
        let mut out = Vec::with_capacity(s.len().saturating_mul(2));
        let mut units = [0u16; 2];
        let mut buf = [0u8; 4];
        for c in s.chars() {
            let c = if c == '\t' { ' ' } else { c };
            match self {
                Self::Ucs2 => {
                    let unit = u16::try_from(u32::from(c)).unwrap_or(u16::from(b'?'));
                    out.extend_from_slice(&unit.to_be_bytes());
                }
                Self::Utf16 => {
                    for unit in c.encode_utf16(&mut units) {
                        out.extend_from_slice(&unit.to_be_bytes());
                    }
                }
                Self::Utf8 => out.extend_from_slice(c.encode_utf8(&mut buf).as_bytes()),
                Self::Utf32 => out.extend_from_slice(&u32::from(c).to_be_bytes()),
            }
        }
        out
    }
}

/// Whether [`win_ansi`] keeps `c` rather than writing `?` (a tab becomes a space).
pub fn win_ansi_encodable(c: char) -> bool {
    c == '\t' || win_ansi_byte(c).is_some()
}

/// The first character of `s` that [`win_ansi`] can't encode and would write as `?`.
pub fn first_non_win_ansi(s: &str) -> Option<char> {
    s.chars().find(|c| !win_ansi_encodable(*c))
}

/// The first character of `s` the standard fonts (Helvetica and the rest of the standard 14, in
/// WinAnsiEncoding) can't draw, so that a generated appearance would show it as `?`. Line breaks
/// don't count: appearances lay lines out themselves. PdfCraft refuses such text with
/// [`undrawable_message`] instead of writing `?` (AGENTS.md §4: never lose work silently).
pub fn first_undrawable(s: &str) -> Option<char> {
    s.chars().find(|c| !matches!(c, '\n' | '\r') && !win_ansi_encodable(*c))
}

/// Whether PdfCraft draws `s` in Inter instead of the standard fonts: they can't draw one of its
/// characters (Turkish ş, Polish ł, Greek, Cyrillic, …) and Inter, which every build has, can draw
/// every one of those. Line breaks don't count.
pub fn needs_inter(s: &str) -> bool {
    let mut any = false;
    for c in s.chars().filter(|c| !matches!(c, '\n' | '\r') && !win_ansi_encodable(*c)) {
        if !inter_has(c) {
            return false;
        }
        any = true;
    }
    any
}

/// [`first_undrawable`] where text the standard fonts can't draw is drawn in Inter instead
/// ([`needs_inter`]): the first character that would still come out as `?`, or `None`.
pub fn first_undrawable_inter(s: &str) -> Option<char> {
    if needs_inter(s) { None } else { first_undrawable(s) }
}

/// The error for text the standard fonts can't draw ([`first_undrawable`]): which character, in
/// what (`"the comment"`, `"the reason"`), and what can be written instead. One wording for every
/// path that writes text in the standard fonts, so agents and users can act on it the same way.
pub fn undrawable_message(what: &str, c: char) -> String {
    format!(
        "the standard fonts can't draw \"{}\" (U+{:04X}) in {what}; only Western European characters can be written there for now",
        c.escape_debug(),
        u32::from(c)
    )
}

/// The private key on an annotation (a comment or a form field's widget) whose appearance PdfCraft
/// drew with `?` in place of characters its fonts can't draw: a text string holding the first
/// such character. The text itself (a field's `/V`, a comment's `/Contents`) is stored as typed;
/// the mark lets flattening refuse to make that `?` permanent, lets an edit report it, and tells
/// such an appearance apart from one another program drew with a font that has those characters.
pub const UNDRAWN_MARK: &[u8] = b"PCUndrawn";

/// The character [`UNDRAWN_MARK`] records on `d`, if PdfCraft drew its appearance with `?`.
pub fn undrawn_mark(d: &pdfcraft_cos::Dict) -> Option<char> {
    d.get(UNDRAWN_MARK).and_then(pdfcraft_cos::Object::as_string).and_then(|s| s.to_text().chars().next())
}

/// Record (`Some`) or clear (`None`) [`UNDRAWN_MARK`] on `d`, for the appearance just drawn.
pub fn set_undrawn_mark(d: &mut pdfcraft_cos::Dict, c: Option<char>) {
    match c {
        Some(c) => d.set(UNDRAWN_MARK.to_vec(), pdfcraft_cos::PdfString::text(&c.to_string())),
        None => {
            d.remove(UNDRAWN_MARK);
        }
    }
}

/// Bytes as a PDF literal string, `(` … `)`, with delimiters escaped.
pub fn literal(bytes: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(bytes.len() + 2);
    out.push(b'(');
    for &b in bytes {
        match b {
            b'(' | b')' | b'\\' => out.extend_from_slice(&[b'\\', b]),
            b'\r' => out.extend_from_slice(b"\\r"),
            b'\n' => out.extend_from_slice(b"\\n"),
            _ => out.push(b),
        }
    }
    out.push(b')');
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn win_ansi_encodability_matches_the_encoder() {
        for c in ['a', 'é', 'ÿ', '€', '—', '™', '\t', '?'] {
            assert!(win_ansi_encodable(c), "{c:?}");
        }
        for c in ['中', 'あ', 'Ω', 'č', 'ă', '☃', '😀'] {
            assert!(!win_ansi_encodable(c), "{c:?}");
        }
        assert_eq!(first_non_win_ansi("Café — 5€"), None);
        assert_eq!(first_non_win_ansi("Dvořák"), Some('ř'));
        // Line breaks are laid out, not drawn; other controls would be written as `?`.
        assert_eq!(first_undrawable("Café\r\n— 5€\tok"), None);
        assert_eq!(first_undrawable("签名 Ada"), Some('签'));
        assert_eq!(first_undrawable("a\u{7}b"), Some('\u{7}'));
        assert_eq!(first_undrawable("𠀋"), Some('𠀋'));
        let m = undrawable_message("the comment", '签');
        assert_eq!(m, "the standard fonts can't draw \"签\" (U+7B7E) in the comment; only Western European characters can be written there for now");
        assert!(undrawable_message("the reason", '\u{7}').contains("\"\\u{7}\" (U+0007)"));
        assert!(undrawable_message("x", '𠀋').contains("(U+2000B)"));
    }

    /// Text beyond Western European (Turkish, Polish, Czech, Romanian, Greek, Cyrillic) is drawn in
    /// the bundled Inter instead of as `?`; text Inter can't draw either keeps the old behaviour.
    #[test]
    fn inter_draws_what_the_standard_fonts_cannot() {
        for s in ["Mustafa çokyaşar", "ÇçĞğİıÖöŞşÜü", "Zażółć gęślą jaźń", "Příliš žluťoučký kůň", "Țară și", "Ελληνικά", "Привет, Їжак"]
        {
            assert!(needs_inter(s), "{s}");
            assert_eq!(first_undrawable_inter(s), None, "{s}");
            for c in s.chars().filter(|c| !win_ansi_encodable(*c)) {
                assert!(inter_has(c), "{s}: {c}");
            }
        }
        // Western European text stays in the standard fonts.
        assert!(!needs_inter("Café — 5€\r\n™"));
        assert_eq!(first_undrawable_inter("Café"), None);
        // Inter can't draw 中, so the text stays in Helvetica, where ş is the first `?`.
        assert!(!needs_inter("ş 中"));
        assert_eq!(first_undrawable_inter("ş 中"), Some('ş'));
        assert_eq!(first_undrawable_inter("签名"), Some('签'));
        // A line as paths: curves, closed, at the given place and size.
        let path = inter_path("ş", 10.0, 5.0, 7.0);
        assert!(path.contains(" m\n") && path.contains(" c\n") && path.ends_with("h\n"), "{path}");
        let xs: Vec<f64> = path.lines().filter_map(|l| l.split(' ').next()?.parse().ok()).collect();
        assert!(xs.iter().all(|x| (5.0..=11.0).contains(x)), "{xs:?}");
        assert!(inter_path(" \t", 10.0, 0.0, 0.0).is_empty(), "spaces have no outline");
        // Inter's advances, close to Helvetica's estimate for the same letters.
        let (inter, helvetica) = (inter_width("Mustafa çokyaşar", 10.0), helvetica_width("Mustafa cokyasar", 10.0));
        assert!(inter > helvetica * 0.8 && inter < helvetica * 1.3, "{inter} vs {helvetica}");
        assert_eq!(inter_width("a\tb", 10.0), inter_width("a b", 10.0));
    }

    #[test]
    fn the_undrawn_mark_records_one_character_and_clears() {
        let mut d = pdfcraft_cos::Dict::new();
        assert_eq!(undrawn_mark(&d), None);
        set_undrawn_mark(&mut d, Some('𠀋'));
        assert_eq!(undrawn_mark(&d), Some('𠀋'), "beyond U+FFFF, as a UTF-16 text string");
        set_undrawn_mark(&mut d, Some('签'));
        assert_eq!(undrawn_mark(&d), Some('签'));
        set_undrawn_mark(&mut d, None);
        assert!(!d.contains(UNDRAWN_MARK));
        // Another value under the key is not a character: no mark.
        d.set(UNDRAWN_MARK.to_vec(), pdfcraft_cos::Object::Bool(true));
        assert_eq!(undrawn_mark(&d), None);
    }

    #[test]
    fn widths_wrap_and_encode() {
        assert!(helvetica_width("MMMM", 10.0) > helvetica_width("iiii", 10.0) * 2.0);
        assert_eq!(helvetica_width("", 12.0), 0.0);
        let lines = wrap("the quick brown fox jumps over the lazy dog", 12.0, 80.0);
        assert!(lines.len() > 2 && lines.iter().all(|l| helvetica_width(l, 12.0) <= 80.0));
        assert_eq!(wrap("a\nb", 12.0, 100.0), ["a", "b"]);
        let long = wrap("Supercalifragilisticexpialidocious", 12.0, 40.0);
        assert!(long.len() > 3 && long.concat() == "Supercalifragilisticexpialidocious");
        assert_eq!(win_ansi("Café — 5€ ☃"), b"Caf\xe9 \x97 5\x80 ?");
        assert_eq!(literal(b"a(b)\\c"), b"(a\\(b\\)\\\\c)");
    }

    #[test]
    fn win_ansi_covers_the_0x80_to_0x9f_glyphs() {
        // Every character in the WinAnsiEncoding table encodes to its code and back (#347).
        for (code, &u) in encodings::WIN_ANSI.iter().enumerate() {
            let Some(c) = char::from_u32(u).filter(|_| u != 0) else { continue };
            assert_eq!(win_ansi_byte(c), Some(code as u8), "{c:?} (U+{u:04X})");
            assert_eq!(win_ansi(&c.to_string()), [code as u8]);
        }
        assert_eq!(
            win_ansi("Šta? žaba, œuvre — „ok“ ‰ ƒ † ‡ ˆ ˜ ‹›Ÿ"),
            b"\x8ata? \x9eaba, \x9cuvre \x97 \x84ok\x93 \x89 \x83 \x86 \x87 \x88 \x98 \x8b\x9b\x9f"
        );
        // Not in WinAnsi: č ć still need the fallback font; undefined codes are never produced.
        assert_eq!(win_ansi("čć\u{81}\u{8d}\t"), b"???? ");
        assert_eq!(win_ansi_byte('\0'), None);
    }

    #[test]
    fn unicode_cmaps_encode_cjk_text() {
        assert_eq!(UnicodeCMap::from_name(b"UniJIS-UTF16-H"), Some(UnicodeCMap::Utf16));
        assert_eq!(UnicodeCMap::from_name(b"UniGB-UCS2-V"), None);
        assert_eq!(UnicodeCMap::from_name(b"UniJIS-UCS2-HW-H"), Some(UnicodeCMap::Ucs2));
        assert_eq!(UnicodeCMap::from_name(b"UniKS-UTF8-H"), Some(UnicodeCMap::Utf8));
        assert_eq!(UnicodeCMap::from_name(b"UniJIS2004-UTF32-H"), Some(UnicodeCMap::Utf32));
        assert_eq!(UnicodeCMap::from_name(b"Identity-H"), None);
        assert_eq!(UnicodeCMap::from_name(b"90ms-RKSJ-H"), None);
        assert_eq!(UnicodeCMap::from_name(b"\xff\xfe"), None);
        assert_eq!(UnicodeCMap::Utf16.encode("令A"), [0x4e, 0xe4, 0x00, 0x41]);
        assert_eq!(UnicodeCMap::Utf16.encode("𠀋"), [0xd8, 0x40, 0xdc, 0x0b]);
        assert_eq!(UnicodeCMap::Ucs2.encode("𠀋\t"), [0x00, b'?', 0x00, b' ']);
        // That `?` is never written silently: callers check first.
        assert_eq!(UnicodeCMap::Ucs2.first_unencodable("令和𠀋"), Some('𠀋'));
        assert_eq!(UnicodeCMap::Ucs2.first_unencodable("令和"), None);
        for cmap in [UnicodeCMap::Utf16, UnicodeCMap::Utf8, UnicodeCMap::Utf32] {
            assert_eq!(cmap.first_unencodable("令和𠀋"), None, "{cmap:?}");
        }
        assert_eq!(UnicodeCMap::Utf8.encode("令"), "令".as_bytes());
        assert_eq!(UnicodeCMap::Utf32.encode("令"), [0, 0, 0x4e, 0xe4]);
        assert_eq!(cjk_width("令和", 10.0), 20.0);
        assert_eq!(cjk_width("ｱ", 10.0), 5.0);
        assert_eq!(cjk_width("A", 10.0), helvetica_width("A", 10.0));
        let lines = wrap_with("日本語のテキストは文字単位で折り返されます", 10.0, 45.0, cjk_width);
        assert!(lines.len() > 3 && lines.iter().all(|l| cjk_width(l, 10.0) <= 45.0), "{lines:?}");
    }
}
