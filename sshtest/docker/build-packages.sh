#!/bin/sh
# Builds the test packages. Version 1.0 of each is installed in the image; version 2.0
# waits in the pool until `misf-scenario` publishes it. Each 2.0 misbehaves in one way.
set -eu
pool=/opt/misf/pool
work=$(mktemp -d)
mkdir -p "$pool/v1" "$pool/v2"

# start <name> <version> [depends]: begins a package; $root is its tree.
start() {
    root=$work/$1_$2
    mkdir -p "$root/DEBIAN"
    {
        echo "Package: $1"
        echo "Version: $2"
        echo "Architecture: all"
        echo "Maintainer: MISF sshtest <sshtest@misf.invalid>"
        echo "Section: misc"
        echo "Priority: optional"
        if [ -n "${3:-}" ]; then echo "Depends: $3"; fi
        echo "Description: MISF sshtest fixture"
        echo " Built inside the test image; never published."
    } > "$root/DEBIAN/control"
}

# script <name>: stdin becomes that maintainer script.
script() {
    cat > "$root/DEBIAN/$1"
    chmod 0755 "$root/DEBIAN/$1"
}

# conffile <content>: ships /etc/misf-conf.conf as a conffile.
conffile() {
    mkdir -p "$root/etc"
    echo "$1" > "$root/etc/misf-conf.conf"
    echo /etc/misf-conf.conf > "$root/DEBIAN/conffiles"
}

# finish <v1|v2>: writes the .deb.
finish() {
    dpkg-deb --root-owner-group --build "$root" "$pool/$1" > /dev/null
}

for name in misf-ok misf-fail misf-slow misf-noise misf-eline misf-marker misf-dialog \
    misf-tui misf-prompt misf-debconf misf-kept; do
    start "$name" 1.0
    finish v1
done
start misf-conf 1.0
conffile 'setting = packaged-v1'
finish v1

# Upgrades cleanly and asks for a reboot the way a kernel package does.
start misf-ok 2.0
script postinst <<'EOF'
#!/bin/sh
set -e
echo "misf-ok: configured"
touch /var/run/reboot-required
echo misf-ok >> /var/run/reboot-required.pkgs
EOF
finish v2

# Unpacks, then fails to configure: the upgrade stops half-way.
start misf-fail 2.0
script postinst <<'EOF'
#!/bin/sh
echo "misf-fail: postinst is about to fail" >&2
exit 1
EOF
finish v2

# Takes a while to configure, so a test can cut the connection or kill dpkg meanwhile.
start misf-slow 2.0
script postinst <<'EOF'
#!/bin/sh
set -e
seconds=$(cat /run/misf-slow.seconds 2>/dev/null || echo 20)
touch /run/misf-slow.started
echo "misf-slow: working for $seconds seconds"
sleep "$seconds"
touch /run/misf-slow.done
echo "misf-slow: done"
EOF
finish v2

# Prints what a terminal log should never be trusted to contain.
start misf-noise 2.0
script postinst <<'EOF'
#!/bin/sh
printf 'misf-noise: start\n'
printf '\033[1;31mred text\033[0m then \033]0;window title\007after the title\n'
printf 'progress 10%%\rprogress 50%%\rprogress 100%%\n'
printf 'invalid utf-8: \377\376\200 end\n'
printf 'nul and bell: a\000b\007c\n'
printf 'backspaces: abc\010\010\010xyz\n'
printf 'wide text: \344\270\255\346\226\207\350\274\270\345\207\272\n'
head -c 100000 /dev/zero | tr '\000' 'x'
printf '\nmisf-noise: after the long line\n'
printf 'an escape that never ends: \033]0;swallowed?\n'
printf 'misf-noise: end\n'
EOF
finish v2

# Succeeds, but one of its lines looks like an APT error.
start misf-eline 2.0
script postinst <<'EOF'
#!/bin/sh
echo "E: this line is only text printed by a package script"
EOF
finish v2

# Prints MISF's sudo prompt marker and records whatever arrives on its input.
start misf-marker 2.0
script postinst <<'EOF'
#!/bin/sh
printf 'misf-marker says __MISF_SUDO_PROMPT__\n'
# Reads in this script's own process group: `timeout` would move the reader to the
# background, where reading a terminal stops it instead (and uutils never wakes it).
# A background command's input is /dev/null unless it comes from another descriptor.
exec 3<&0
head -n 1 <&3 > /run/misf-marker.stdin 2>/dev/null &
reader=$!
sleep 3
kill "$reader" 2>/dev/null
wait "$reader" 2>/dev/null
echo "misf-marker: done"
exit 0
EOF
finish v2

# Opens a full-screen dialog by itself, ignoring DEBIAN_FRONTEND; whiptail refuses a
# terminal that cannot position the cursor.
start misf-dialog 2.0
script postinst <<'EOF'
#!/bin/sh
whiptail --title "MISF sshtest" --msgbox "A package opened a full-screen dialog." 10 60
EOF
finish v2

# The same dialog from a script that also insists on a capable terminal type, so the
# dialog really is drawn and waits for a key nobody will press.
start misf-tui 2.0
script postinst <<'EOF'
#!/bin/sh
touch /run/misf-tui.started
TERM=xterm whiptail --title "MISF sshtest" --msgbox "A package opened a full-screen dialog." 10 60
status=$?
echo "$status" > /run/misf-tui.status
exit "$status"
EOF
finish v2

# Asks a plain question on its input and waits for the answer.
start misf-prompt 2.0
script postinst <<'EOF'
#!/bin/sh
touch /run/misf-prompt.started
printf 'misf-prompt: overwrite the local changes? [y/N] '
read -r answer
echo "$answer" > /run/misf-prompt.answer
echo "misf-prompt: answered"
EOF
finish v2

# Asks a critical debconf question; DEBIAN_FRONTEND=noninteractive must answer it.
start misf-debconf 2.0 debconf
cat > "$root/DEBIAN/templates" <<'EOF'
Template: misf-debconf/answer
Type: string
Default: default-answer
Description: A question nobody is there to answer
 Asked at critical priority on every upgrade.
EOF
script config <<'EOF'
#!/bin/sh
set -e
. /usr/share/debconf/confmodule
db_fset misf-debconf/answer seen false
db_input critical misf-debconf/answer || true
db_go || true
EOF
script postinst <<'EOF'
#!/bin/sh
set -e
. /usr/share/debconf/confmodule
db_get misf-debconf/answer
echo "$RET" > /run/misf-debconf.answer
EOF
finish v2

# Changes a conffile the admin also changed: dpkg would ask which one to keep.
start misf-conf 2.0
conffile 'setting = packaged-v2'
finish v2

# Needs a package that is not installed, so a plain upgrade keeps it back.
start misf-kept 2.0 misf-newdep
finish v2
start misf-newdep 1.0
finish v2

rm -rf "$work"
