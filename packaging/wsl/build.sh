#!/usr/bin/env bash
# Build the Thoric WSL distribution package from packaging/wsl/Dockerfile.
# Usage: packaging/wsl/build.sh [--platform linux/amd64|linux/arm64] [output.wsl]
# (set WSL_COMPRESS=gzip for a faster, larger build)
# The package installs the manager release THORIC_VERSION, by default the version in
# core/manager/Cargo.toml. It is for the platform of the Docker host unless --platform names the
# other one; linux/arm64 is the package for Windows on ARM. Without an output it is written under
# the name of its release asset, which the Windows helper downloads: dist/thoric.wsl for
# linux/amd64 and dist/thoric-arm64.wsl for linux/arm64.
set -euo pipefail
cd "$(dirname "$0")/../.."
platform=
while [[ $# -gt 0 ]]; do
    case $1 in
        --platform)
            if [[ $# -lt 2 ]]; then echo "--platform needs a value such as linux/arm64" >&2; exit 2; fi
            platform=$2
            shift 2
            ;;
        --platform=*) platform=${1#*=}; shift ;;
        -*) echo "Unknown option $1" >&2; exit 2 ;;
        *) break ;;
    esac
done
host=$(docker version --format '{{.Server.Os}}/{{.Server.Arch}}')
platform=${platform:-$host}
case $platform in
    linux/amd64) package=thoric.wsl ;;
    linux/arm64) package=thoric-arm64.wsl ;;
    *) echo "WSL runs linux/amd64 and linux/arm64 distributions, not '$platform'" >&2; exit 2 ;;
esac
output=${1:-dist/$package}
image=thoric/wsl-rootfs:local-${platform#linux/}
version=${THORIC_VERSION:-$(awk -F'"' '/^version = /{ print $2; exit }' core/manager/Cargo.toml)}
# The version becomes an image tag: letters, digits, underscores, periods and dashes.
if ! [[ $version =~ ^[A-Za-z0-9_][A-Za-z0-9_.-]{0,127}$ ]]; then
    echo "THORIC_VERSION must be an image tag such as 1.2.3, not '$version'" >&2
    exit 2
fi
# WSL has read xz root filesystems since 0.61.4, well before the 2.4.4 that .wsl files need. The
# 64 MiB dictionary of -9e finds the code the Docker and NVIDIA Go binaries share, which gzip cannot.
case ${WSL_COMPRESS:-xz} in
    xz) compress=(xz -T0 -9e) ;;
    gzip) compress=(gzip -9) ;;
    *) echo "WSL_COMPRESS must be xz or gzip" >&2; exit 2 ;;
esac
# Another platform's build runs its commands under emulation, which QEMU registered with the
# kernel's binfmt_misc provides (docs/WSL_PACKAGE.md#build-the-package).
if [[ $platform != "$host" ]]; then
    echo "Building the $platform package on a $host host, under emulation."
fi
docker build --pull --platform "$platform" --build-arg "THORIC_VERSION=$version" -t "$image" -f packaging/wsl/Dockerfile .
container=$(docker create --platform "$platform" "$image")
trap 'docker rm -f "$container" >/dev/null' EXIT
# A COPY without --chmod takes its mode from the checkout, which is 0777 on a Windows drive, and one
# with --chmod=0644 gives that mode to a directory it creates. Nothing in the package may be writable
# by others except symlinks and sticky directories such as /tmp, and every directory must be enterable.
bad_modes=$(docker export "$container" | tar -tv | awk '
    (substr($1, 9, 1) == "w" && $1 !~ /^l/ && $1 !~ /[tT]$/) || ($1 ~ /^d/ && substr($1, 4, 1) !~ /[xs]/)')
if [[ -n $bad_modes ]]; then
    printf 'Wrong modes in the package:\n%s\n' "$bad_modes" >&2
    exit 1
fi
mkdir -p "$(dirname "$output")"
docker export "$container" | "${compress[@]}" > "$output"
ls -lh "$output"
echo "The $platform package installs manager $version."
