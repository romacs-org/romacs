#!/usr/bin/env bash
# Prepare immutable external fixtures for the Linux GUI integration tests.
# Run once after installing native test tools; tests themselves stay offline.
set -euo pipefail

repo_root="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/../.." && pwd)"
package_cache="$repo_root/target/romacs-gui-tests/packages"
mkdir -p "$package_cache"
exec 9>"$package_cache/.setup.lock"
flock 9

prepare_package() {
    local package="$1" repository="$2" revision="$3"
    local destination="$package_cache/$package"
    if [[ -e "$destination" ]]; then
        if [[ "$(git -C "$destination" rev-parse HEAD)" != "$revision" ]] \
            || ! git -C "$destination" diff --quiet HEAD \
            || [[ ! -f "$destination/$package.el" ]]; then
            echo "Refusing to overwrite mismatched GUI fixture: $destination" >&2
            return 1
        fi
        return
    fi
    local staging
    staging="$(mktemp -d "$package_cache/$package.XXXXXX")"
    git -C "$staging" init --quiet
    git -C "$staging" fetch --quiet --depth=1 "$repository" "$revision"
    git -C "$staging" -c advice.detachedHead=false checkout --quiet --detach FETCH_HEAD
    test -f "$staging/$package.el"
    mv -T -- "$staging" "$destination"
}

# svg-lib matches the existing MELPA source lock; svg-tag-mode is pinned too.
prepare_package svg-lib https://github.com/rougier/svg-lib f2cc9615ef3a052747135d34f31c423a26592f14
prepare_package svg-tag-mode https://github.com/rougier/svg-tag-mode 13e888b8bd9a0664d060149a44a751b2113331b6
