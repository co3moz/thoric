# Ubuntu 24.04 of a dated build, which Dependabot raises; tests/test_pinned_versions.py refuses a
# moving tag. Both stages from Ubuntu name the same build.
FROM ubuntu:noble-20261024.1 AS common
ARG DEBIAN_FRONTEND=noninteractive
ARG VIRTUALGL_VERSION=3.1.5
RUN apt-get update && apt-get install -y --no-install-recommends \
    dbus-x11 tigervnc-standalone-server tigervnc-common \
    x11-utils x11-xserver-utils x11-apps xdotool wmctrl imagemagick sudo tini curl ca-certificates python3 \
    at-spi2-core python3-pyatspi \
    pulseaudio pulseaudio-utils mesa-utils mesa-utils-extra libgl1-mesa-dri libegl1 \
    mesa-vulkan-drivers vulkan-tools fonts-dejavu fonts-noto-color-emoji fonts-noto-cjk locales \
    && locale-gen en_US.UTF-8 && rm -rf /var/lib/apt/lists/*
# VirtualGL publishes a package for each architecture the images are built for, amd64 and arm64.
RUN curl -fL --retry 3 "https://github.com/VirtualGL/virtualgl/releases/download/${VIRTUALGL_VERSION}/virtualgl_${VIRTUALGL_VERSION}_$(dpkg --print-architecture).deb" -o /tmp/virtualgl.deb \
    && apt-get update && apt-get install -y /tmp/virtualgl.deb && rm /tmp/virtualgl.deb && rm -rf /var/lib/apt/lists/*
RUN useradd --create-home --shell /bin/bash --uid 1001 desktop \
    && mkdir -p /shared /workspace && chown desktop:desktop /shared /workspace \
    && mkdir -p /usr/local/share/applications
LABEL thoric.desktop=true
# desktop-session listens for VNC only in the private network, so the members of a container's
# groups do not reach the screen; the manager shows containers from images without this.
LABEL thoric.vnc=private
ENV LANG=en_US.UTF-8 DISPLAY=:0 DESKTOP_RESOLUTION=1920x1080 \
    DESKTOP_AUDIO=false DESKTOP_MICROPHONE=false DESKTOP_GPU=false DESKTOP_SUDO=true \
    PULSE_SERVER=unix:/run/desktop/pulse/native
EXPOSE 5900
HEALTHCHECK --interval=10s --timeout=4s --start-period=30s CMD /usr/local/bin/desktop-healthcheck
ENTRYPOINT ["/usr/bin/tini", "-g", "--", "/usr/local/bin/desktop-entrypoint"]

# VirtualGL's Chrome workaround for Chromium-family processes only, preloaded by gpu-run.
FROM ubuntu:noble-20261024.1 AS chromium-gpu
RUN apt-get update && apt-get install -y --no-install-recommends gcc libc6-dev \
    && rm -rf /var/lib/apt/lists/*
COPY chromium-gpu.c /src/
RUN mkdir -p /out /tmp/chromium \
    && gcc -O2 -Wall -Wextra -Werror -shared -fPIC -o /out/chromium-gpu.so /src/chromium-gpu.c \
    && cp /usr/bin/env /tmp/chromium/chrome && touch /tmp/chromium/icudtl.dat /tmp/chromium/resources.pak \
    && export LD_PRELOAD=/out/chromium-gpu.so \
    && /tmp/chromium/chrome | grep -qx VGL_CHROMEHACK=1 \
    && VGL_CHROMEHACK=0 /tmp/chromium/chrome | grep -qx VGL_CHROMEHACK=0 \
    && ! /tmp/chromium/chrome /usr/bin/env | grep -q VGL_CHROMEHACK \
    && ! env | grep -q VGL_CHROMEHACK

FROM common AS scripts
COPY desktop-entrypoint.sh /usr/local/bin/desktop-entrypoint
COPY desktop-session.sh /usr/local/bin/desktop-session
COPY desktop-screenshot.py /usr/local/bin/desktop-screenshot
COPY desktop-diagnostics.sh /usr/local/bin/desktop-diagnostics
COPY gpu-run.sh /usr/local/bin/gpu-run
COPY desktop-gnome.sh /usr/local/bin/desktop-gnome
COPY desktop-healthcheck.sh /usr/local/bin/desktop-healthcheck
COPY desktop-private-address.py /usr/local/bin/desktop-private-address
COPY thoric-mcp.py /usr/local/bin/thoric-mcp
COPY thoric-mcp-tray.py /usr/local/bin/thoric-mcp-tray
# The tray of both sessions: its menu entry, icons and autostart entry.
COPY mcp-tray/thoric-mcp.desktop /usr/local/share/applications/
COPY mcp-tray/icons/ /usr/local/share/icons/hicolor/
COPY mcp-tray/thoric-mcp-tray.desktop /etc/xdg/autostart/
# What the Thoric programs above are licensed under, beside every Ubuntu package's copyright.
COPY copyright /usr/share/doc/thoric/copyright
# NVIDIA's EGL library for GPU desktops on a Linux host. The NVIDIA Container Toolkit's hook adds
# the library without this file, and glvnd skips the entry where the library is missing, as in WSL.
# The final stages copy it after their packages, so a change here keeps those layers cached.
RUN printf '{"file_format_version":"1.0.0","ICD":{"library_path":"libEGL_nvidia.so.0"}}\n' \
    > /usr/share/glvnd/egl_vendor.d/10_nvidia.json
RUN chmod +x /usr/local/bin/desktop-* /usr/local/bin/gpu-run /usr/local/bin/thoric-mcp \
        /usr/local/bin/thoric-mcp-tray \
    && chmod -R u=rwX,go=rX /usr/local/share/icons/hicolor \
    && chmod 644 /usr/local/share/applications/thoric-mcp.desktop \
        /etc/xdg/autostart/thoric-mcp-tray.desktop /usr/share/doc/thoric/copyright

FROM common AS xfce-packages
# librsvg2-common is GdkPixbuf's SVG loader; without it XFCE's default wallpaper and every icon
# that exists only as SVG stay blank. The GNOME stage installs it too. PyGObject and Ayatana's
# AppIndicator run the MCP servers tray, which XFCE's Status Tray and GNOME's AppIndicator
# extension show alike. GStreamer's plugins with its bindings for PyGObject turn recordings into
# H.264 MP4 videos (appsrc, jpegdec, x264enc from the ugly set, mp4mux); GNOME installs them too.
RUN apt-get update && apt-get install -y --no-install-recommends xfce4 xfce4-terminal librsvg2-common \
    python3-gi gir1.2-gtk-3.0 gir1.2-ayatanaappindicator3-0.1 \
    gir1.2-gstreamer-1.0 gstreamer1.0-plugins-base gstreamer1.0-plugins-good gstreamer1.0-plugins-ugly \
    && rm -rf /var/lib/apt/lists/*
FROM xfce-packages AS xfce
COPY --from=scripts /usr/local/bin/ /usr/local/bin/
COPY --from=scripts /usr/local/share/applications/thoric-mcp.desktop /usr/local/share/applications/
COPY --from=scripts /usr/local/share/icons/ /usr/local/share/icons/
COPY --from=scripts /etc/xdg/autostart/thoric-mcp-tray.desktop /etc/xdg/autostart/
COPY --from=scripts /usr/share/doc/thoric/ /usr/share/doc/thoric/
COPY --from=scripts /usr/share/glvnd/egl_vendor.d/10_nvidia.json /usr/share/glvnd/egl_vendor.d/
COPY --from=chromium-gpu /out/chromium-gpu.so /usr/local/lib/thoric/chromium-gpu.so
COPY gpu-terminal.desktop /usr/share/applications/gpu-terminal.desktop
ENV DESKTOP_SESSION=xfce
# thoric.desktop-level, the same in both images, is DESKTOP_LEVEL in core/manager/src/catalog.rs.
# Raise both together when this image changes in a way an older manager cannot handle, such as a
# script, path, user or label it relies on going away or changing its form, so an older manager
# refuses the image, and anything built from it, instead of failing inside it. Images built,
# committed or imported from this one keep the label.
LABEL thoric.desktop-level=1
LABEL thoric.profile=ubuntu-xfce thoric.session=xfce \
    org.opencontainers.image.title="Ubuntu 24.04 · Lightweight XFCE" \
    org.opencontainers.image.description="Ubuntu LTS with a lightweight XFCE desktop, independent VNC display and optional GPU/audio" \
    org.opencontainers.image.version="24.04"

FROM common AS gnome-packages
RUN apt-get update && apt-get install -y --no-install-recommends \
    ubuntu-session gnome-session gnome-shell gnome-settings-daemon gnome-control-center \
    gnome-terminal nautilus gnome-shell-extension-ubuntu-dock gnome-shell-extension-appindicator \
    yaru-theme-gtk yaru-theme-icon yaru-theme-gnome-shell gsettings-ubuntu-schemas \
    gnome-themes-extra dbus-user-session at-spi2-core \
    && rm -rf /var/lib/apt/lists/*
RUN apt-get update && apt-get install -y --no-install-recommends ubuntu-wallpapers fonts-ubuntu \
    librsvg2-common gir1.2-dbusmenu-glib-0.4 ibus \
    python3-gi gir1.2-gtk-3.0 gir1.2-ayatanaappindicator3-0.1 \
    gir1.2-gstreamer-1.0 gstreamer1.0-plugins-base gstreamer1.0-plugins-good gstreamer1.0-plugins-ugly \
    && rm -rf /var/lib/apt/lists/*
FROM gnome-packages AS gnome
COPY --from=scripts /usr/local/bin/ /usr/local/bin/
COPY --from=scripts /usr/local/share/applications/thoric-mcp.desktop /usr/local/share/applications/
COPY --from=scripts /usr/local/share/icons/ /usr/local/share/icons/
COPY --from=scripts /etc/xdg/autostart/thoric-mcp-tray.desktop /etc/xdg/autostart/
COPY --from=scripts /usr/share/doc/thoric/ /usr/share/doc/thoric/
COPY --from=scripts /usr/share/glvnd/egl_vendor.d/10_nvidia.json /usr/share/glvnd/egl_vendor.d/
COPY --from=chromium-gpu /out/chromium-gpu.so /usr/local/lib/thoric/chromium-gpu.so
COPY gnome-container.session /usr/share/gnome-session/sessions/thoric-ubuntu.session
ENV DESKTOP_SESSION=gnome
LABEL thoric.desktop-level=1
LABEL thoric.profile=ubuntu-gnome thoric.session=gnome \
    org.opencontainers.image.title="Ubuntu 24.04 · GNOME" \
    org.opencontainers.image.description="Ubuntu GNOME Shell with Ubuntu session and Yaru appearance, independent VNC display and optional GPU/audio" \
    org.opencontainers.image.version="24.04"

# An unqualified docker build retains the lightweight default.
FROM xfce AS default

