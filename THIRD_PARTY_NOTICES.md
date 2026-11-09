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

## Slint

The user interface is built with [Slint](https://slint.dev) 1.18.1 (the `slint`
and `i-slint-*` crates), copyright SixtyFPS GmbH. Slint is offered under
GPL-3.0-only, the Slint Royalty-free License 2.0, or the Slint Software License
3.0; MISF uses it under **GPL-3.0-only**, which section 13 of the GNU AGPL v3
allows combining with MISF. The full GPL v3 text is in
`licenses/THIRD_PARTY_LICENSES.html`. The source of the Slint version used is
available from crates.io and https://github.com/slint-ui/slint.

## Noto Sans TC

The interface font, Noto Sans TC (variable weight,
`assets/fonts/NotoSansTC-VariableFont_wght.ttf`), is embedded in the binary.
Copyright 2014-2021 Adobe (http://www.adobe.com/), with Reserved Font Name
'Source'. It is licensed under the SIL Open Font License 1.1; the full text is
in `assets/fonts/OFL.txt` (distributed as `licenses/NotoSansTC/OFL.txt`).
Other fonts, such as the monospace font for commands, are loaded from the system
at run time and are not distributed with MISF.

## MISF

MISF Management Terminal is licensed under the GNU Affero General Public
License v3; the full text is in `LICENSE`.
