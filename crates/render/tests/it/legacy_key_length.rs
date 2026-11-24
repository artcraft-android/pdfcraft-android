//! Contributor-original synthetic files: an `/Encrypt` dictionary of revision 2–4 whose key
//! would be longer than the 16-byte MD5 hash it is taken from (`/V 5` with `/R 3`, or
//! `/Length 256`). The parser panicked slicing the hash (vendored hayro-syntax patch 35, found
//! by fuzzing); the file is now refused as invalid encryption, and nothing is caught as a crash.
use pdfcraft_render::{OpenError, PageRenderer, RenderConfig, inspect};
use std::sync::Arc;

/// A one-page file encrypted with the `/Encrypt` entries `encrypt`, with a cross-reference
/// table, and the same without one (rebuilt by scanning, as the fuzzer's input was).
fn files(encrypt: &str) -> [Arc<Vec<u8>>; 2] {
    let objects = [
        "<< /Type /Catalog /Pages 2 0 R >>".to_string(),
        "<< /Type /Pages /Kids [3 0 R] /Count 1 >>".to_string(),
        "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 200 200] >>".to_string(),
        format!("<< /Filter /Standard {encrypt} /O <00> /U <00> /P -4 >>"),
    ];
    let mut out = b"%PDF-1.7\n".to_vec();
    let mut offsets = Vec::new();
    for (i, body) in objects.iter().enumerate() {
        offsets.push(out.len());
        out.extend_from_slice(format!("{} 0 obj\n{body}\nendobj\n", i + 1).as_bytes());
    }
    let trailer = "<< /Size 5 /Root 1 0 R /Encrypt 4 0 R /ID [<0123456789ABCDEF> <0123456789ABCDEF>] >>";
    let scanned = [out.clone(), format!("trailer\n{trailer}\n%%EOF\n").into_bytes()].concat();
    let xref_at = out.len();
    out.extend_from_slice(b"xref\n0 5\n0000000000 65535 f \n");
    for o in offsets {
        out.extend_from_slice(format!("{o:010} 00000 n \n").as_bytes());
    }
    out.extend_from_slice(format!("trailer\n{trailer}\nstartxref\n{xref_at}\n%%EOF\n").as_bytes());
    [Arc::new(out), Arc::new(scanned)]
}

const OVERSIZED: [&str; 3] =
    ["/V 5 /R 3", "/V 2 /R 3 /Length 256", "/V 4 /R 4 /Length 256 /CF << /StdCF << /CFM /AESV2 >> >> /StmF /StdCF /StrF /StdCF"];

#[test]
fn a_legacy_key_longer_than_its_hash_is_refused_not_caught_as_a_crash() {
    for encrypt in OVERSIZED {
        for bytes in files(encrypt) {
            match inspect(bytes.clone(), None) {
                Err(OpenError::Invalid(why) | OpenError::Unsupported(why)) => assert!(!why.contains("crashed"), "{encrypt}: {why}"),
                other => panic!("{encrypt}: {other:?}"),
            }
            assert_eq!(PageRenderer::new(bytes, RenderConfig::default()).page_count(), 0, "{encrypt}");
        }
    }
}

#[test]
fn a_legacy_key_of_128_bits_still_asks_for_the_password() {
    // The control: a valid key length, read through the cross-reference table.
    let [with_table, _] = files("/V 2 /R 3 /Length 128");
    let r = inspect(with_table, None);
    assert!(matches!(r, Err(OpenError::NeedsPassword)), "{r:?}");
}
