//! apk against a real Alpine container: failing package scripts, packages that cannot be
//! installed, unreadable or unsigned indexes, lost connections, output no log should
//! trust, and package scripts that wait for a person.
//!
//! Every test starts its own container, so they can run in any order, in parallel and
//! again and again. They need Docker and are ignored by default:
//!
//! ```text
//! cargo test -p misf-sshtest --test apk -- --ignored
//! ```
//!
//! One test takes about four minutes and only runs when `MISF_SSHTEST_SLOW=1` is set.
//!
//! The assertions describe what MISF does today. Where that is a gap rather than the
//! wanted behaviour, the test says `GAP` and `sshtest/apk/README.md` lists it; fixing the
//! gap in core means changing that assertion with it.

mod connection;
mod failures;
mod output;
mod prompts;
mod support;
mod upgrade;
