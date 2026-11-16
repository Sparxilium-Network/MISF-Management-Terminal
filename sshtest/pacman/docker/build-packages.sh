#!/bin/sh
# Builds the test packages with makepkg. Version 1.0 of each is installed in the image;
# version 2.0 waits in the pool until `misf-scenario` publishes it. Each 2.0 misbehaves in
# one way, mostly from its post_upgrade function.
set -eu
pool=/opt/misf/pool
work=$(mktemp -d)
mkdir -p "$pool/v1" "$pool/v2"
chown builder "$work"

# build <v1|v2> <name> <version>: writes the package. Set beforehand, and cleared after:
# DEPENDS, CONF (content of the config file it ships), PRE and POST (pre_upgrade and
# post_upgrade bodies).
build() {
    dir=$work/$2-$3
    mkdir -p "$dir"
    {
        if [ -n "${PRE:-}" ]; then printf 'pre_upgrade() {\n%s\n}\n' "$PRE"; fi
        if [ -n "${POST:-}" ]; then printf 'post_upgrade() {\n%s\n}\n' "$POST"; fi
    } > "$dir/$2.install"
    {
        echo "pkgname=$2"
        echo "pkgver=$3"
        echo "pkgrel=1"
        echo "pkgdesc='MISF sshtest fixture'"
        echo "arch=(any)"
        echo "license=(MIT)"
        echo "depends=(${DEPENDS:-})"
        echo "options=(!strip !debug)"
        if [ -s "$dir/$2.install" ]; then echo "install=$2.install"; fi
        if [ -n "${CONF:-}" ]; then echo "backup=(etc/misf-conf.conf)"; fi
        echo "package() {"
        echo "    mkdir -p \"\$pkgdir\"/usr/share/misf"
        echo "    echo $3 > \"\$pkgdir\"/usr/share/misf/$2"
        if [ -n "${CONF:-}" ]; then
            echo "    mkdir -p \"\$pkgdir\"/etc"
            echo "    echo '$CONF' > \"\$pkgdir\"/etc/misf-conf.conf"
        fi
        echo "}"
    } > "$dir/PKGBUILD"
    chown -R builder "$dir"
    su builder -s /bin/sh -c "cd '$dir' && PKGDEST='$dir' makepkg --nodeps --noconfirm --skipinteg >/dev/null 2>&1"
    mv "$dir/$2-$3-1-any.pkg.tar.zst" "$pool/$1/"
    DEPENDS='' CONF='' PRE='' POST=''
}

names="misf-ok misf-fail misf-prefail misf-slow misf-noise misf-eline misf-marker \
    misf-prompt misf-dialog misf-tui misf-tty misf-dep misf-broken"
for name in $names; do
    build v1 "$name" 1.0
done
CONF='setting = packaged-v1'
build v1 misf-conf 1.0
echo "$names misf-conf" > /opt/misf/names

# Upgrades cleanly.
POST='echo "misf-ok: configured"'
build v2 misf-ok 2.0

# Installs, then its post_upgrade fails.
POST='echo "misf-fail: the post script is about to fail" >&2
return 1'
build v2 misf-fail 2.0

# Its pre_upgrade fails.
PRE='echo "misf-prefail: the pre script is about to fail" >&2
return 1'
build v2 misf-prefail 2.0

# Takes a while, so a test can cut the connection or kill pacman meanwhile.
POST='seconds=$(cat /run/misf-slow.seconds 2>/dev/null || echo 20)
touch /run/misf-slow.started
echo "misf-slow: working for $seconds seconds"
sleep "$seconds"
touch /run/misf-slow.done
echo "misf-slow: done"'
build v2 misf-slow 2.0

# Prints what a terminal log should never be trusted to contain.
POST='printf "misf-noise: start\n"
printf "\033[1;31mred text\033[0m then \033]0;window title\007after the title\n"
printf "progress 10\rprogress 50\rprogress 100\n"
printf "nul and bell: a\000b\007c\n"
printf "backspaces: abc\010\010\010xyz\n"
printf "invalid utf-8: \377\376\200 end\n"
printf "wide text: \344\270\255\346\226\207\350\274\270\345\207\272\n"
head -c 100000 /dev/zero | tr "\000" x
printf "\nmisf-noise: after the long line\n"
printf "an escape that never ends: \033]0;swallowed?\n"
printf "misf-noise: end\n"'
build v2 misf-noise 2.0

# Succeeds, but one of its lines looks like a pacman error.
POST='echo "error: this line is only text printed by a package script"'
build v2 misf-eline 2.0

# Prints MISF's sudo prompt marker and records whatever arrives on its input.
POST='printf "misf-marker says __MISF_SUDO_PROMPT__\n"
exec 3<&0
head -n 1 <&3 > /run/misf-marker.stdin 2>/dev/null &
reader=$!
sleep 3
kill "$reader" 2>/dev/null
wait "$reader" 2>/dev/null
echo "misf-marker: done"
return 0'
build v2 misf-marker 2.0

# Asks a plain question on its input and waits for the answer.
POST='touch /run/misf-prompt.started
printf "misf-prompt: overwrite the local changes? [y/N] "
read -r answer
echo "answer=$answer" > /run/misf-prompt.answer
echo "misf-prompt: answered"
return 0'
build v2 misf-prompt 2.0

# Opens a full-screen dialog; whiptail refuses a terminal that cannot position the cursor.
POST='whiptail --title "MISF sshtest" --msgbox "A package opened a full-screen dialog." 10 60
return 0'
build v2 misf-dialog 2.0

# The same dialog from a script that insists on a capable terminal and goes straight to
# the controlling terminal, whatever its own input and output are.
POST='touch /run/misf-tui.started
TERM=xterm whiptail --title "MISF sshtest" --msgbox "A package opened a full-screen dialog." 10 60 < /dev/tty > /dev/tty 2>&1
echo "$?" > /run/misf-tui.status
return 0'
build v2 misf-tui 2.0

# Asks a question on the controlling terminal instead of its input.
POST='touch /run/misf-tty.started
if ! { printf "misf-tty: overwrite the local changes? [y/N] " > /dev/tty; } 2>/dev/null; then
    echo "no terminal" > /run/misf-tty.answer
    echo "misf-tty: there is no terminal to ask on"
    return 0
fi
read -r answer < /dev/tty
echo "answer=$answer" > /run/misf-tty.answer
echo "misf-tty: answered"
return 0'
build v2 misf-tty 2.0

# Changes a config file the admin also changed.
CONF='setting = packaged-v2'
build v2 misf-conf 2.0

# Needs a package that is not installed yet, which the upgrade then installs.
DEPENDS='misf-newdep'
build v2 misf-dep 2.0
build v2 misf-newdep 1.0

# Needs a package no repository has, so it cannot be upgraded at all.
DEPENDS='misf-nowhere'
build v2 misf-broken 2.0

rm -rf "$work"
