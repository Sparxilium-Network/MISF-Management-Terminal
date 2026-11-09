//! YUM (CentOS 7 and similar). Listing and parsing are shared with DNF in `rpm.rs`.

use super::Commands;

pub const UPDATE: &str = "LC_ALL=C yum -q -y makecache";
pub const SUDO_UPDATE: &str = "LC_ALL=C sudo -S -p '__MISF_SUDO_PROMPT__' -- yum -q -y makecache";
// Listings read root's metadata cache, so they go through sudo like the update.
pub const LIST: &str = "LC_ALL=C sh -c 'yum -q -C check-update; rc=$?; echo __MISF_INSTALLED__; rpm -qa --qf \"%{NAME}.%{ARCH} %|EPOCH?{%{EPOCH}:}:{}|%{VERSION}-%{RELEASE}\\n\"; exit $rc'";
pub const SUDO_LIST: &str = "LC_ALL=C sudo -S -p '__MISF_SUDO_PROMPT__' -- sh -c 'yum -q -C check-update; rc=$?; echo __MISF_INSTALLED__; rpm -qa --qf \"%{NAME}.%{ARCH} %|EPOCH?{%{EPOCH}:}:{}|%{VERSION}-%{RELEASE}\\n\"; exit $rc'";
pub const UPGRADE: &str = "LC_ALL=C yum -y upgrade";
pub const SUDO_UPGRADE: &str = "LC_ALL=C sudo -S -p '__MISF_SUDO_PROMPT__' -- yum -y upgrade";

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
