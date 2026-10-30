# SQLite storage security — after-patch test report

Date: 2026-10-31
Project: MISF Management Terminal
Tested source: GitHub `origin/main` at `470927f`
Local follow-up: `acf0a0e` adds the `.daybrake/dangerzone/` ignore rule.

## Result

The latest source fixes the A7 hostile-WAL issue tested here. MISF rejects a database with a `hosts.db-wal` sidecar before opening it. The command exits with an error, and the supplied main database and WAL remain unchanged.

This result does not resolve every storage finding. SQLite does not authenticate individual pages, so a valid altered SQLite image can be read as changed data if it reaches the database reader. In plaintext mode, a valid data-page bit flip is accepted. In the encrypted A6 follow-up, the test directly passed a modified image to the vault's internal writer while unlocked; this is not an offline ciphertext attack and does not demonstrate tampering with a live MISF process through its public interface. A direct bit flip to an encrypted vault chunk is rejected by the container's authentication check. The Windows-only D1 check was not run in this Ubuntu environment.

## Verification

| Check | Result |
|---|---|
| `cargo build --locked -p misf-core` | Passed |
| `cargo test --locked --workspace` | Passed: 91 tests, 0 failures |
| A1–A5 hostile schema, application ID, and version cases | Rejected; source main-file hashes unchanged |
| A6 truncated database, invalid b-tree header, and invalid page size | Rejected; source main-file hashes unchanged |
| A6 valid-page bit flip in plaintext `hosts.db` | Not detected: `POC_SECRET` became `QOC_SECRET` and the stored value was returned |
| A6 malformed SQLite images inside an authenticated vault | Truncated image, invalid page size, and invalid b-tree freeblock pointer were rejected after decryption |
| A6 valid-page bit flip passed to the unlocked vault's internal image writer by the test harness | Accepted as `rsh-secret-sentinel`; SQLite has no page-level authentication tag (details below) |
| A6 offline bit flip to an encrypted vault chunk | Rejected by the container authentication check before the SQLite image was read |
| A7 malicious WAL | Rejected before SQLite open; source main hash unchanged; WAL remained present with identical SHA-256 and 4,152-byte length |
| B1–B2 encrypted storage lifecycle | Passed; plaintext removed before encrypted writes, no plaintext returned, no secret found in vault bytes |
| B3 secure deletion | Passed; deleted secret absent from database bytes |
| B4 interrupted write recovery | Passed; rollback journal recovered and `quick_check` returned `ok` |
| C1 JSON import | Passed; source JSON hash unchanged and host/password preserved in SQLite |
| C2 migration | Passed: backup matched v2 source and had mode `0600`; failed migration rolled back; v2 binary refused v3 without changing the database |
| D2 concurrent writers and D3 permissions | Passed; both writers completed, lock timeout was about five seconds, database mode was `0600` |
| D1 Windows DLL side-loading | Not tested; requires a Windows runner |
| E1 same-account plaintext tampering | Not rerun in this pass; remains a known limitation for a process that can write the profile |

The A7 fixture staged an extra trigger in the WAL. MISF returned the WAL refusal error before replaying it. The test measured the source file and sidecar bytes before opening a disposable inspection copy, so the inspection itself could not consume the source WAL.

## Detailed A6: valid SQLite page bit flip

### What was tested

SQLite's `quick_check` and schema validation check database structure; SQLite pages do not carry a cryptographic authentication tag for each stored value. A bit change that leaves a text value and its page structurally valid can therefore appear to SQLite as ordinary, changed data.

- In the plaintext POC, a valid data-page bit flip changed `POC_SECRET` to `QOC_SECRET`. MISF opened the database and returned the changed value; the file was not rewritten by that read.
- In the encrypted follow-up, a temporary Rust test created a throwaway encrypted vault, read the database into a SQLite image, flipped one bit in `ssh-secret-sentinel`, and passed the modified image to the private `VaultFile::write_image` method. Reading the image back returned `rsh-secret-sentinel`. This exercises the internal writer with a valid altered image while the vault is unlocked.
- As a control, the test flipped a byte in an encrypted vault chunk on disk without first decrypting it. Opening the vault failed its container authentication check, before SQLite read the image.
- Truncated images, an invalid page size, and a malformed b-tree freeblock pointer were rejected after decryption. Those are structural corruption cases, unlike the valid-value change above.

### What this establishes—and what it does not

The result establishes that SQLite structural checks do not detect an altered but still valid value, and that the encrypted container rejects unauthenticated changes to ciphertext at rest. The encrypted acceptance case used a test harness that directly called a private internal writer. The harness temporarily injected a Rust test into `core/src/vault.rs` and restored that file byte-for-byte after the run. It did not attach a debugger, inject code into, or otherwise tamper with a running MISF process, and it did not exercise a public user-interface action that accepts arbitrary SQLite images.

In encrypted mode, MISF keeps the decrypted SQLite image in memory and does not write a plaintext `.db` file. Consequently, editing the locked `hosts.lbx` file alone cannot reproduce the accepted-image case; the ciphertext authentication check rejects that change. Reproducing it against a live installation would require control over the decrypted data or the process/write path while the vault is unlocked (or an equivalent authorized way to supply and re-encrypt a modified image). The POC does not show that an ordinary same-account process can do this, and it does not establish that root is required. The plaintext result has a simpler boundary: a process with write access to `hosts.db` can alter valid stored values.

### Security impact and handling

Treat A6 as a known integrity limitation of ordinary SQLite pages, not as a bypass of the encrypted container's authentication. In plaintext mode, a same-account writer can change a valid value, potentially including a host address or credential, without `quick_check` detecting the bit change. In encrypted mode, offline file-only bit flips are rejected; the encrypted follow-up's valid-image acceptance assumes the modified data already reaches the unlocked writer. Product and security claims must not promise that SQLite detects every bit flip. If detection of unauthorized valid-value changes is required, it needs a separate integrity design with a secret and a clearly defined key/access boundary; encryption-at-rest authentication already protects the container against offline ciphertext modification.

The repeatable encrypted harness is `.daybrake/dangerzone/probe_encrypted_a6.py`.

The workflow integration tests bind temporary local TCP sockets. The sandbox denied those binds on the first run; the complete workspace test command was rerun with the required permission and all 91 tests passed.

All POC profiles used temporary directories and fake credentials. No real MISF profile was used.
