//! Stand-ins for three `pdfcraft_cos::Document` methods the upstream signing code calls, for
//! workspaces whose `pdfcraft-cos` does not have them yet. A method on `Document` itself wins
//! over these, so they fall away on their own once `pdfcraft-cos` gains the real ones.
//!
//! What they give up, until then:
//! - `open_with_same_credentials` reopens a signed revision without the document's own key, so
//!   an encrypted document's *earlier* signature can read as unverifiable once it is
//!   counter-signed (upstream #1808). Unencrypted documents are unaffected.
//! - `xref_oddities` reports none, and `xref_entry_is_exact` accepts every entry, so the check
//!   that rejects a later cross-reference section the renderer may resolve differently from
//!   the reader does not run.

use std::sync::Arc;

use pdfcraft_cos::{CosError, Document};

pub(crate) trait CosCompat {
    /// Open `data` as another revision of this document.
    fn open_with_same_credentials(&self, data: Arc<Vec<u8>>) -> Result<Document, CosError>;
    /// Cross-reference entries a reader sets aside, as (offset, object number); none known here.
    fn xref_oddities(&self) -> &[(u64, u32)];
    /// Whether the entry of `num` locates that object itself; always assumed here.
    fn xref_entry_is_exact(&self, num: u32) -> bool;
}

impl CosCompat for Document {
    fn open_with_same_credentials(&self, data: Arc<Vec<u8>>) -> Result<Document, CosError> {
        Document::open(data)
    }

    fn xref_oddities(&self) -> &[(u64, u32)] {
        &[]
    }

    fn xref_entry_is_exact(&self, _num: u32) -> bool {
        true
    }
}
