# SQLite storage security — after-patch test report

Date: 2026-10-31
Project: MISF Management Terminal
Tested source: GitHub `origin/main` at `470927f`
Local follow-up: `acf0a0e` adds the `.daybrake/dangerzone/` ignore rule.

## Result

The latest source fixes the A7 hostile-WAL issue tested here. MISF rejects a database with a `hosts.db-wal` sidecar before opening it. The command exits with an error, and the supplied main database and WAL remain unchanged.

This result does not resolve every storage finding. A valid plaintext SQLite page bit flip still changes a stored value without being detected, and the Windows-only D1 check was not run in this Ubuntu environment.

## Verification

| Check | Result |
|---|---|
| `cargo build --locked -p misf-core` | Passed |
| `cargo test --locked --workspace` | Passed: 91 tests, 0 failures |
| A1–A5 hostile schema, application ID, and version cases | Rejected; source main-file hashes unchanged |
| A6 truncated database, invalid b-tree header, and invalid page size | Rejected; source main-file hashes unchanged |
| A6 valid-page bit flip | Not detected: `POC_SECRET` became `QOC_SECRET` and the stored value was returned |
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

The workflow integration tests bind temporary local TCP sockets. The sandbox denied those binds on the first run; the complete workspace test command was rerun with the required permission and all 91 tests passed.

All POC profiles used temporary directories and fake credentials. No real MISF profile was used.
