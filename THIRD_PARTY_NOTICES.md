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

Other Rust dependencies retain their respective upstream licenses and
copyrights; exact resolved packages are recorded in Cargo.lock.
