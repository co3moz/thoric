# Third-party components

Thoric uses the components below to provide its browser desktop, GPU rendering and Ubuntu environment. Thoric's own source is covered by the [GNU General Public License, version 3 or later](LICENSE); these components keep their upstream licenses, each compatible with it.

| Component | License information |
| --- | --- |
| [noVNC](https://github.com/novnc/noVNC) 1.7.0 | MPL-2.0. The generated frontend includes a notice, and the full npm license is served at `/novnc-LICENSE.txt`. The license of the pako code noVNC vendors (MIT) is in `/third-party-licenses.txt`. Unmodified sources are available from the pinned npm package and repository tag; the source map also embeds the bundled source files. |
| [Oxanium](https://github.com/sevmeyer/oxanium) and [Geist Mono](https://github.com/vercel/geist-font) | SIL Open Font License 1.1. The interface bundles the variable fonts from the pinned Fontsource packages; their licenses are served at `/oxanium-LICENSE.txt` and `/geist-mono-LICENSE.txt`. |
| [Remix Icon](https://remixicon.com) 4.8.0 | Apache-2.0. This project uses icons from Remix Icon through `@remixicon/react` 4.8.0, the last release under the Apache License; later releases are under the Remix Icon License v1.0, whose restrictions GPL-3.0-or-later cannot carry. The package ships no license file, so the build adds the Apache License to `/third-party-licenses.txt`. The icons are interface elements only; the Thoric mark is not derived from them. |
| [Monaco Editor](https://github.com/microsoft/monaco-editor) 0.57.0 | MIT. The code editor of scripts, the build configuration, files and the MCP inspector's JSON, loaded the first time one shows. The notices of the code it bundles from other projects are served at `/monaco-third-party-notices.txt`; its own license is in `/third-party-licenses.txt`. |
| Frontend libraries such as React, Radix UI, shadcn/ui, TanStack Query and Table, Recharts, Sonner and Vaul | MIT, ISC, Apache-2.0 (class-variance-authority) or 0BSD (tslib). The build collects the license of every package in the bundle into `/third-party-licenses.txt`; `bun.lock` records the exact versions. |
| [VirtualGL](https://github.com/VirtualGL/virtualgl) 3.1.5 | Upstream licenses. The Debian package installs its license files under `/opt/VirtualGL`. |
| TigerVNC, XFCE, GNOME, Mesa, PulseAudio and other Ubuntu packages | Each package retains its own license. The desktop images include package documentation under `/usr/share/doc`. Recordings are encoded by GStreamer (LGPL-2.1-or-later) with its `x264enc` plugin from `gstreamer1.0-plugins-ugly`, which links x264 (GPL-2.0-or-later). |
| Rust dependencies | Each dependency retains its own license. `Cargo.lock` records the exact versions used. |
| [Docker CLI and Buildx](https://github.com/docker/cli) | Apache-2.0, Copyright Docker, Inc. The manager image copies the `docker` and `docker-buildx` binaries from a Docker 28 `docker:<version>-cli` image, the release `core/manager/Dockerfile` names. |
| [Tesseract OCR](https://github.com/tesseract-ocr/tesseract) | Apache-2.0. The manager image installs Debian's `tesseract-ocr` with its English data, `tesseract-ocr-eng`, to read the text on desktops' screens for `find_text` and `wait_for_condition`. Each package keeps its license and copyright under `/usr/share/doc` in the image. |
| [Moby](https://github.com/moby/moby) seccomp profile | Apache-2.0. `images/desktop/seccomp.json` is Docker's default seccomp profile with one rule added, so browser sandboxes can create user namespaces. |
| [Rust MCP SDK (rmcp)](https://github.com/modelcontextprotocol/rust-sdk) 3.9.0 | Apache-2.0. Provides the embedded MCP protocol and HTTP transport. |

The dashboard's **Dependencies** page lists every interface package, crate, image, system package, tool and workflow action a release is built from, with the license each package names; the manager image's build makes that list and refuses to finish when a package that ships names none.
