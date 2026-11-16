# A throwaway APT host for the MISF fault tests: sshd, three accounts and a local
# package repository. Nothing here is a real credential; the tests publish the SSH port
# on 127.0.0.1 only and remove the container when they finish.
ARG BASE=debian:13
FROM ${BASE}

ENV DEBIAN_FRONTEND=noninteractive
RUN apt-get update \
    && apt-get install -y --no-install-recommends \
        openssh-server sudo whiptail apt-utils procps needrestart \
    && rm -rf /var/lib/apt/lists/*

# root signs in directly; misfadmin's sudo asks for a password; misfdeploy's does not.
RUN useradd --create-home --shell /bin/sh misfadmin \
    && useradd --create-home --shell /bin/sh misfdeploy \
    && printf '%s\n' 'root:misf-root-pw' 'misfadmin:misf-admin-pw' 'misfdeploy:misf-deploy-pw' | chpasswd \
    && printf '%s\n' 'misfadmin ALL=(ALL:ALL) ALL' 'misfdeploy ALL=(ALL:ALL) NOPASSWD: ALL' > /etc/sudoers.d/misf \
    && chmod 0440 /etc/sudoers.d/misf \
    && visudo -c -q

# MISF verifies Ed25519 and ECDSA host keys only.
RUN mkdir -p /run/sshd \
    && rm -f /etc/ssh/ssh_host_*_key /etc/ssh/ssh_host_*_key.pub \
    && ssh-keygen -q -t ed25519 -N '' -f /etc/ssh/ssh_host_ed25519_key \
    && printf '%s\n' \
        'HostKey /etc/ssh/ssh_host_ed25519_key' \
        'PermitRootLogin yes' \
        'PasswordAuthentication yes' \
        'KbdInteractiveAuthentication no' \
        'UseDNS no' \
        > /etc/ssh/sshd_config.d/misf.conf

COPY build-packages.sh misf-scenario /usr/local/bin/
RUN chmod 0755 /usr/local/bin/build-packages.sh /usr/local/bin/misf-scenario \
    && build-packages.sh \
    && dpkg -i /opt/misf/pool/v1/*.deb \
    && echo 'setting = changed-by-the-admin' > /etc/misf-conf.conf \
    # Only the local repository from here on: the tests never need the network.
    && rm -f /etc/apt/sources.list /etc/apt/sources.list.d/* \
    && echo 'deb [trusted=yes] file:/opt/misf/repo ./' > /etc/apt/sources.list \
    && misf-scenario

EXPOSE 22
CMD ["/usr/sbin/sshd", "-D", "-e"]
