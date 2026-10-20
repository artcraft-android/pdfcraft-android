# Digital-signature port (vksp2k7/pdfcraft `feat/upstream-signature` -> pdfcraft-android)

Android already had the digital-ID flow (Fill & Sign > "Sign with a digital ID": create or import
a .p12, sign, validate, trust a signer). This port brings in the upstream branch's newer signing
code underneath it.

## Changed
- `crates/sign/**`: replaced with the upstream branch's version (hardened PKCS #12 open/write,
  validated-signature appearances, stricter ExtendedKeyUsage, new tests).
- `crates/sign/src/compat.rs` (new): stand-ins for three `pdfcraft-cos` methods upstream's signing
  code calls and this workspace's cos lacks. See the file header for what they give up.
- `crates/engine/src/lib.rs`: `display_bytes` now draws validated signature appearances; signatures
  are validated before the display copy is built; trust changes redraw the page.
- `crates/automation/src/{signing,tools}.rs`: `sign_document` takes `appearance` and `labels`;
  password minimum is `pkcs12::MIN_PASSWORD_CHARS` (8).
- Android UI (`DigitalSign.kt`): Create-ID password minimum 6 -> 8 (`MIN_ID_PASSWORD`).
- Tests: passwords `secret1` -> `secret12`; three upstream sign tests are `#[ignore]`d (they need
  the cos xref hardening).

## Not verified
No Rust toolchain was available where this was prepared, so nothing here was compiled or run.
Run `cargo test -p pdfcraft-sign -p pdfcraft-engine -p pdfcraft-automation` and the Android
`cargo test` / Gradle build first.

## Note
New IDs now use 600,000 PBKDF2 rounds (upstream's `WRITE_ROUNDS`), which is noticeably slower on a
phone when creating or opening an ID than the old 2048.
