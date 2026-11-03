# Rehearsal server for `cargo xtask setup --target docker`: a blank
# Ubuntu with systemd as PID 1, sshd and sudo, nothing of the handbook.
# `xtask provision` does the rest over SSH, exactly as on production.
FROM ubuntu:26.04

ENV container=docker DEBIAN_FRONTEND=noninteractive

RUN apt-get update \
 && apt-get install -y --no-install-recommends \
      systemd systemd-sysv dbus openssh-server sudo ca-certificates \
      curl sqlite3 procps iproute2 \
 && rm -rf /var/lib/apt/lists/* \
 && systemctl enable ssh \
 && systemctl mask getty.target console-getty.service systemd-logind.service \
      systemd-udevd.service systemd-udev-trigger.service systemd-firstboot.service \
 && sed -i 's/^#\?PasswordAuthentication .*/PasswordAuthentication no/; s/^#\?PermitRootLogin .*/PermitRootLogin prohibit-password/' /etc/ssh/sshd_config \
 && mkdir -p /root/.ssh && chmod 700 /root/.ssh

STOPSIGNAL SIGRTMIN+3
CMD ["/sbin/init"]
