//! Explicit profile selection. Debug profiles never resolve/migrate the normal profile.
use anyhow::{Context, Result, bail, ensure};
use std::{ffi::OsString, path::PathBuf};

#[derive(Clone)]
pub struct RuntimePaths {
    pub store: PathBuf,
    pub debug: bool,
    pub debug_allow_passkey: bool,
}

impl RuntimePaths {
    pub fn from_args(args: impl IntoIterator<Item = OsString>) -> Result<Self> {
        let (directory, debug_allow_passkey) = debug_directory(args)?;
        match directory {
            Some(directory) => {
                std::fs::create_dir_all(&directory).context("Cannot create debug directory")?;
                let directory = directory
                    .canonicalize()
                    .context("Invalid debug directory")?;
                // Do not allow an explicit debug launch to mutate the default profile.
                if let Some(home) =
                    std::env::var_os("USERPROFILE").or_else(|| std::env::var_os("HOME"))
                {
                    for name in [".misf-management", ".msif-management"] {
                        let normal = PathBuf::from(&home).join(name);
                        ensure!(
                            normal.canonicalize().ok().as_ref() != Some(&directory),
                            "Debug directory must not be the normal profile directory"
                        );
                    }
                }
                Ok(Self {
                    store: directory.join("hosts.db"),
                    debug: true,
                    debug_allow_passkey,
                })
            }
            None => Ok(Self {
                store: crate::store::Store::default_path()?,
                debug: false,
                debug_allow_passkey: false,
            }),
        }
    }

    pub fn passkey_allowed(&self) -> bool {
        !self.debug || self.debug_allow_passkey
    }
}

fn debug_directory(args: impl IntoIterator<Item = OsString>) -> Result<(Option<PathBuf>, bool)> {
    let mut args = args.into_iter();
    let mut directory = None;
    let mut allow_passkey = false;
    while let Some(argument) = args.next() {
        if argument == "--debug.allow-passkey" {
            ensure!(!allow_passkey, "Duplicate --debug.allow-passkey");
            allow_passkey = true;
            continue;
        }
        ensure!(
            directory.is_none(),
            "Duplicate --debug.dir or unsupported argument"
        );
        let value = if argument == "--debug.dir" || argument == "--debug.dir=" {
            args.next().context("--debug.dir requires a directory")?
        } else if let Some(value) = argument
            .to_str()
            .and_then(|s| s.strip_prefix("--debug.dir="))
        {
            OsString::from(value)
        } else {
            bail!("Unsupported argument; use --debug.dir=<directory> [--debug.allow-passkey]");
        };
        ensure!(
            !value.is_empty() && !value.to_string_lossy().starts_with("--"),
            "--debug.dir requires a directory"
        );
        directory = Some(PathBuf::from(value));
    }
    ensure!(
        !allow_passkey || directory.is_some(),
        "--debug.allow-passkey requires an explicit --debug.dir"
    );
    Ok((directory, allow_passkey))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn debug_arguments_are_explicit_and_fail_closed() {
        for args in [
            vec!["--debug.dir=/tmp/test profile"],
            vec!["--debug.dir", "/tmp/test profile"],
            vec!["--debug.dir=", "/tmp/test profile"],
        ] {
            assert_eq!(
                debug_directory(args.into_iter().map(OsString::from)).unwrap(),
                (Some(PathBuf::from("/tmp/test profile")), false)
            );
        }
        for args in [
            vec!["--debug.dir"],
            vec!["--debug.dir="],
            vec!["--debug.dir", "--oops"],
            vec!["--oops"],
            vec!["--debug.dir=a", "--debug.dir=b"],
            vec!["--debug.allow-passkey"],
            vec![
                "--debug.dir=a",
                "--debug.allow-passkey",
                "--debug.allow-passkey",
            ],
        ] {
            assert!(debug_directory(args.into_iter().map(OsString::from)).is_err());
        }
    }

    #[test]
    fn hardware_testing_requires_explicit_opt_in_and_keeps_debug_profile() {
        for args in [
            vec!["--debug.allow-passkey", "--debug.dir=test"],
            vec!["--debug.dir=test", "--debug.allow-passkey"],
        ] {
            assert_eq!(
                debug_directory(args.into_iter().map(OsString::from)).unwrap(),
                (Some(PathBuf::from("test")), true)
            );
        }
    }

    #[test]
    fn explicit_debug_profile_is_resolved_without_populating_data() {
        let parent = tempfile::tempdir().unwrap();
        let directory = parent.path().join("isolated profile");
        let paths = RuntimePaths::from_args([
            OsString::from("--debug.dir="),
            directory.as_os_str().to_owned(),
        ])
        .unwrap();
        assert!(paths.debug);
        assert_eq!(
            paths.store,
            directory.canonicalize().unwrap().join("hosts.db")
        );
        assert_eq!(std::fs::read_dir(directory).unwrap().count(), 0);
    }
}
