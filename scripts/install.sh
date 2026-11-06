#!/usr/bin/env bash
# Install Romacs for the current user on x86_64 Linux.
#
#   curl -fsSL https://thanosapollo.org/romacs/install.sh | bash
#
# Arguments, after `bash -s --' when piped:
#
#   --nightly     the rolling build of master, not the latest stable release
#   --no-compile  skip compiling the Lisp ahead of use; Romacs compiles
#                 what it loads as you go, so early sessions are slower
#   --uninstall   remove the installation; your init and data stay
#
# It verifies the release against its SHA256SUMS and installs under
# ~/.local. It asks before using sudo, and only to install missing system
# libraries. The whole script is inside `main', so a truncated download
# runs nothing.
set -euo pipefail

releases=${ROMACS_RELEASES:-https://github.com/romacs-org/romacs/releases}
name=romacs-x86_64-linux
opt=$HOME/.local/opt
prefix=$opt/$name
bin=$HOME/.local/bin
share=$HOME/.local/share
cache=${XDG_CACHE_HOME:-$HOME/.cache}/romacs
launcher=$bin/romacs
staging=''
work=''
min_glibc=2.39
libraries=(libfontconfig.so.1 libfreetype.so.6 libz.so.1 libwayland-client.so.0
           libxkbcommon.so.0 libvulkan.so.1)

say() { printf '\033[1m==>\033[0m %s\n' "$*"; }
warn() { printf 'romacs: %s\n' "$*" >&2; }
die() { warn "$*"; exit 1; }

# The release URL of CHANNEL (stable or nightly).
release_url() {
    case $1 in
        nightly) printf '%s/download/nightly\n' "$releases" ;;
        *) printf '%s/latest/download\n' "$releases" ;;
    esac
}

# True when dotted version $1 is at least $2.
version_at_least() {
    [ "$(printf '%s\n%s\n' "$2" "$1" | sort -V | head -n1)" = "$2" ]
}

check_platform() {
    [ "$(uname -s)" = Linux ] || die "Romacs needs Linux; this is $(uname -s)."
    [ "$(uname -m)" = x86_64 ] || die "releases are x86_64 only; this is $(uname -m). See README.md to build from source."
    local glibc
    glibc=$(getconf GNU_LIBC_VERSION 2>/dev/null | awk '{print $2}') || true
    [ -n "$glibc" ] || die "releases need glibc; this system has another C library. See README.md to build from source."
    version_at_least "$glibc" "$min_glibc" ||
        die "releases need glibc $min_glibc or newer; this system has $glibc (Ubuntu 24.04, Debian 13 or newer work)."
}

# The system libraries from `libraries' the dynamic linker can't find.
missing_libraries() {
    local known library
    known=$(PATH=/usr/sbin:/sbin:/usr/bin:/bin:$PATH ldconfig -p 2>/dev/null) || true
    for library in "${libraries[@]}"; do
        grep -qF "$library" <<<"$known" || printf '%s\n' "$library"
    done
}

# The command that installs the runtime's packages here, or nothing when
# this distribution isn't one we know.
package_command() {
    local id='' id_like=''
    if [ -r /etc/os-release ]; then
        id=$(. /etc/os-release && printf '%s' "${ID:-}")
        id_like=$(. /etc/os-release && printf '%s' "${ID_LIKE:-}")
    fi
    case " $id $id_like " in
        *' arch '*)
            echo 'pacman -S --needed fontconfig freetype2 zlib wayland libxkbcommon vulkan-icd-loader ttf-dejavu' ;;
        *' debian '* | *' ubuntu '*)
            echo 'apt-get install -y libfontconfig1 libfreetype6 zlib1g libwayland-client0 libxkbcommon0 libvulkan1 mesa-vulkan-drivers fonts-dejavu-core' ;;
    esac
}

# Ask on the terminal even with the script on stdin; no terminal is a no.
confirm() {
    local answer=''
    { : </dev/tty; } 2>/dev/null || return 1
    printf '%s [y/N] ' "$1" >/dev/tty
    read -r answer </dev/tty
    case $answer in [yY]*) return 0 ;; *) return 1 ;; esac
}

install_libraries() {
    local missing command
    missing=$(missing_libraries)
    [ -n "$missing" ] || return 0
    warn "missing system libraries: $(tr '\n' ' ' <<<"$missing")"
    command=$(package_command)
    [ -n "$command" ] || die "install them with your package manager, then run this again."
    if confirm "Run 'sudo $command'?"; then
        # shellcheck disable=SC2086
        sudo $command </dev/tty
    else
        die "run 'sudo $command', then run this again."
    fi
}

# Download the release to directory $2 and verify it.
fetch_release() {
    local url
    url=$(release_url "$1")
    say "Downloading Romacs ($1)"
    curl -fL --progress-bar -o "$2/$name.tar.gz" "$url/$name.tar.gz"
    curl -fsSL -o "$2/SHA256SUMS" "$url/SHA256SUMS"
    (cd "$2" && sha256sum --check --quiet SHA256SUMS) ||
        die "the download doesn't match its SHA256SUMS; nothing was installed."
}

# True when the launcher path is this installer's link.
launcher_is_ours() {
    [ "$(readlink "$launcher")" = "$prefix/bin/romacs" ]
}

# Refuse to replace a `romacs' this installer didn't put there.
check_launcher() {
    [ ! -e "$launcher" ] && [ ! -L "$launcher" ] && return 0
    launcher_is_ours || die "$launcher already exists and isn't this installer's; move it and run this again."
}

# Replace the installation with the tree in archive $1, so no file of an
# older release remains; any failure leaves the old one working.
place_release() {
    mkdir -p "$opt" "$bin" "$share"
    staging=$(mktemp -d "$opt/.romacs-new.XXXXXX")
    tar -xzf "$1" -C "$staging"
    [ -x "$staging/$name/bin/romacs" ] || die "the archive holds no bin/romacs; nothing was installed."
    [ ! -e "$prefix" ] || mv "$prefix" "$staging/old"
    if ! mv "$staging/$name" "$prefix"; then
        [ ! -e "$staging/old" ] || mv "$staging/old" "$prefix"
        die "couldn't place the new release; nothing was installed."
    fi
    ln -sfn "$prefix/bin/romacs" "$launcher"
    cp -r "$prefix/share/applications" "$prefix/share/icons" "$share/"
    # A desktop session's PATH often lacks ~/.local/bin.
    sed -i "s|^Exec=romacs|Exec=\"$launcher\"|" "$share/applications/romacs.desktop"
    if command -v update-desktop-database >/dev/null; then
        update-desktop-database -q "$share/applications" || true
    fi
}

# Build the GUI world, so the first launch restores it.
build_world() {
    say 'Building the startup image'
    "$prefix/bin/romacs" -Q --loadup --world-build
}

# Fill the compiled-Lisp store, so the first session already runs at full
# speed. Status 2 means some files stay source, which still load.
compile_lisp() {
    local status=0
    mkdir -p "$cache"
    say 'Compiling the Lisp ahead of use: a few minutes, and Ctrl-C skips it safely'
    nice -n 10 "$prefix/bin/romacs" --compile-lisp-tree --jobs "$(nproc)" \
         >"$cache/compile-lisp.log" 2>&1 || status=$?
    case $status in
        0 | 2) ;;
        *) warn "compiling failed (log: $cache/compile-lisp.log); Romacs still runs, and compiles as it loads." ;;
    esac
}

report_session() {
    case :$PATH: in
        *":$bin:"*) ;;
        *) warn "$bin isn't on your PATH; add it to your shell's profile." ;;
    esac
    [ -n "${WAYLAND_DISPLAY:-}" ] || warn "no Wayland session here; Romacs's window needs one (--batch doesn't)."
    compgen -G '/usr/share/vulkan/icd.d/*.json' >/dev/null ||
        compgen -G '/etc/vulkan/icd.d/*.json' >/dev/null ||
        warn "no Vulkan driver found; install your GPU's (Arch: vulkan-radeon, vulkan-intel or nvidia-utils)."
}

uninstall() {
    if launcher_is_ours; then rm -f -- "$launcher"; fi
    rm -rf -- "$prefix"
    rm -f -- "$share/applications/romacs.desktop" "$share/icons/hicolor/scalable/apps/romacs.svg"
    say 'Removed Romacs; your init and data stay where they were.'
}

main() {
    local channel=stable compile=yes
    while [ $# -gt 0 ]; do
        case $1 in
            --nightly) channel=nightly ;;
            --no-compile) compile=no ;;
            --uninstall) uninstall; return ;;
            *) die "unknown argument: $1" ;;
        esac
        shift
    done
    check_platform
    check_launcher
    install_libraries
    work=$(mktemp -d)
    trap 'rm -rf -- "$work" "$staging"' EXIT
    fetch_release "$channel" "$work"
    place_release "$work/$name.tar.gz"
    build_world
    report_session
    say "Romacs is installed. Run: romacs"
    [ "$compile" = no ] || compile_lisp
}

main "$@"
