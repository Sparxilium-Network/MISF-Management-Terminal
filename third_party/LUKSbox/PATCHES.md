# MISF FIDO2 compatibility patch

Modified by MISF Management Terminal on 2026-10-30.

Source: [PentHertz/LUKSbox v0.6.1](https://github.com/PentHertz/LUKSbox/tree/69448f708f1328d672deb595e359579d630cb0b0/crates/luksbox-fido2),
commit `69448f708f1328d672deb595e359579d630cb0b0`.
The `luksbox-fido2/` directory contains the upstream crate's `src/` and
`build.rs`, with a standalone manifest. Apache-2.0 license and upstream notice
are preserved in this directory's `LICENSE` and `NOTICE` files.

The root Cargo patch replaces only `luksbox-fido2`. The container's core,
format, VFS, and PQ libraries remain pinned to the unmodified upstream commit.
This is a local compatibility copy, not a new upstream LUKSbox release.

## Changes

- `Cargo.toml`: resolve standalone dependencies instead of inheriting an
  upstream workspace. Upgrade P-256 to 0.14.0, AES to 0.9.3, HMAC to 0.13.0,
  SHA-2 to 0.11.1, Windows bindings to 0.62.2, and bindgen to 0.73.2.
  Use getrandom 0.4.3 instead of rand_core 0.6's OsRng. Other dependencies
  use their current stable releases; exact resolution is in the root lockfile.
  Upstream examples are not included; the library is not publishable.
- `src/protocol.rs`: use P-256's `Generate` API, AES cipher 0.5 block/trait
  types, and HMAC's explicit `KeyInit` import. ECDH, SHA-256, AES-256-CBC,
  zero IV, salt authentication, and wire encoding remain the same.
- `src/mock.rs`, `src/hid.rs`, `src/webauthn.rs`: replace OsRng calls with
  fallible OS randomness. Windows API types now come from windows 0.62.
- Modified Rust files carry an explicit MISF modification notice.
- `build.rs`, `authenticator.rs`, `error.rs`, `ffi.rs`, `lib.rs`, and
  `webauthn_paths.rs` are unchanged from the pinned source, ignoring line endings.

The application's own `core/src/windows_passkey.rs` remains the Windows
transport used by MISF. The copied upstream WebAuthn implementation is not
substituted for it. Linux continues to use the copied upstream libfido2 backend.
RP ID, PRF salt conventions, keyslot representation, and vault format are
unchanged. No known vulnerability is suppressed in Cargo audit.

## Validation and maintenance

This crate is a workspace member, so `cargo test --locked --workspace
--all-features` runs its original protocol and mock tests, including ECDH
agreement, complete hmac-secret exchange, tamper rejection, and OpenSSL
AES/HMAC test vectors. Workspace Clippy includes the local source as well.
See `docs/dependency-updates.md` for application compatibility tests and limits.

When upstream updates its dependency stack, compare its changes against this
copy, retest existing containers and physical authenticators, and remove the
Cargo patch once an upstream release provides the needed versions and behavior.
