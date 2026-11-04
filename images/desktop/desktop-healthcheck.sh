#!/usr/bin/env bash
# The desktop's health: its session has started, its X display answers within three seconds, below
# Docker's limit of four, and a GNOME session has not fallen back to its failure screen. A failure
# prints one line that begins with what failed, which the manager shows from Docker's log of the
# latest checks: desktop-starting, display-held, display-gone or session-failed.
set -uo pipefail
if [[ ! -f /run/desktop/ready ]]; then
    echo 'desktop-starting: The desktop session has not finished starting'
    exit 1
fi
# A program can hold the display with a server grab, as ImageMagick's import does while it waits
# for a window to be clicked; xdpyinfo then waits until timeout ends it.
runuser -u desktop -- timeout 3 xdpyinfo -display :0 >/dev/null 2>&1
status=$?
if ((status == 124)); then
    echo 'display-held: The X display did not answer within 3 seconds, so a program may hold it'
    exit 1
elif ((status != 0)); then
    echo 'display-gone: The X display is not running'
    exit 1
fi
if [[ ${DESKTOP_SESSION:-xfce} == gnome ]]; then
    if ! pgrep -u desktop -x gnome-shell >/dev/null; then
        echo 'session-failed: GNOME Shell is not running'
        exit 1
    fi
    if pgrep -u desktop -f '^/usr/libexec/gnome-session-failed' >/dev/null; then
        echo 'session-failed: GNOME session failure screen detected'
        exit 1
    fi
fi
