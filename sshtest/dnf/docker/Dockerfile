# A throwaway DNF host for the MISF fault tests: sshd, three accounts and a local
# package repository. Nothing here is a real credential; the tests publish the SSH port
# on 127.0.0.1 only and remove the container when they finish.
ARG BASE=rockylinux/rockylinux:9
FROM ${BASE}

RUN dnf -y install openssh-server sudo newt procps-ng rpm-build createrepo_c \
        shadow-utils hostname \
    && dnf clean all

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
    && printf '%s\n' \
        'HostKey /etc/ssh/ssh_host_ed25519_key' \
        'PermitRootLogin yes' \
        'PasswordAuthentication yes' \
        'KbdInteractiveAuthentication no' \
        'UseDNS no' \
        > /etc/ssh/sshd_config.d/00-misf.conf \
    && sed -i 's/^\(session\s\+\)required\(\s\+pam_loginuid\.so\)/\1optional\2/' /etc/pam.d/sshd

COPY build-packages.sh misf-scenario /usr/local/bin/
RUN chmod 0755 /usr/local/bin/build-packages.sh /usr/local/bin/misf-scenario \
    && build-packages.sh \
    && rpm -i --define '_pkgverify_level none' /opt/misf/pool/v1/*.rpm \
    && echo 'setting = changed-by-the-admin' > /etc/misf-conf.conf \
    # Only the local repository from here on: the tests never need the network.
    && rm -f /etc/yum.repos.d/* \
    && printf '%s\n' '[misf-test]' 'name=MISF sshtest' 'baseurl=file:///opt/misf/repo' \
        'enabled=1' 'gpgcheck=0' > /etc/yum.repos.d/misf-test.repo \
    && misf-scenario \
    && dnf clean all

EXPOSE 22
CMD ["/usr/sbin/sshd", "-D", "-e"]
