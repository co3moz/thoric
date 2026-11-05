#!/usr/bin/env bash
set -euo pipefail
install -d -m 0700 -o desktop -g desktop /run/desktop
if [[ ${DESKTOP_SESSION:-xfce} == gnome ]]; then
    install -d /run/dbus
    dbus-daemon --system --fork --nopidfile
fi
if [[ ${DESKTOP_SUDO:-true} == true ]]; then
    printf 'desktop ALL=(ALL) NOPASSWD:ALL\n' > /etc/sudoers.d/desktop
    chmod 0440 /etc/sudoers.d/desktop
else
    rm -f /etc/sudoers.d/desktop
fi
# Thoric gives a container's proxy as these variables, and a command run with sudo, such as
# apt, goes through the proxy too.
printf 'Defaults env_keep += "HTTP_PROXY HTTPS_PROXY NO_PROXY http_proxy https_proxy no_proxy"\n' \
    > /etc/sudoers.d/thoric-proxy
chmod 0440 /etc/sudoers.d/thoric-proxy
# Devices passed from the host keep its group IDs, and runuser gives the desktop user only the
# groups /etc/group names, so it joins a group with the device's ID, made under `prefix` when
# the image has none.
join_device_group() {
    local node=$1 prefix=$2 gid group
    [[ -c $node ]] || return 0
    gid=$(stat -c %g "$node")
    [[ $gid != 0 ]] || return 0
    getent group "$gid" >/dev/null || groupadd --gid "$gid" "$prefix$gid" 2>/dev/null || true
    group=$(getent group "$gid" | cut -d: -f1) || group=
    if [[ -n $group ]]; then usermod -aG "$group" desktop; fi
}
# A container with the kvm permission has the host's virtualization device.
join_device_group /dev/kvm kvm
if [[ ${DESKTOP_GPU:-false} == true ]]; then
    if [[ -e /dev/dxg ]]; then
        export LD_LIBRARY_PATH="/usr/lib/wsl/lib:${LD_LIBRARY_PATH:-}"
        export MESA_D3D12_DEFAULT_ADAPTER_NAME="${GPU_ADAPTER:-}"
    fi
    unset LIBGL_ALWAYS_SOFTWARE
    for node in /dev/dri/*; do
        join_device_group "$node" dri
    done
else
    export LIBGL_ALWAYS_SOFTWARE=1
fi
# Refresh runtime settings on every start, including starts from committed images.
printf 'export DISPLAY=:0\nexport PULSE_SERVER=unix:/run/desktop/pulse/native\n' > /etc/profile.d/desktop.sh
if [[ ${DESKTOP_GPU:-false} == true && -e /dev/dxg ]]; then
    # shellcheck disable=SC2016 # the profile expands it in each login shell
    printf 'export LD_LIBRARY_PATH=/usr/lib/wsl/lib:${LD_LIBRARY_PATH:-}\n' >> /etc/profile.d/desktop.sh
fi
exec runuser -u desktop -- /usr/local/bin/desktop-session

