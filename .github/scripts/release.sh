#!/usr/bin/env bash
# Release helpers for .github/workflows/release.yml.
#
# Every input arrives as an argument and is only ever expanded inside double
# quotes, never spliced into shell source, so a hostile version string is
# just a string that fails validation. Tested by release.test.sh.
#
#   release.sh validate VERSION
#   release.sh bump VERSION [MANIFEST]
#   release.sh publish-commit VERSION BASE_SHA COMMIT_EPOCH
set -euo pipefail

die() {
    echo "::error::$*" >&2
    exit 1
}

# Canonical SemVer without build metadata: a `+` would end up in asset file
# names and release URLs. No leading `v`; the tag adds it.
semver_num='(0|[1-9][0-9]*)'
semver_id='(0|[1-9][0-9]*|[0-9]*[A-Za-z-][0-9A-Za-z-]*)'
semver_re="^${semver_num}\.${semver_num}\.${semver_num}(-${semver_id}(\.${semver_id})*)?$"

# The rejected value is deliberately not echoed: it could carry a newline
# followed by a workflow command such as ::add-mask:: or ::error::.
validate() {
    local version="$1"
    [[ "$version" =~ $semver_re ]] ||
        die "version is not canonical SemVer: expected MAJOR.MINOR.PATCH[-PRERELEASE] without a leading v, e.g. 0.5.5 or 1.0.0-rc.1"
}

# Writes VERSION into both [package] and [package.metadata.bundle], the
# latter being what cargo-bundle stamps into the macOS .app/.dmg. Section-aware
# on purpose: a blanket sed would also rewrite dependency versions, and a
# first-match-only edit would silently leave the macOS bundle behind. A
# trailing \r is stripped from section headers so a CRLF checkout still matches.
bump() {
    local version="$1" manifest="${2:-Cargo.toml}"
    validate "$version"
    awk -v v="$version" '
        /^\[/ { sect = $0; sub(/\r$/, "", sect) }
        /^version = / && (sect == "[package]" || sect == "[package.metadata.bundle]") {
            sub(/"[^"]*"/, "\"" v "\"")
        }
        { print }
    ' "$manifest" > "$manifest.new"
    mv "$manifest.new" "$manifest"

    # Both must now read the new version, or the edit missed one.
    if [ "$(grep -c "^version = \"$version\"" "$manifest")" -ne 2 ]; then
        grep -n '^version' "$manifest" >&2
        die "expected 2 version lines at $version in $manifest"
    fi
}

# Commits the already-bumped Cargo.toml/Cargo.lock on top of BASE_SHA and
# publishes it to origin as main + tag vVERSION in one atomic push.
#
# The commit's identity, dates and message are all fixed, so rerunning this
# with the same arguments yields the same hash. That makes a rerun after a
# partial failure recognizable: if the tag already points at exactly this
# commit, the push already happened and there is nothing left to do here.
# Anything else that changed remotely aborts without touching origin.
#
# Prints the release commit's hash on stdout.
publish_commit() {
    local version="$1" base="$2" epoch="$3"
    validate "$version"
    [[ "$base" =~ ^[0-9a-f]{40}$ ]] || die "BASE_SHA must be a full commit hash"
    [[ "$epoch" =~ ^[0-9]+$ ]] || die "COMMIT_EPOCH must be a unix timestamp"
    [ "$(git rev-parse HEAD)" = "$base" ] || die "HEAD is not the release base $base"

    local tag="v$version"
    git add Cargo.toml Cargo.lock
    GIT_AUTHOR_DATE="$epoch +0000" GIT_COMMITTER_DATE="$epoch +0000" \
        git -c user.name='github-actions[bot]' \
            -c user.email='github-actions[bot]@users.noreply.github.com' \
            -c commit.gpgsign=false \
            commit --quiet -m "chore: release $tag [skip ci]" >&2
    local release_commit
    release_commit="$(git rev-parse HEAD)"

    local remote_tag remote_main
    remote_tag="$(git ls-remote origin "refs/tags/$tag" | cut -f1)"
    if [ -n "$remote_tag" ]; then
        [ "$remote_tag" = "$release_commit" ] ||
            die "tag $tag already exists at $remote_tag, which is not this release's commit $release_commit"
        echo "$tag already points at $release_commit; resuming after an earlier push" >&2
        echo "$release_commit"
        return
    fi

    remote_main="$(git ls-remote origin refs/heads/main | cut -f1)"
    [ "$remote_main" = "$base" ] ||
        die "main moved from $base to $remote_main after this release started; nothing was pushed. Start a new release run."

    # --atomic: main and the tag move together or not at all. If main moves
    # between the check above and this push, it is rejected as non-fast-forward.
    git tag "$tag" "$release_commit"
    git push --atomic origin "$release_commit:refs/heads/main" "refs/tags/$tag" >&2
    echo "$release_commit"
}

cmd="${1:-}"
[ "$#" -gt 0 ] && shift
case "$cmd" in
    validate)
        [ "$#" -eq 1 ] || die "usage: release.sh validate VERSION"
        validate "$1"
        ;;
    bump)
        [ "$#" -ge 1 ] && [ "$#" -le 2 ] || die "usage: release.sh bump VERSION [MANIFEST]"
        bump "$@"
        ;;
    publish-commit)
        [ "$#" -eq 3 ] || die "usage: release.sh publish-commit VERSION BASE_SHA COMMIT_EPOCH"
        publish_commit "$@"
        ;;
    *)
        die "usage: release.sh {validate|bump|publish-commit} ..."
        ;;
esac
