#!/usr/bin/env bash
# Thoric's helper for Linux and macOS: installs Thoric with Docker Compose on a Linux machine, a
# Raspberry Pi or a Mac with Docker Desktop, then starts, stops, updates and removes it. It runs
# straight from a release, without saving it:
#
#   curl -fsSL https://github.com/co3moz/thoric/releases/latest/download/thoric.sh | bash
#
# A command after 'bash -s --' skips the menu, such as 'bash -s -- update'. macOS runs this with its
# own bash 3.2, so the script keeps to what that release has (no ${var,,}, mapfile, associative
# arrays or empty arrays under set -u) and to the options BSD and GNU tools share. Everything runs
# from main on the last line, so a download cut short runs nothing.
#
# Copyright (C) 2026 Thoric contributors. Thoric is free software under the GNU General Public
# License, version 3 or later, and comes with no warranty: https://github.com/co3moz/thoric/blob/master/LICENSE
set -euo pipefail

MANAGER_REPOSITORY=ghcr.io/co3moz/thoric
RELEASES=https://api.github.com/repos/co3moz/thoric/releases/latest
HELPER='curl -fsSL https://github.com/co3moz/thoric/releases/latest/download/thoric.sh | bash'
WINDOWS_HELPER='irm https://github.com/co3moz/thoric/releases/latest/download/thoric.ps1 | iex'
GUIDE=https://github.com/co3moz/thoric/blob/master/docs/INSTALLATION.md
# The installation files every manager image bundles under /app/install, which the WSL package
# takes too. .shipped.sha256 records their checksums, which tells an update whether one was edited
# by hand, and marks a folder this helper installed.
INSTALL_FILES=(compose.yml deploy/Caddyfile .env.example)
# The manager's container, as compose.yml names it by default. Compose labels it with the folder it
# started from, which is how a later run finds the installation.
MANAGER=thoric-manager
# How long the helper waits for the manager to answer, in seconds: after a start, and after an
# update, whose manager may first move its database to a new layout.
START_WAIT=120
UPDATE_WAIT=180

usage() {
    cat <<'EOF'
Usage: thoric.sh [command]

Without a command, the helper installs Thoric, or shows a menu once it is installed.

  start    Start the manager
  stop     Stop the desktops and the manager
  status   Show the manager state and the dashboard address
  update   Move the manager and its compose.yml to the newest release
  mcp      Show the agent (MCP) endpoint, its token and client settings
  remove   Remove the manager's container, and the installation folder if you ask
EOF
}

# Answers come from the terminal, since the pipe from curl is the script's own input. Without a
# terminal, as in a test, they come from stdin, and an answer that does not do ends the helper, since
# it cannot be asked again. Commands the helper runs read nothing either way.
open_input() {
    if (: </dev/tty) 2>/dev/null; then
        exec 3</dev/tty
        interactive=yes
    else
        exec 3<&0
        interactive=""
    fi
    exec </dev/null
}
fail_unless_interactive() {
    if [ -z "$interactive" ]; then exit 1; fi
}
lower() {
    printf '%s' "$1" | tr '[:upper:]' '[:lower:]'
}
# Reads one line into answer; the default in brackets stands for an empty one.
ask() {
    answer=""
    IFS= read -r -p "$1${2:+ [$2]}: " answer <&3 || true
    if [ -z "$answer" ]; then answer=${2:-}; fi
}
# Asks for one of the options, listed with numbers. The answer is a number, the option's first word
# or its first letter, or empty for the default, the first option unless default_choice names
# another. Sets choice to the picked option's first word in lowercase.
choose() {
    local question=$1 picked="" fallback=${default_choice:-1} i word option
    shift
    default_choice=""
    if [ -n "$interactive" ]; then
        echo "$question"
        i=1
        for option in "$@"; do
            echo "  $i) $option"
            i=$((i + 1))
        done
    fi
    while [ -z "$picked" ]; do
        ask "Choose 1-$#" "$fallback"
        answer=$(lower "$answer")
        i=1
        for option in "$@"; do
            word=$(lower "${option%%[ ,]*}")
            if [ "$answer" = "$i" ] || [ "$answer" = "$word" ] || [ "$answer" = "${word:0:1}" ]; then
                picked=$i
                choice=$word
                break
            fi
            i=$((i + 1))
        done
        if [ -z "$picked" ]; then
            echo "$question Unknown answer: $answer"
            fail_unless_interactive
        fi
    done
    # Without a terminal the menu is not shown, so the answer goes into the output with its question.
    if [ -z "$interactive" ]; then
        option=${!picked}
        echo "$question ${option%%,*}"
    fi
}

as_root() {
    if [ "$(id -u)" = 0 ]; then "$@"; else sudo "$@"; fi
}
# docker, or sudo docker for a user outside the docker group.
dock() {
    if [ -n "$sudo_docker" ]; then sudo docker "$@"; else docker "$@"; fi
}
compose() {
    (cd "$folder" && dock compose "$@")
}
checksum() {
    if command -v sha256sum >/dev/null 2>&1; then sha256sum <"$1"; else shasum -a 256 <"$1"; fi | cut -c1-64
}
# Sets KEY=value in a settings file, in place of its line or at the end. Values go through the
# environment, so awk keeps characters such as & | \ in a password as they are. The file is written
# beside the old one and renamed over it, readable by its owner alone, since .env holds the password
# and the agent token.
set_in() {
    (umask 077 && KEY=$2 VALUE=$3 awk 'BEGIN { k = ENVIRON["KEY"]; v = ENVIRON["VALUE"] }
        index($0, k "=") == 1 { print k "=" v; found = 1; next } { print }
        END { if (!found) print k "=" v }' "$1" >"$1.new")
    mv "$1.new" "$1"
}
# One setting of the installation's .env, without the quotes around the password and the token.
setting() {
    local value single="^'(.*)'$" double='^"(.*)"$'
    value=$(sed -n "s/^$1=//p" "$folder/.env" | tail -n 1)
    if [[ $value =~ $single || $value =~ $double ]]; then value=${BASH_REMATCH[1]}; fi
    printf '%s\n' "$value"
}
port() {
    local value
    value=$(setting PORT)
    echo "${value:-8080}"
}
address() {
    echo "http://localhost:$(port)"
}
bind_address() {
    local value
    value=$(setting BIND_ADDRESS)
    echo "${value:-127.0.0.1}"
}
access() {
    local bind
    bind=$(bind_address)
    if [[ $bind == 127.* || $bind == localhost ]]; then
        echo "this computer only ($bind:$(port))"
    else
        echo "other devices too ($bind:$(port))"
    fi
}
is_version() {
    local release='^[0-9]+\.[0-9]+\.[0-9]+$'
    [[ $1 =~ $release ]]
}
# Whether the first release is newer than the second.
newer() {
    awk -v a="$1" -v b="$2" 'BEGIN {
        split(a, x, "."); split(b, y, ".")
        for (i = 1; i <= 3; i++) { if (x[i] + 0 > y[i] + 0) exit 0; if (x[i] + 0 < y[i] + 0) exit 1 }
        exit 1
    }'
}
# Prints the version of the newest published release, without the v of its tag.
newest_release() {
    local version
    version=$(curl -fsS --max-time 20 "$RELEASES" | grep -o '"tag_name": *"v[^"]*"' | head -n 1 |
        sed 's/.*"v\([^"]*\)"$/\1/') || true
    if ! is_version "$version"; then
        echo "Could not find the newest release at $RELEASES." >&2
        return 1
    fi
    echo "$version"
}
# Whether the manager answers its health check on the address Compose publishes it on, within the
# seconds given. A manager that refuses its settings or its database exits, and Docker starts it again.
manager_answers() {
    local host i=0
    host=$(bind_address)
    if [[ $host == 0.0.0.0 || $host == localhost ]]; then host=127.0.0.1; fi
    while [ "$i" -lt "$1" ]; do
        if curl -fsS --max-time 2 -o /dev/null "http://$host:$(port)/healthz" 2>/dev/null; then return 0; fi
        sleep 1
        i=$((i + 1))
    done
    return 1
}
# Whether something on this computer already answers on a port.
port_taken() {
    (: <"/dev/tcp/127.0.0.1/$1") 2>/dev/null
}

check_system() {
    system=$(uname -s)
    machine=$(uname -m)
    case $system in
        Linux | Darwin) ;;
        *)
            echo "This helper installs Thoric on Linux and macOS. On Windows, run the Windows helper in PowerShell:"
            echo "  $WINDOWS_HELPER"
            exit 1
            ;;
    esac
    case $machine in
        x86_64 | amd64 | aarch64 | arm64) ;;
        *)
            echo "Thoric runs on 64-bit x86_64 and ARM systems, and this one is $machine."
            if [[ $machine == arm* ]]; then
                echo "On a Raspberry Pi 4 or 5, install a 64-bit system such as the 64-bit Raspberry Pi OS."
            fi
            exit 1
            ;;
    esac
    # The WSL package records the level of its files here, and no other distribution has the file.
    if [ -f /usr/lib/wsl/package-level ]; then
        echo "This is the Thoric WSL distribution, which comes with Thoric installed. Manage it with the"
        echo "thoric command, such as 'thoric status', or with the Windows helper."
        exit 1
    fi
    if grep -qi microsoft /proc/sys/kernel/osrelease 2>/dev/null; then
        echo "This is a WSL distribution. The Thoric WSL package comes with Docker and the GPU runtime set up;"
        echo "the Windows helper installs it: $WINDOWS_HELPER"
        echo "This helper installs Thoric into this distribution's own Docker instead."
        echo
    fi
}

# Docker Engine on Linux comes from Docker's own install script, which knows the distributions and
# their packages, and only when asked. On a Mac it is Docker Desktop, installed by hand.
install_docker() {
    echo "Docker is not installed."
    if [ "$system" = Darwin ]; then
        echo "Install Docker Desktop and start it, then run this again:"
        echo "  https://docs.docker.com/desktop/setup/install/mac-install/"
        if command -v brew >/dev/null 2>&1; then
            echo "With Homebrew: brew install --cask docker"
        fi
        exit 1
    fi
    echo "Docker's install script from get.docker.com installs Docker Engine and Compose on Debian,"
    echo "Raspberry Pi OS, Ubuntu, Fedora, CentOS and RHEL, as root through sudo."
    choose "Install Docker Engine now?" "Yes" "No"
    if [ "$choice" != yes ]; then
        echo "Install Docker Engine and its Compose plugin, then run this again: https://docs.docker.com/engine/install/"
        exit 1
    fi
    curl -fsSL https://get.docker.com -o "$tmp/get-docker.sh"
    as_root sh "$tmp/get-docker.sh"
    if command -v systemctl >/dev/null 2>&1 && systemctl cat docker.service >/dev/null 2>&1; then
        as_root systemctl enable --now docker
    fi
}
docker_answers() {
    if docker info >/dev/null 2>&1; then
        sudo_docker=""
    elif [ "$system" = Linux ] && [ "$(id -u)" != 0 ] && command -v sudo >/dev/null 2>&1 && sudo docker info >/dev/null 2>&1; then
        sudo_docker=yes
    else
        return 1
    fi
}
# Docker Desktop starts by itself on a Mac; on Linux the daemon may be stopped, or this user outside
# the docker group, which leaves the helper sudo docker.
ensure_docker() {
    local i started="" arch
    sudo_docker=""
    if ! command -v docker >/dev/null 2>&1; then install_docker; fi
    if ! docker_answers; then
        if [ "$system" = Darwin ] && [ -d /Applications/Docker.app ]; then
            echo "Starting Docker Desktop."
            open -a Docker
            started=yes
        elif [ "$system" = Linux ] && command -v systemctl >/dev/null 2>&1 && systemctl cat docker.service >/dev/null 2>&1 &&
            ! systemctl is-active --quiet docker; then
            echo "Starting Docker."
            as_root systemctl start docker
            started=yes
        fi
        i=0
        until docker_answers; do
            i=$((i + 1))
            if [ -z "$started" ] || [ "$i" -gt 120 ]; then
                echo "Docker does not answer: $(docker info 2>&1 | tail -n 1)"
                if [ "$system" = Darwin ]; then
                    echo "Start Docker Desktop, wait until it runs, then run this again."
                else
                    echo "Start Docker, or ask for access to it, then run this again."
                fi
                exit 1
            fi
            sleep 1
        done
    fi
    if [ -n "$sudo_docker" ]; then
        user=$(id -un)
        echo "$user is not in the docker group, so the helper runs docker with sudo. Members of the group"
        echo "control Docker without sudo, which is as much as root can do."
        choose "Add $user to the docker group? It applies from the next sign-in." "Yes" "No"
        if [ "$choice" = yes ]; then
            sudo usermod -aG docker "$user"
            echo "Added $user to the docker group; until the next sign-in, the helper keeps using sudo."
        fi
    fi
    if ! dock compose version >/dev/null 2>&1; then
        echo "Docker Compose v2 is missing. Install Docker's Compose plugin, the docker-compose-plugin package"
        echo "(docker-compose-v2 in Ubuntu's own), then run this again: https://docs.docker.com/compose/install/linux/"
        exit 1
    fi
    # A 64-bit kernel can run a 32-bit Docker, as on a Raspberry Pi with a 32-bit system.
    arch=$(dock version --format '{{.Server.Arch}}' 2>/dev/null || true)
    case $arch in
        amd64 | arm64) ;;
        *)
            echo "Docker runs ${arch:-unknown} images here, and Thoric's are for amd64 and arm64. Use a 64-bit system with a 64-bit Docker."
            exit 1
            ;;
    esac
}

# The installation files of a manager image, into a folder, with their checksums. Like the WSL
# package's, the example names the manager the installation runs.
fetch_files() {
    local image=$1 into=$2 file
    for file in "${INSTALL_FILES[@]}"; do
        mkdir -p "$into/$(dirname "$file")"
        if ! dock run --rm --network none --entrypoint cat "$image" "/app/install/$file" >"$into/$file" ||
            [ ! -s "$into/$file" ]; then
            echo "$image has no /app/install/$file." >&2
            return 1
        fi
    done
    set_in "$into/.env.example" MANAGER_IMAGE "$image"
    for file in "${INSTALL_FILES[@]}"; do
        printf '%s  %s\n' "$(checksum "$into/$file")" "$file"
    done >"$into/.shipped.sha256"
}

install_thoric() {
    local username password repeat mcp mcp_auth="" mcp_token="" access_choice port_answer hosts=""
    local name_list='^[a-z0-9]([a-z0-9.-]*[a-z0-9])?(,[a-z0-9]([a-z0-9.-]*[a-z0-9])?)*$'
    local user_name='^[A-Za-z0-9._@-]{1,64}$' port_number='^[1-9][0-9]{0,4}$'
    local version image file
    echo "Thoric: installation"
    echo
    echo "The installation folder holds compose.yml and .env with the settings. Images, containers and"
    echo "volumes stay in Docker."
    while true; do
        ask "Folder" "$folder"
        # A tilde typed at the question is a person's home, which only the shell's own words expand.
        # shellcheck disable=SC2088
        case $answer in "~") answer=$HOME ;; "~/"*) answer=$HOME/${answer#"~/"} ;; esac
        if [[ $answer != /* ]]; then
            echo "Use a full path, such as $HOME/thoric."
        elif [ -e "$answer" ] && [ -n "$(ls -A "$answer" 2>/dev/null)" ]; then
            echo "$answer is not empty; choose a new folder."
        else
            folder=$answer
            break
        fi
        fail_unless_interactive
    done

    echo
    echo "The dashboard asks for one username and password."
    while true; do
        ask "Username" admin
        username=$answer
        if [[ $username =~ $user_name ]]; then break; fi
        echo "Use letters, digits, dots, dashes, underscores and @ (at most 64 characters)."
        fail_unless_interactive
    done
    # The manager requires at least eight characters for a password; an empty one turns sign-in off.
    while true; do
        password="" repeat=""
        IFS= read -rs -p "Password (at least 8 characters, or empty for no password): " password <&3 || true
        if [ -n "$interactive" ]; then echo; fi
        if [ -z "$password" ]; then
            break
        fi
        if [ ${#password} -lt 8 ]; then
            echo "The password needs at least 8 characters."
        elif [[ $password == *"'"* ]]; then
            echo "The password must not contain single quotes."
        else
            IFS= read -rs -p "Repeat the password: " repeat <&3 || true
            if [ -n "$interactive" ]; then echo; fi
            if [ "$password" = "$repeat" ]; then break; fi
            echo "The passwords do not match."
        fi
        fail_unless_interactive
    done

    echo
    echo "Agents such as Claude Code reach every desktop through the root MCP endpoint, /mcp. MCP endpoints"
    echo "for chosen desktops can be made on the dashboard later, whatever you answer here."
    choose "Enable root agent access (/mcp)?" "Yes" "No"
    mcp=$choice
    if [ "$mcp" = yes ]; then
        choose "How do agents authenticate?" "Random token, generated now" "Custom token, entered next" \
            "None, no token required"
        mcp_auth=$choice
    fi
    # An HTTP header carries visible ASCII only, and the token is single-quoted in .env like the password.
    if [ "$mcp_auth" = custom ]; then
        while true; do
            ask "Token (up to 256 characters)"
            mcp_token=$answer
            if [ -n "$mcp_token" ] && [ ${#mcp_token} -le 256 ] && [[ $mcp_token != *"'"* ]] &&
                printf '%s\n' "$mcp_token" | LC_ALL=C grep -qx '[!-~]*'; then
                break
            fi
            echo "Use 1 to 256 visible ASCII characters, without spaces or single quotes."
            fail_unless_interactive
        done
        if [ ${#mcp_token} -lt 32 ]; then
            echo "A short token is easy to guess, and whoever has it controls every desktop."
        fi
    fi

    echo
    if [ -z "$password" ]; then
        echo "There is no password, so anyone who reaches the dashboard controls every desktop."
    fi
    if [ "$mcp_auth" = none ]; then
        echo "Agents need no token, so anyone who reaches the agent endpoint controls every desktop."
    fi
    choose "Who can reach the dashboard?" "This computer only, 127.0.0.1" "Other devices too, 0.0.0.0"
    access_choice=$choice

    echo
    while true; do
        ask "Port of the dashboard" 8080
        port_answer=$answer
        if ! [[ $port_answer =~ $port_number ]] || [ "$port_answer" -gt 65535 ]; then
            echo "Use a port number from 1 to 65535."
        elif port_taken "$port_answer"; then
            echo "Something on this computer already answers on port $port_answer; choose another one."
        else
            break
        fi
        fail_unless_interactive
    done
    # The manager answers private addresses by themselves; a name, such as the computer's, has to be listed.
    if [ "$access_choice" = other ]; then
        echo
        echo "Other devices open the dashboard at this computer's IP address. To open it by a name too, such"
        echo "as the computer's name, list the names here; ALLOWED_HOSTS in .env changes them later."
        while true; do
            ask "Names, separated by commas (empty for none)"
            hosts=$(lower "$answer" | sed -E 's/[[:space:]]*,[[:space:]]*/,/g; s/^[[:space:]]+//; s/[[:space:]]+$//')
            if [ -z "$hosts" ] || [[ $hosts =~ $name_list ]]; then break; fi
            echo "Use letters, digits, dots and dashes, with a comma between two names."
            fail_unless_interactive
        done
    fi

    # Releases pin the manager, as the WSL package does, so an installation stays on its release
    # until an update moves it.
    echo
    if version=$(newest_release); then
        image=$MANAGER_REPOSITORY:$version
    else
        image=$MANAGER_REPOSITORY:latest
        echo "Installing $image instead."
    fi
    echo "Downloading the manager $image. Desktop images download when you first create a desktop."
    dock pull "$image"
    fetch_files "$image" "$tmp/files"
    mkdir -p "$folder/deploy" "$folder/update"
    for file in "${INSTALL_FILES[@]}" .shipped.sha256; do
        cp "$tmp/files/$file" "$folder/$file"
    done
    (umask 077 && cp "$folder/.env.example" "$folder/.env")
    if [ -n "$password" ]; then
        set_in "$folder/.env" AUTH_MODE password
        set_in "$folder/.env" ADMIN_PASSWORD "'$password'"
    else
        set_in "$folder/.env" AUTH_MODE none
    fi
    set_in "$folder/.env" ADMIN_USERNAME "$username"
    if [ "$mcp" = yes ]; then
        set_in "$folder/.env" ROOT_MCP_ENABLED true
        if [ "$mcp_auth" = none ]; then
            set_in "$folder/.env" MCP_AUTH_MODE none
        else
            set_in "$folder/.env" MCP_AUTH_MODE token
            if [ -z "$mcp_token" ]; then mcp_token=$(od -An -tx1 -N32 /dev/urandom | tr -d ' \n'); fi
            set_in "$folder/.env" MCP_TOKEN "'$mcp_token'"
        fi
    else
        set_in "$folder/.env" ROOT_MCP_ENABLED false
    fi
    if [ "$access_choice" = other ]; then
        set_in "$folder/.env" BIND_ADDRESS 0.0.0.0
    else
        set_in "$folder/.env" BIND_ADDRESS 127.0.0.1
    fi
    set_in "$folder/.env" PORT "$port_answer"
    if [ -n "$hosts" ]; then
        set_in "$folder/.env" ALLOWED_HOSTS "localhost,$hosts"
    fi

    echo "Settings written to $folder/.env."
    if [ -z "$password" ]; then
        echo "The dashboard opens without signing in; set ADMIN_PASSWORD and AUTH_MODE=password in .env before"
        echo "other people can reach it."
    fi
    if [ "$access_choice" = other ]; then
        echo "Other devices can reach the dashboard too: it listens on 0.0.0.0:$port_answer, and this"
        echo "computer's firewall has to let them in."
        if [ -n "$hosts" ]; then
            echo "Besides this computer's IP addresses, it answers to ${hosts//,/, }."
        fi
    else
        echo "Only this computer can reach the dashboard: it listens on 127.0.0.1:$port_answer."
    fi
    host_notes
    echo

    choose "Start Thoric now?" "Yes" "No"
    if [ "$choice" = yes ]; then
        start_thoric
    else
        echo "Run the helper again to start it, or 'docker compose up -d' in $folder."
    fi
    echo
    if [ "$mcp" = yes ]; then
        agent_settings
        echo
    else
        echo "Root agent access (/mcp) is off; MCP endpoints made on the dashboard work without it."
    fi
    echo "Run the same line again to start, stop, update or remove Thoric:"
    echo "  $HELPER"
}

# What the host itself may need, which the helper leaves to the person: memory for the desktops, the
# kernel's memory controller on a Raspberry Pi and the NVIDIA Container Toolkit.
host_notes() {
    local info memory runtimes
    info=$(dock info 2>&1 || true)
    memory=$(dock info --format '{{.MemTotal}}' 2>/dev/null || echo 0)
    if [[ $memory =~ ^[0-9]+$ ]] && [ "$memory" -gt 0 ] && [ "$memory" -lt 7516192768 ]; then
        echo "Docker has $(awk -v m="$memory" 'BEGIN { printf "%.1f", m / 1073741824 }') GiB of memory, and every desktop takes some of its own; 8 GB or more suits several."
        if [ "$system" = Darwin ]; then echo "Docker Desktop's Settings > Resources sets it."; fi
    fi
    if [[ $info == *"No memory limit support"* ]]; then
        echo "Docker warns 'No memory limit support', so it ignores the memory limits of desktops. On a Raspberry"
        echo "Pi, add cgroup_enable=memory to the line in /boot/firmware/cmdline.txt and reboot."
    fi
    if [ "$system" = Darwin ]; then
        echo "Docker Desktop passes no GPU to its virtual machine, so desktops render in software."
    elif [ -e /proc/driver/nvidia/version ]; then
        runtimes=$(dock info --format '{{json .Runtimes}}' 2>/dev/null || true)
        if [[ $runtimes != *nvidia* ]] && [ ! -e /etc/cdi/nvidia.yaml ] && [ ! -e /var/run/cdi/nvidia.yaml ]; then
            echo "This computer has an NVIDIA GPU, which desktops reach through the NVIDIA Container Toolkit, and"
            echo "Docker does not list it. Install it, then run 'sudo nvidia-ctk runtime configure --runtime=docker'"
            echo "and restart Docker: $GUIDE#gpu-on-a-linux-host"
        fi
    fi
}

manager_container() {
    local name
    name=$(setting MANAGER_CONTAINER)
    echo "${name:-$MANAGER}"
}
manager_state() {
    dock inspect --format '{{.State.Status}}' "$(manager_container)" 2>/dev/null || echo missing
}
start_thoric() {
    echo "Starting Thoric."
    compose up -d
    if ! manager_answers "$START_WAIT"; then
        echo "The manager did not answer within $START_WAIT seconds. Its last lines:"
        compose logs --no-log-prefix --tail=30 manager || true
        exit 1
    fi
    echo "Thoric answers at $(address), reachable from $(access)."
}
# The desktops stop with the manager, as they do with the WSL distribution; the dashboard starts
# them again.
stop_desktops() {
    local desktops
    desktops=$(dock ps -q --filter label=thoric.environment)
    if [ -n "$desktops" ]; then
        echo "Stopping the desktops."
        # shellcheck disable=SC2086 # one container ID a word
        dock stop $desktops >/dev/null
    fi
}
stop_thoric() {
    stop_desktops
    compose stop
    echo "Thoric stopped. Start desktops again from the dashboard once it runs."
}
show_status() {
    local state
    state=$(manager_state)
    echo "Thoric is installed in $folder, with the manager $(setting MANAGER_IMAGE), which is $state."
    if [ "$state" = running ]; then
        echo "Dashboard: $(address), reachable from $(access)"
    fi
}

agent_settings() {
    local enabled mode token header claude_header manager
    enabled=$(setting ROOT_MCP_ENABLED)
    mode=$(setting MCP_AUTH_MODE)
    token=$(setting MCP_TOKEN)
    if [ "$enabled" != true ] || { [ "$mode" != none ] && [ -z "$token" ]; }; then
        echo "Root agent access (/mcp) is off. MCP endpoints made on the dashboard's MCP page work without it,"
        echo "each with its own token, and its Connect shows their client settings. To turn on /mcp, which"
        echo "reaches every desktop, set ROOT_MCP_ENABLED=true and MCP_TOKEN in $folder/.env, then run"
        echo "'docker compose up -d' there."
        return 0
    fi
    manager=$(manager_container)
    if [ "$mode" = none ]; then
        echo "Agent access (MCP) is enabled without a token. Anything that reaches the address controls every desktop."
        header="" claude_header=""
    else
        echo "Agent access (MCP) is enabled. The token grants administrator access to every desktop."
        header=$'\n'"  Header:  Authorization: Bearer $token"
        claude_header=" --header 'Authorization: Bearer $token'"
    fi
    cat <<EOF

Streamable HTTP
  URL:     $(address)/mcp$header

Claude Code:
  claude mcp add --transport http thoric $(address)/mcp$claude_header

stdio bridge (no token in the client; it runs inside the manager container):
  {
    "mcpServers": {
      "thoric": {
        "command": "docker",
        "args": ["exec", "-i", "$manager", "python3", "/app/mcp_bridge.py"]
      }
    }
  }

The token is MCP_TOKEN in $folder/.env; run 'docker compose up -d' there after changing it.
EOF
}

# Settings added in a release reach the manager only through the environment list in its
# compose.yml, so an update brings the installation files of the manager it moves to, as the WSL
# package's does. A file still as it was installed is replaced. One edited by hand stays when the
# release brings the same file again; otherwise it is saved as <file>.bak first. .env holds the
# settings and is never one of them.
update_install_files() {
    local file new current shipped
    for file in "${INSTALL_FILES[@]}"; do
        new=$(checksum "$tmp/files/$file")
        current=""
        if [ -e "$folder/$file" ]; then current=$(checksum "$folder/$file"); fi
        shipped=$(awk -v file="$file" '$2 == file { print $1 }' "$folder/.shipped.sha256" 2>/dev/null || true)
        if [ "$current" = "$new" ]; then
            continue
        elif [ -n "$current" ] && [ "$current" != "$shipped" ]; then
            if [ "$new" = "$shipped" ]; then continue; fi
            cp -p "$folder/$file" "$folder/$file.bak"
            echo "$file had changes of your own; they are saved in $folder/$file.bak. Carry over what you still need."
        fi
        mkdir -p "$(dirname "$folder/$file")"
        cp "$tmp/files/$file" "$folder/$file.new"
        mv -f "$folder/$file.new" "$folder/$file"
        echo "Updated $file."
    done
    cp "$tmp/files/.shipped.sha256" "$folder/.shipped.sha256"
}
# Until the new manager answers, which clears previous, any end of the update puts .env and the
# installation files back as they were and recreates the manager they name.
UPDATED_FILES=(.env "${INSTALL_FILES[@]}" .shipped.sha256)
finish_update() {
    local status=$? file
    if [ -n "$previous" ]; then
        status=1
        echo "Taking the update back: .env and the installation files are put back as they were."
        for file in "${UPDATED_FILES[@]}"; do
            if [ -e "$previous/$file" ]; then
                cp -p "$previous/$file" "$folder/$file" || echo "Could not put back $folder/$file." >&2
            fi
        done
        echo "Recreating the previous manager."
        if compose up -d && manager_answers "$UPDATE_WAIT"; then
            echo "The previous manager answers again."
        else
            echo "The previous manager does not answer either; 'docker compose logs manager' in $folder shows why."
        fi
    fi
    rm -rf "$tmp"
    exit "$status"
}
# The helper pins the manager to a release, and only an update moves that pin, to the newest release
# and never back. Any other MANAGER_IMAGE, such as latest or a digest, was chosen by hand and is
# pulled again as it is.
update_thoric() {
    local current image pinned version="" file
    current=$(setting MANAGER_IMAGE)
    image=$current
    pinned=${current#"$MANAGER_REPOSITORY":}
    if [ -z "$current" ] || { [ "$pinned" != "$current" ] && is_version "$pinned"; }; then
        version=$(newest_release) || exit 1
        if [ -n "$current" ] && ! newer "$version" "$pinned"; then
            echo "Thoric runs $pinned, and the newest release is $version, so there is nothing to update."
            return 0
        fi
        image=$MANAGER_REPOSITORY:$version
    fi
    echo "Downloading the manager $image."
    dock pull "$image"
    fetch_files "$image" "$tmp/files"
    previous=$tmp/previous
    mkdir -p "$previous/deploy"
    for file in "${UPDATED_FILES[@]}"; do
        if [ -e "$folder/$file" ]; then cp -p "$folder/$file" "$previous/$file"; fi
    done
    trap finish_update EXIT
    update_install_files
    if [ "$image" != "$current" ]; then
        set_in "$folder/.env" MANAGER_IMAGE "$image"
        echo "The manager moves from ${pinned:-latest} to $version."
    fi
    echo "Recreating the manager $image."
    compose up -d
    if ! manager_answers "$UPDATE_WAIT"; then
        echo "The manager $image did not answer within $UPDATE_WAIT seconds. Its last lines:"
        compose logs --no-log-prefix --tail=30 manager || true
        exit 1
    fi
    previous=""
    echo "The manager answers at $(address)."
    echo "Desktop images you have downloaded update from their row on the dashboard's Images page."
}

# The manager's container goes, with its network. What the desktops keep stays in Docker: their
# containers, images and volumes, and the manager's database in its volume, which a new installation
# with the same settings takes up again.
remove_thoric() {
    local volume file
    volume=$(setting MANAGER_DATA_VOLUME)
    volume=${volume:-thoric-data}
    echo "Removing Thoric takes away the manager's container. Desktop containers, images and volumes stay,"
    echo "and so does the manager's database in the $volume volume, which a new installation takes up again."
    default_choice=2
    choose "Remove Thoric?" "Yes" "No"
    if [ "$choice" != yes ]; then return 0; fi
    stop_desktops
    compose down
    # Only the files the helper wrote go, so a folder that holds others stays with them.
    default_choice=2
    choose "Delete the settings in $folder too, with the password and the agent token?" "Yes" "No"
    if [ "$choice" = yes ]; then
        for file in "${UPDATED_FILES[@]}"; do
            rm -f "$folder/$file" "$folder/$file.bak"
        done
        rmdir "$folder/update" "$folder/deploy" "$folder" 2>/dev/null || true
        if [ -e "$folder" ]; then
            echo "Deleted the settings; $folder stays, since it holds other files too."
        else
            echo "Deleted $folder."
        fi
    fi
    echo "Thoric is removed. To remove what stays as well, list it with:"
    echo "  docker ps -a --filter label=thoric.managed"
    echo "  docker volume ls --filter label=thoric.managed"
    echo "  docker image ls 'ghcr.io/co3moz/thoric/*'"
    echo "and remove what you no longer need, and the database with 'docker volume rm $volume'."
}

menu() {
    local state
    show_status
    state=$(manager_state)
    echo
    if [ "$state" = running ]; then
        default_choice=5
        choose "What next?" "Update to the newest release" "Stop Thoric and its desktops" \
            "Agent settings, for connecting an agent" "Remove Thoric" "Quit"
    else
        choose "What next?" "Start Thoric" "Update to the newest release" \
            "Agent settings, for connecting an agent" "Remove Thoric" "Quit"
    fi
    run_command "$choice"
}
run_command() {
    case $1 in
        start) start_thoric ;;
        stop) stop_thoric ;;
        status) show_status ;;
        update) update_thoric ;;
        mcp | agent) agent_settings ;;
        remove) remove_thoric ;;
        quit) ;;
        help | -h | --help) usage ;;
        *)
            usage >&2
            exit 2
            ;;
    esac
}

main() {
    interactive="" default_choice="" sudo_docker="" previous="" choice="" answer=""
    case ${1:-} in
        help | -h | --help)
            usage
            return 0
            ;;
    esac
    open_input
    tmp=$(mktemp -d "${TMPDIR:-/tmp}/thoric.XXXXXX")
    trap 'rm -rf "$tmp"' EXIT
    check_system
    ensure_docker
    # The installation is the folder Compose started the manager from, or else ~/thoric.
    local manager_here=""
    folder=$(dock inspect --format '{{index .Config.Labels "com.docker.compose.project.working_dir"}}' "$MANAGER" 2>/dev/null || true)
    if [ "$folder" = "<no value>" ]; then
        echo "A container named $MANAGER is on this Docker, and Compose did not start it. Remove or rename it,"
        echo "then run this again."
        exit 1
    elif [ -n "$folder" ]; then
        manager_here=yes
    else
        folder=$HOME/thoric
    fi
    if [ "$folder" = /opt/thoric ]; then
        echo "Thoric runs from the WSL package here; manage it with the thoric command, such as 'thoric status'."
        exit 1
    fi
    if [ -f "$folder/.shipped.sha256" ] && [ -f "$folder/.env" ]; then
        if [ $# -gt 0 ]; then run_command "$1"; else menu; fi
    elif [ -n "$manager_here" ]; then
        echo "Thoric runs from $folder, which this helper did not install. Manage it there with docker compose,"
        echo "as $GUIDE describes."
        exit 1
    elif [ $# -gt 0 ]; then
        echo "Thoric is not installed yet; run the helper without a command to install it."
        exit 1
    else
        install_thoric
    fi
}

main "$@"
