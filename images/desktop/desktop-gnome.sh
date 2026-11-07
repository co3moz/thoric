#!/usr/bin/env bash
set -euo pipefail
export XDG_SESSION_TYPE=x11 XDG_CURRENT_DESKTOP=ubuntu:GNOME
export XDG_SESSION_DESKTOP=ubuntu GNOME_SHELL_SESSION_MODE=ubuntu
mkdir -p "$HOME/.config/autostart"
for component in Power UsbProtection Sharing Smartcard Rfkill; do
    printf '[Desktop Entry]\nType=Application\nHidden=true\n' > "$HOME/.config/autostart/org.gnome.SettingsDaemon.${component}.desktop"
done
# The shared session supervisor owns PulseAudio (including the explicitly disabled case).
printf '[Desktop Entry]\nType=Application\nHidden=true\n' > "$HOME/.config/autostart/pulseaudio.desktop"
# Desktop locking/logind integration is unavailable in this container session.
gsettings set org.gnome.desktop.screensaver lock-enabled false
gsettings set org.gnome.desktop.session idle-delay 0
# GNOME's proxy settings, which browsers read here in place of the variables, follow the proxy
# Thoric gives a container, and go back to none in a container without one that came from a
# commit of one that had it.
if [[ -n ${HTTP_PROXY:-} ]]; then
    address=${HTTP_PROXY#http://}
    address=${address%/}
    gsettings set org.gnome.system.proxy mode manual
    for scheme in http https; do
        gsettings set "org.gnome.system.proxy.$scheme" host "${address%:*}"
        gsettings set "org.gnome.system.proxy.$scheme" port "${address##*:}"
    done
    gsettings set org.gnome.system.proxy ignore-hosts "['localhost', '127.0.0.0/8', '::1']"
elif [[ $(gsettings get org.gnome.system.proxy.http host) == "'127.0.0.1'" &&
    $(gsettings get org.gnome.system.proxy.http port) == 31280 ]]; then
    gsettings set org.gnome.system.proxy mode none
fi
if [[ ! -f "$HOME/.config/thoric-gnome-initialized" ]]; then
    gsettings set org.gnome.desktop.interface gtk-theme Yaru
    gsettings set org.gnome.desktop.interface icon-theme Yaru
    gsettings set org.gnome.shell enabled-extensions "['ubuntu-dock@ubuntu.com', 'ubuntu-appindicators@ubuntu.com']"
    touch "$HOME/.config/thoric-gnome-initialized"
fi
exec gnome-session --session=thoric-ubuntu --disable-acceleration-check
