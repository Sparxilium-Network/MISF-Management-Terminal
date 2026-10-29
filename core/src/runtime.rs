//! Explicit profile selection. Debug profiles never resolve/migrate the normal profile.
use anyhow::{Context, Result, bail, ensure};
use std::{ffi::OsString, path::PathBuf};

#[derive(Clone)]
pub struct RuntimePaths {
    pub store: PathBuf,
    pub debug: bool,
}

impl RuntimePaths {
    pub fn from_args(args: impl IntoIterator<Item = OsString>) -> Result<Self> {
        match debug_directory(args)? {
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
                    store: directory.join("hosts.json"),
                    debug: true,
                })
            }
            None => Ok(Self {
                store: crate::store::Store::default_path()?,
                debug: false,
            }),
        }
    }
}

fn debug_directory(args: impl IntoIterator<Item = OsString>) -> Result<Option<PathBuf>> {
    let mut args = args.into_iter();
    let mut directory = None;
    while let Some(argument) = args.next() {
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
            bail!("Unsupported argument; use --debug.dir=<directory>");
        };
        ensure!(
            !value.is_empty() && !value.to_string_lossy().starts_with("--"),
            "--debug.dir requires a directory"
        );
        directory = Some(PathBuf::from(value));
    }
    Ok(directory)
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
                Some(PathBuf::from("/tmp/test profile"))
            );
        }
        for args in [
            vec!["--debug.dir"],
            vec!["--debug.dir="],
            vec!["--debug.dir", "--oops"],
            vec!["--oops"],
            vec!["--debug.dir=a", "--debug.dir=b"],
        ] {
            assert!(debug_directory(args.into_iter().map(OsString::from)).is_err());
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
            directory.canonicalize().unwrap().join("hosts.json")
        );
        assert_eq!(std::fs::read_dir(directory).unwrap().count(), 0);
    }
}
