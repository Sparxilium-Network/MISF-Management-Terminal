# A throwaway Alpine host for the MISF fault tests: sshd, sudo, three accounts and a
# local signed package repository. Nothing here is a real credential; the tests publish
# the SSH port on 127.0.0.1 only and remove the container when they finish.
ARG BASE=alpine:3.24
FROM ${BASE}

# Alpine ships doas; MISF needs sudo for accounts other than root. abuild builds and
# signs the test packages; newt is whiptail.
RUN apk add --no-cache openssh-server sudo abuild newt procps

# root signs in directly; misfadmin's sudo asks for a password; misfdeploy's does not.
RUN adduser -D -s /bin/sh misfadmin \
    && adduser -D -s /bin/sh misfdeploy \
    && printf '%s\n' 'root:misf-root-pw' 'misfadmin:misf-admin-pw' 'misfdeploy:misf-deploy-pw' | chpasswd \
    && printf '%s\n' 'misfadmin ALL=(ALL:ALL) ALL' 'misfdeploy ALL=(ALL:ALL) NOPASSWD: ALL' > /etc/sudoers.d/misf \
    && chmod 0440 /etc/sudoers.d/misf \
    && visudo -c -q

# MISF verifies Ed25519 and ECDSA host keys only.
RUN rm -f /etc/ssh/ssh_host_*_key /etc/ssh/ssh_host_*_key.pub \
    && ssh-keygen -q -t ed25519 -N '' -f /etc/ssh/ssh_host_ed25519_key \
    && mkdir -p /etc/ssh/sshd_config.d \
    && printf '%s\n' \
        'HostKey /etc/ssh/ssh_host_ed25519_key' \
        'PermitRootLogin yes' \
        'PasswordAuthentication yes' \
        'KbdInteractiveAuthentication no' \
        'UseDNS no' \
        > /etc/ssh/sshd_config.d/00-misf.conf \
    && grep -q '^Include /etc/ssh/sshd_config.d' /etc/ssh/sshd_config \
    || sed -i '1i Include /etc/ssh/sshd_config.d/*.conf' /etc/ssh/sshd_config

# abuild refuses to run as root; its key signs the repository index, and apk trusts it.
RUN adduser -D -G abuild builder \
    && su builder -s /bin/sh -c 'abuild-keygen -a -n -q' \
    && mkdir -p /opt/misf \
    && cp /home/builder/.abuild/*.rsa /opt/misf/sign.rsa \
    && cp /home/builder/.abuild/*.rsa.pub /etc/apk/keys/ \
    && cp /home/builder/.abuild/*.rsa.pub /opt/misf/

COPY build-packages.sh misf-scenario /usr/local/bin/
RUN chmod 0755 /usr/local/bin/build-packages.sh /usr/local/bin/misf-scenario \
    && build-packages.sh \
    # Version 1.0 goes in from a repository, so apk upgrades it like any other package
    # rather than keeping the exact file it was given.
    && misf-scenario --v1 \
    && echo /opt/misf/repo > /etc/apk/repositories \
    && apk update -q \
    && apk add -q $(cat /opt/misf/names) \
    && echo 'setting = changed-by-the-admin' > /etc/misf-conf.conf \
    # Only the local repository from here on: the tests never need the network.
    && misf-scenario \
    && rm -rf /var/cache/apk/*

EXPOSE 22
CMD ["/usr/sbin/sshd", "-D", "-e"]
