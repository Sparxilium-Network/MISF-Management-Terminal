//! DNF (Fedora, RHEL 8 and later). Listing and parsing are shared with YUM in `rpm.rs`.

use super::Commands;

pub const UPDATE: &str = "LC_ALL=C dnf -q -y makecache --refresh";
pub const SUDO_UPDATE: &str =
    "LC_ALL=C sudo -S -p '__MISF_SUDO_PROMPT__' -- dnf -q -y makecache --refresh";
// Listings read root's metadata cache, so they go through sudo like the update.
pub const LIST: &str = "LC_ALL=C sh -c 'dnf -q -C check-update; rc=$?; echo __MISF_INSTALLED__; rpm -qa --qf \"%{NAME}.%{ARCH} %|EPOCH?{%{EPOCH}:}:{}|%{VERSION}-%{RELEASE}\\n\"; exit $rc'";
pub const SUDO_LIST: &str = "LC_ALL=C sudo -S -p '__MISF_SUDO_PROMPT__' -- sh -c 'dnf -q -C check-update; rc=$?; echo __MISF_INSTALLED__; rpm -qa --qf \"%{NAME}.%{ARCH} %|EPOCH?{%{EPOCH}:}:{}|%{VERSION}-%{RELEASE}\\n\"; exit $rc'";
pub const UPGRADE: &str = "LC_ALL=C dnf -y upgrade";
pub const SUDO_UPGRADE: &str = "LC_ALL=C sudo -S -p '__MISF_SUDO_PROMPT__' -- dnf -y upgrade";

pub(super) const ROOT: Commands = Commands {
    update: UPDATE,
    list: LIST,
    upgrade: UPGRADE,
    dist_upgrade: None,
    simulate: None,
    status: None,
};
pub(super) const SUDO: Commands = Commands {
    update: SUDO_UPDATE,
    list: SUDO_LIST,
    upgrade: SUDO_UPGRADE,
    dist_upgrade: None,
    simulate: None,
    status: None,
};

#[cfg(test)]
mod tests {
    #[test]
    fn listings_print_the_marker_the_parser_splits_on() {
        for list in [super::LIST, super::SUDO_LIST] {
            assert!(list.contains(super::super::rpm::INSTALLED_MARKER));
        }
    }
}
