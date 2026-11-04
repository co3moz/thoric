# Every base image names an exact version, which Dependabot raises, and tests/test_pinned_versions.py
# refuses a moving tag. The ui stage builds with the Bun the checks install, the chef stage with the
# Rust release of rust-toolchain.toml.
FROM oven/bun:1.4.4-slim AS ui
WORKDIR /build
COPY THIRD_PARTY_NOTICES.md LICENSE ./
WORKDIR /build/core/web
COPY core/web/package.json core/web/bun.lock core/web/bunfig.toml ./
RUN bun install --frozen-lockfile
COPY core/web .
RUN bun run build

FROM rust:1.99.1-bookworm AS chef
RUN cargo install cargo-chef --locked --version 0.1.78
WORKDIR /build

# The recipe names only the dependencies, with the crate's own version masked, so a source change
# or a version bump leaves it as it was and the dependencies come from the build cache.
FROM chef AS planner
COPY Cargo.toml Cargo.lock ./
COPY core/manager/Cargo.toml core/manager/Cargo.toml
COPY core/manager/src core/manager/src
RUN cargo chef prepare --recipe-path recipe.json

FROM chef AS cooked
COPY --from=planner /build/recipe.json recipe.json
RUN cargo chef cook --release --locked --recipe-path recipe.json

FROM cooked AS backend
COPY Cargo.toml Cargo.lock ./
COPY core/manager/Cargo.toml core/manager/Cargo.toml
COPY core/manager/src core/manager/src
COPY core/manager/helpers core/manager/helpers
COPY images/desktop/profiles.json images/desktop/profiles.json
# The publishing workflow names the tag of the desktop images it publishes with this manager, the
# version for a release and sha-<commit> for a build of master, and the manager then downloads
# those. A build without it, such as one from the source, downloads each catalog image's reference.
ARG THORIC_RELEASE
# Only the binary is left in this layer, so the build cache keeps one copy of the compiled
# dependencies, the one above, instead of one with every commit.
RUN cargo build --release --locked && mv target/release/thoric /thoric && rm -rf target

# The list of the Dependencies page, from this build's lockfiles, installed interface packages,
# Dockerfiles, Compose manifests and workflows, so it cannot fall behind them. It fails when a
# package that ships names no license. `cargo metadata` reads the crates the cooked stage holds and
# needs only the binary's entry point; the interface stage is mounted, not copied. Each platform's
# image lists the packages and crates of its own, which BuildKit names in TARGETPLATFORM.
FROM cooked AS dependencies
ARG TARGETPLATFORM
COPY Cargo.toml Cargo.lock compose.yml compose.ghcr.yml ./
COPY core/manager/Cargo.toml core/manager/Dockerfile core/manager/dependencies.py core/manager/
COPY core/manager/src/main.rs core/manager/src/main.rs
COPY images/desktop/Dockerfile images/desktop/Dockerfile
COPY packaging/wsl/Dockerfile packaging/wsl/Dockerfile
COPY .github/workflows .github/workflows
RUN --mount=type=bind,from=ui,source=/build/core/web,target=/build/core/web \
    python3 core/manager/dependencies.py --strict --platform "${TARGETPLATFORM:-linux/amd64}" --output /dependencies.json

FROM docker:28.5.2-cli AS dockercli
# The Debian release the rust image above is built on, so the manager finds the C library it was
# linked against.
FROM debian:bookworm-20261027-slim
LABEL org.opencontainers.image.title="Thoric" org.opencontainers.image.licenses="GPL-3.0-or-later"
# Tesseract reads the text on desktops' screens for find_text and wait_for_condition. It runs in
# the manager rather than in the desktops, so it reads every desktop image, older ones too.
RUN apt-get update && apt-get install -y --no-install-recommends ca-certificates curl tini python3 \
    tesseract-ocr tesseract-ocr-eng && rm -rf /var/lib/apt/lists/*
COPY --from=dockercli /usr/local/bin/docker /usr/local/bin/docker
COPY --from=dockercli /usr/local/libexec/docker/cli-plugins/docker-buildx /usr/local/libexec/docker/cli-plugins/docker-buildx
COPY --from=backend /thoric /usr/local/bin/thoric
COPY --from=ui /build/core/web/dist /app/web
COPY images/desktop /app/desktop
COPY core/bridge/mcp_bridge.py /app/mcp_bridge.py
COPY compose.ghcr.yml /app/install/compose.yml
COPY .env.example /app/install/.env.example
COPY deploy/Caddyfile /app/install/deploy/Caddyfile
COPY packaging/wsl/thoric /app/install/thoric
COPY packaging/wsl/thoric-update.path /app/install/systemd/thoric-update.path
COPY packaging/wsl/thoric-update.service /app/install/systemd/thoric-update.service
COPY packaging/wsl/thoric-settings.path /app/install/systemd/thoric-settings.path
COPY packaging/wsl/thoric-settings.service /app/install/systemd/thoric-settings.service
# The WSL package's system files at the paths it has them, which `thoric update` brings to
# installations of older packages (SYSTEM_FILES in packaging/wsl/thoric).
COPY packaging/wsl/thoric-docker.pref /app/install/system/etc/apt/preferences.d/thoric-docker.pref
COPY packaging/wsl/daemon.json /app/install/system/etc/docker/daemon.json
COPY packaging/wsl/thoric-firewall.service /app/install/system/etc/systemd/system/thoric-firewall.service
COPY packaging/wsl/firewall-watch.sh /app/install/system/usr/lib/wsl/firewall-watch.sh
COPY packaging/wsl/keepalive.sh /app/install/system/usr/lib/wsl/keepalive.sh
COPY packaging/wsl/thoric-completion.bash /app/install/system/usr/share/bash-completion/completions/thoric
COPY LICENSE /usr/share/doc/thoric/LICENSE
COPY --from=dependencies /dependencies.json /usr/share/doc/thoric/dependencies.json
ENV DATA_DIR=/data WEB_DIR=/app/web DESKTOP_CONTEXT=/app/desktop LISTEN_ADDR=0.0.0.0:8080
WORKDIR /app
EXPOSE 8080
HEALTHCHECK --interval=20s --timeout=3s CMD curl -fsS http://127.0.0.1:8080/healthz || exit 1
ENTRYPOINT ["/usr/bin/tini", "--", "/usr/local/bin/thoric"]
# The same tag as a label, which `thoric update` in the WSL package reads to download the desktop
# images this manager would. Declared last so it leaves the cache of the layers above alone.
ARG THORIC_RELEASE
LABEL thoric.release="${THORIC_RELEASE}"

