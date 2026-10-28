# Third-party notices

MISF embeds the `luksbox-core`, `luksbox-format`, `luksbox-pq`, and
`luksbox-vfs` Rust libraries from [PentHertz/LUKSbox](https://github.com/PentHertz/LUKSbox).

- Upstream version: 0.6.1
- Pinned source commit: `69448f708f1328d672deb595e359579d630cb0b0`
- Copyright: 2026 Penthertz
- License: Apache License 2.0
- Unmodified license and notice: `third_party/LUKSbox/LICENSE` and
  `third_party/LUKSbox/NOTICE` (distributed as `licenses/LUKSbox/` in the portable app).

No upstream source modifications are included. MISF uses its own GUI and
storage adapter. This distribution does not include LUKSbox's mount drivers,
WinFsp, libfido2, TPM adapters, or the upstream GUI. The integration is not an
endorsement by Penthertz. LUKSbox and Penthertz names retain their upstream
trademark ownership.

Other Rust dependencies retain their respective upstream licenses and
copyrights; exact resolved packages are recorded in Cargo.lock.
