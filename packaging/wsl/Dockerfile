# Root filesystem for the Thoric WSL distribution: Ubuntu 24.04 with Docker Engine, Compose,
# Buildx, the NVIDIA Container Toolkit and the Thoric installation files. The image is
# exported with `packaging/wsl/build.sh` into a `.wsl` package that `wsl --install --from-file`
# accepts. The base is a dated build of Ubuntu 24.04, which Dependabot raises. It is built for
# linux/amd64 and for linux/arm64, Windows on ARM, and installs the same packages on both: Docker's
# and NVIDIA's repositories publish each of them for both architectures, so apt takes the build's own.
FROM ubuntu:noble-20261024.1
ARG DEBIAN_FRONTEND=noninteractive
RUN apt-get update && apt-get install -y --no-install-recommends \
    ca-certificates curl gnupg sudo systemd systemd-sysv dbus iproute2 iptables \
    openssl less nano locales tzdata dbus-user-session libpam-systemd bash-completion \
    && locale-gen en_US.UTF-8 && rm -rf /var/lib/apt/lists/*
# Docker Engine stays within one major release, here and in every 'thoric update' after; the
# directory exists in the base image.
COPY --chmod=0644 packaging/wsl/thoric-docker.pref /etc/apt/preferences.d/thoric-docker.pref
RUN install -m 0755 -d /etc/apt/keyrings \
    && curl -fsSL https://download.docker.com/linux/ubuntu/gpg -o /etc/apt/keyrings/docker.asc \
    && chmod a+r /etc/apt/keyrings/docker.asc \
    && echo "deb [arch=$(dpkg --print-architecture) signed-by=/etc/apt/keyrings/docker.asc] https://download.docker.com/linux/ubuntu noble stable" > /etc/apt/sources.list.d/docker.list \
    && curl -fsSL https://nvidia.github.io/libnvidia-container/gpgkey | gpg --dearmor -o /usr/share/keyrings/nvidia-container-toolkit-keyring.gpg \
    && curl -fsSL https://nvidia.github.io/libnvidia-container/stable/deb/nvidia-container-toolkit.list \
       | sed 's#deb https://#deb [signed-by=/usr/share/keyrings/nvidia-container-toolkit-keyring.gpg] https://#g' \
       > /etc/apt/sources.list.d/nvidia-container-toolkit.list \
    && apt-get update && apt-get install -y --no-install-recommends \
       docker-ce docker-ce-cli containerd.io docker-buildx-plugin docker-compose-plugin nvidia-container-toolkit \
    && rm -rf /var/lib/apt/lists/*
# Docker starts with systemd; daemon.json, copied below, registers the NVIDIA runtime now, so the
# first boot needs no network, turns on live-restore, so containers keep running when Docker
# restarts, and has every container log through json-file in at most three files of 10 MB. An
# update brings it, like the package's other system files, unless it was edited by hand. WSL 2
# runs every distribution in one network namespace. nftables.service flushes the
# whole ruleset when it starts (the default /etc/nftables.conf) and when it stops, which clears
# the firewall rules of every Docker Engine on the machine, so it is masked.
RUN systemctl enable docker containerd \
    && systemctl mask systemd-firstboot.service kmod-static-nodes.service nftables.service \
    && userdel -r ubuntu \
    && printf '%%sudo ALL=(ALL) NOPASSWD:ALL\n' > /etc/sudoers.d/thoric \
    && chmod 0440 /etc/sudoers.d/thoric \
    && install -d -m 0755 /opt/thoric /opt/thoric/deploy /opt/thoric/update /usr/lib/wsl /usr/share/doc/thoric /etc/docker
# Every COPY names its mode, since a checkout on a Windows drive makes each file 0777. --chmod also
# applies to a directory the COPY creates, so each destination directory exists beforehand.
COPY --chmod=0644 packaging/wsl/daemon.json /etc/docker/daemon.json
COPY --chmod=0644 compose.ghcr.yml /opt/thoric/compose.yml
COPY --chmod=0644 .env.example /opt/thoric/.env.example
COPY --chmod=0644 deploy/Caddyfile /opt/thoric/deploy/Caddyfile
COPY --chmod=0644 packaging/wsl/wsl.conf /etc/wsl.conf
COPY --chmod=0644 packaging/wsl/wsl-distribution.conf /etc/wsl-distribution.conf
COPY --chmod=0755 packaging/wsl/oobe.sh /usr/lib/wsl/oobe.sh
COPY --chmod=0755 packaging/wsl/keepalive.sh /usr/lib/wsl/keepalive.sh
COPY --chmod=0755 packaging/wsl/firewall-watch.sh /usr/lib/wsl/firewall-watch.sh
COPY --chmod=0644 packaging/wsl/thoric-firewall.service /etc/systemd/system/thoric-firewall.service
COPY --chmod=0644 packaging/wsl/thoric-update.path /etc/systemd/system/thoric-update.path
COPY --chmod=0644 packaging/wsl/thoric-update.service /etc/systemd/system/thoric-update.service
COPY --chmod=0644 packaging/wsl/thoric-settings.path /etc/systemd/system/thoric-settings.path
COPY --chmod=0644 packaging/wsl/thoric-settings.service /etc/systemd/system/thoric-settings.service
COPY --chmod=0755 packaging/wsl/thoric /usr/local/bin/thoric
COPY --chmod=0644 packaging/wsl/thoric-completion.bash /usr/share/bash-completion/completions/thoric
# thor is a short name for the command, with the same completion. Both are links by name, so the
# command an update installs answers to thor as well.
RUN ln -s thoric /usr/local/bin/thor \
    && ln -s thoric /usr/share/bash-completion/completions/thor
COPY --chmod=0644 LICENSE /usr/share/doc/thoric/LICENSE
# A package installs the manager release it was built for, however many newer ones exist by then;
# `thoric update` moves an installation forward. The first start copies this file to .env.
# The checksums let an update tell installation files edited by hand from the ones shipped here, and
# the command records its release, so an update installs the command of a newer one only.
ARG THORIC_VERSION
RUN test -n "$THORIC_VERSION" \
    && sed -i "s|^MANAGER_IMAGE=.*|MANAGER_IMAGE=ghcr.io/co3moz/thoric:$THORIC_VERSION|" /opt/thoric/.env.example \
    && grep -qFx "MANAGER_IMAGE=ghcr.io/co3moz/thoric:$THORIC_VERSION" /opt/thoric/.env.example \
    && sed -i "s|^CLI_VERSION=.*|CLI_VERSION=$THORIC_VERSION|" /usr/local/bin/thoric \
    && grep -qFx "CLI_VERSION=$THORIC_VERSION" /usr/local/bin/thoric \
    && cd /opt/thoric && sha256sum compose.yml deploy/Caddyfile .env.example > .shipped.sha256
# The package level counts changes to the package's own files, and marks the distribution as
# Thoric's package, whose system an update keeps current. An update brings the system files listed
# in system.sha256 (SYSTEM_FILES in the command) and raises the level to the command's SYSTEM_LEVEL,
# which equals this one; the checksums tell it a file edited by hand. An update installs the
# command of a newer release only when its NEEDS_PACKAGE is at most the level, for what only a new
# package brings. The update and settings units rely on the command, not the other way round, and
# an update installs them where they are missing. 'watched' and 'settings-watched' tell the
# manager that the path units take its update requests and its settings changes.
RUN dockerd --validate --config-file /etc/docker/daemon.json \
    && systemctl enable thoric-firewall.service thoric-update.path thoric-settings.path \
    && touch /opt/thoric/update/watched /opt/thoric/update/settings-watched \
    && printf '2\n' > /usr/lib/wsl/package-level \
    && sha256sum /etc/apt/preferences.d/thoric-docker.pref /etc/docker/daemon.json /etc/systemd/system/thoric-firewall.service /usr/lib/wsl/firewall-watch.sh /usr/lib/wsl/keepalive.sh /usr/share/bash-completion/completions/thoric > /usr/lib/wsl/system.sha256 \
    && rm -f /etc/machine-id /var/lib/dbus/machine-id
