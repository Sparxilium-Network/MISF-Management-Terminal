# Third-party notices

MISF embeds the `luksbox-core`, `luksbox-format`, `luksbox-pq`, `luksbox-fido2`, and
`luksbox-vfs` Rust libraries from [PentHertz/LUKSbox](https://github.com/PentHertz/LUKSbox).

- Upstream version: 0.6.1
- Pinned source commit: `69448f708f1328d672deb595e359579d630cb0b0`
- Copyright: 2026 Penthertz
- License: Apache License 2.0
- Unmodified license and notice: `third_party/LUKSbox/LICENSE` and
  `third_party/LUKSbox/NOTICE` (distributed as `licenses/LUKSbox/` in the portable app).

The FIDO2 crate is a locally modified compatibility copy in
`third_party/LUKSbox/luksbox-fido2/`, updated by MISF on 2026-10-30 to support
current P-256, cryptographic primitives, OS randomness, and Windows bindings.
See `third_party/LUKSbox/PATCHES.md` (distributed as `licenses/LUKSbox/PATCHES.md`)
for provenance and changes. The container core, format, PQ, and VFS crates
remain unmodified at the pinned commit. Upstream copyrights and license notices
are preserved.

MISF uses its own GUI and storage adapter. This distribution does not include LUKSbox's mount drivers,
WinFsp, TPM adapters, or the upstream GUI. Windows uses the system WebAuthn API;
Linux dynamically links to the system's libfido2 (not bundled). The integration is not an
endorsement by Penthertz. LUKSbox and Penthertz names retain their upstream
trademark ownership.

## Other Rust dependencies

Every other crate compiled into `misf` for x86_64 Windows and Linux is listed,
with its version, repository, and full license text, in
`licenses/THIRD_PARTY_LICENSES.html`. That file is generated from `Cargo.lock`
by cargo-about (`./licenses.ps1`); packaging refuses to run when it is out of
date. Where a crate offers a choice of licenses (for example `MIT OR Apache-2.0`),
MISF uses it under a license compatible with the GNU AGPL v3, checked by
`cargo deny check licenses` against `deny.toml`.

SQLite 3.54.1 is compiled in through `libsqlite3-sys` (bundled). SQLite is in
the public domain. `ring` includes code derived from BoringSSL and OpenSSL;
its ISC and Apache-2.0 texts are included in the generated file.

## Fonts embedded by egui

eframe's `default_fonts` feature embeds Ubuntu Light (Ubuntu Font Licence 1.0),
Hack Regular (MIT, with Bitstream Vera and DejaVu notices), and a five-glyph
subset of emoji-icon-font (MIT). Their license texts are in
`licenses/egui-fonts/`. Noto Emoji is not embedded. Fonts loaded from the
system at run time are not distributed with MISF.

## MISF

MISF Management Terminal is licensed under the GNU Affero General Public
License v3; the full text is in `LICENSE`.
