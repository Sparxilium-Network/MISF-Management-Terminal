# A throwaway Arch Linux host for the MISF fault tests: sshd, sudo, pacman-contrib's
# checkupdates, three accounts and a local package repository. Nothing here is a real
# credential; the tests publish the SSH port on 127.0.0.1 only and remove the container
# when they finish.
ARG BASE=archlinux:latest
FROM ${BASE}

# checkupdates (pacman-contrib) needs fakeroot; makepkg builds the test packages; libnewt
# is whiptail. The image's own repositories are used this once.
RUN pacman -Syu --noconfirm --needed openssh sudo pacman-contrib fakeroot libnewt \
        procps-ng \
    && pacman -Scc --noconfirm

# root signs in directly; misfadmin's sudo asks for a password; misfdeploy's does not.
RUN useradd --create-home --shell /bin/sh misfadmin \
    && useradd --create-home --shell /bin/sh misfdeploy \
    && printf '%s\n' 'root:misf-root-pw' 'misfadmin:misf-admin-pw' 'misfdeploy:misf-deploy-pw' | chpasswd \
    && printf '%s\n' 'misfadmin ALL=(ALL:ALL) ALL' 'misfdeploy ALL=(ALL:ALL) NOPASSWD: ALL' > /etc/sudoers.d/misf \
    && chmod 0440 /etc/sudoers.d/misf \
    && visudo -c -q

# MISF verifies Ed25519 and ECDSA host keys only. A container has no audit session to
# join and may carry the boot-time marker that turns away everyone but root.
RUN rm -f /etc/ssh/ssh_host_*_key /etc/ssh/ssh_host_*_key.pub /run/nologin /etc/nologin \
    && ssh-keygen -q -t ed25519 -N '' -f /etc/ssh/ssh_host_ed25519_key \
    && mkdir -p /etc/ssh/sshd_config.d \
    && printf '%s\n' \
        'HostKey /etc/ssh/ssh_host_ed25519_key' \
        'PermitRootLogin yes' \
        'PasswordAuthentication yes' \
        'KbdInteractiveAuthentication no' \
        'UseDNS no' \
        > /etc/ssh/sshd_config.d/00-misf.conf \
    && (grep -q '^Include /etc/ssh/sshd_config.d' /etc/ssh/sshd_config \
        || sed -i '1i Include /etc/ssh/sshd_config.d/*.conf' /etc/ssh/sshd_config) \
    && sed -i 's/^\(session\s\+\)required\(\s\+pam_loginuid\.so\)/\1optional\2/' /etc/pam.d/* \
    && useradd --create-home builder

COPY build-packages.sh misf-scenario /usr/local/bin/
RUN chmod 0755 /usr/local/bin/build-packages.sh /usr/local/bin/misf-scenario \
    && build-packages.sh \
    # Only the local repository from here on: the tests never need the network. The
    # packages are unsigned test fixtures.
    && sed -i -e '/^\[core\]/,$d' -e 's/^SigLevel.*/SigLevel = Never/' \
        -e 's/^LocalFileSigLevel.*/LocalFileSigLevel = Never/' /etc/pacman.conf \
    && printf '%s\n' '[misf-test]' 'Server = file:///opt/misf/repo' >> /etc/pacman.conf \
    && rm -f /var/lib/pacman/sync/* \
    && misf-scenario --v1 \
    && pacman -Sy --noconfirm \
    && pacman -S --noconfirm $(cat /opt/misf/names) \
    && echo 'setting = changed-by-the-admin' > /etc/misf-conf.conf \
    && misf-scenario \
    && pacman -Sy --noconfirm \
    && pacman -Scc --noconfirm

EXPOSE 22
CMD ["/usr/bin/sshd", "-D", "-e"]
