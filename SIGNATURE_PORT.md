# Digital-signature port (vksp2k7/pdfcraft `feat/upstream-signature` -> pdfcraft-android)

Android already had the digital-ID flow (Fill & Sign > "Sign with a digital ID": create or import
a .p12, sign, validate, trust a signer). This port brings in the upstream branch's newer signing
code underneath it.

## Changed
- `crates/sign/**`: replaced with the upstream branch's version (hardened PKCS #12 open/write,
  validated-signature appearances, stricter ExtendedKeyUsage, new tests).
- `crates/cos/src/document.rs`: native credential reuse for encrypted earlier revisions, xref
  oddity tracking, and exact-entry checks used by signature validation.
- `crates/engine/src/lib.rs`: `display_bytes` now draws validated signature appearances; signatures
  are validated before the display copy is built; trust changes redraw the page and increment the
  document generation.
- Android UI refreshes the document after trust changes so Compose/page-image caches request the
  new validated appearance.
- `crates/automation/src/{signing,tools}.rs`: `sign_document` takes `appearance` and `labels`;
  password minimum is `pkcs12::MIN_PASSWORD_CHARS` (8).
- Android UI (`DigitalSign.kt`): Create-ID password minimum 6 -> 8 (`MIN_ID_PASSWORD`).
- Tests: passwords `secret1` -> `secret12`; the three upstream xref/signature regression tests run
  normally now that the cos hardening is native.

## Verification
The Rust cos encryption, xref, and signing regression tests pass. The full workspace checks and
Android release build are run separately as part of the delivery checklist.

## Note
New IDs now use 600,000 PBKDF2 rounds (upstream's `WRITE_ROUNDS`), which is noticeably slower on a
phone when creating or opening an ID than the old 2048.
