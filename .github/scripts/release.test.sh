#!/usr/bin/env bash
# Tests for release.sh. Needs only bash and git: run `bash .github/scripts/release.test.sh`.
set -euo pipefail

script="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)/release.sh"
work="$(mktemp -d)"
trap 'rm -rf "$work"' EXIT

failures=0
pass() { echo "ok   $1"; }
fail() { echo "FAIL $1" >&2; failures=$((failures + 1)); }

expect_valid() {
    if bash "$script" validate "$1" 2>/dev/null; then pass "valid: $1"; else fail "should accept: $1"; fi
}
expect_invalid() {
    if (cd "$work" && bash "$script" validate "$1" 2>/dev/null); then
        fail "should reject: $1"
    else
        pass "invalid: $(printf '%q' "$1")"
    fi
}

# --- validate ---------------------------------------------------------------

for v in 0.5.5 1.0.0 10.20.30 0.0.0; do expect_valid "$v"; done
for v in 1.0.0-rc.1 1.0.0-alpha 1.0.0-alpha.0.x-y 1.0.0-0 1.0.0-x-y-z.--; do expect_valid "$v"; done
for v in "" v1.0.0 1.0 1 1.0.0.0 01.0.0 1.01.0 1.0.0-01 1.0.0- 1.0.0-rc..1 1.0.0+build \
    1.0.0-rc.1+build " 1.0.0" "1.0.0 " 1.0.0-rc_1; do
    expect_invalid "$v"
done
# Shell metacharacters: each would execute or break out if spliced into
# shell source; as an argument it must simply fail validation.
# shellcheck disable=SC2016 # the literal, unexpanded text is the point
for v in '1.0.0"; touch pwned; "' '1.0.0$(touch pwned)' '1.0.0`touch pwned`' \
    "1.0.0'; touch pwned; '" '1.0.0;touch pwned' '1.0.0|touch pwned' '1.0.0&&touch pwned' \
    $'1.0.0\ntouch pwned' $'1.0.0\n::error::spoofed'; do
    expect_invalid "$v"
done
if [ -e "$work/pwned" ] || [ -e pwned ]; then fail "an injected command ran"; else pass "no injected command ran"; fi

# --- bump -------------------------------------------------------------------

manifest_fixture() {
    cat <<'EOF'
[package]
name = "poopman"
version = "0.5.4"

[dependencies]
serde = { version = "1", features = ["derive"] }

[dev-dependencies.flate2]
version = "1.0.0"

[package.metadata.bundle]
name = "Poopman"
version = "0.5.4"
EOF
}

for eol in lf crlf; do
    manifest="$work/Cargo-$eol.toml"
    if [ "$eol" = crlf ]; then manifest_fixture | sed 's/$/\r/' > "$manifest"; else manifest_fixture > "$manifest"; fi
    original="$(cat "$manifest")"
    bash "$script" bump 0.6.0-rc.1 "$manifest"
    changed="$(diff <(printf '%s\n' "$original") "$manifest" | grep -c '^>' || true)"
    if [ "$changed" -eq 2 ] && [ "$(grep -c '^version = "0.6.0-rc.1"' "$manifest")" -eq 2 ] &&
        grep -q '^version = "1.0.0"' "$manifest"; then
        pass "bump ($eol) changes exactly the two package versions"
    else
        fail "bump ($eol) changed the wrong lines"
        diff <(printf '%s\n' "$original") "$manifest" >&2 || true
    fi
done

if bash "$script" bump '1.0.0"; touch pwned' "$work/Cargo-lf.toml" 2>/dev/null; then
    fail "bump should reject an invalid version"
else
    pass "bump rejects an invalid version"
fi

# --- publish-commit ---------------------------------------------------------

git_q() { git -c user.name=test -c user.email=test@example.com -c commit.gpgsign=false "$@"; }

origin="$work/origin.git"
git init --quiet --bare --initial-branch=main "$origin"
seed="$work/seed"
git init --quiet --initial-branch=main "$seed"
manifest_fixture > "$seed/Cargo.toml"
echo 'lock 0.5.4' > "$seed/Cargo.lock"
git_q -C "$seed" add .
git_q -C "$seed" commit --quiet -m init
git -C "$seed" remote add origin "$origin"
git -C "$seed" push --quiet origin main
base="$(git -C "$seed" rev-parse HEAD)"
epoch=1767225600

remote_ref() { git -C "$origin" rev-parse --verify --quiet "$1" || true; }

# Mirrors the publish job: a fresh checkout of BASE with the bumped manifest applied.
release_checkout() {
    local dir="$1" version="$2" at="$3"
    rm -rf "$dir"
    git clone --quiet "$origin" "$dir"
    git -C "$dir" checkout --quiet --detach "$at"
    bash "$script" bump "$version" "$dir/Cargo.toml"
    echo "lock $version" > "$dir/Cargo.lock"
}

release_checkout "$work/run1" 0.5.5 "$base"
first="$(cd "$work/run1" && bash "$script" publish-commit 0.5.5 "$base" "$epoch" 2>/dev/null)"
if [ "$(remote_ref main)" = "$first" ] && [ "$(remote_ref refs/tags/v0.5.5)" = "$first" ] &&
    [ "$(git -C "$origin" rev-parse "$first^")" = "$base" ] &&
    [ "$(git -C "$origin" log -1 --format='%at %ct' "$first")" = "$epoch $epoch" ] &&
    [ "$(git -C "$origin" log -1 --format=%s "$first")" = "chore: release v0.5.5 [skip ci]" ]; then
    pass "publish-commit pushes main and the tag to the release commit"
else
    fail "publish-commit first push"
fi

# A rerun (e.g. "Re-run failed jobs" after the release upload failed) must
# rebuild the identical commit and recognize the push already happened.
release_checkout "$work/run2" 0.5.5 "$base"
if second="$(cd "$work/run2" && bash "$script" publish-commit 0.5.5 "$base" "$epoch" 2>"$work/run2.err")" &&
    [ "$second" = "$first" ] && grep -q resuming "$work/run2.err" &&
    [ "$(remote_ref main)" = "$first" ]; then
    pass "publish-commit rerun resumes with the identical commit"
else
    fail "publish-commit rerun"
fi

# main advances after the release starts: nothing may be pushed.
base2="$first"
release_checkout "$work/run3" 0.5.6 "$base2"
git_q -C "$seed" pull --quiet origin main
echo change >> "$seed/README"
git_q -C "$seed" add README
git_q -C "$seed" commit --quiet -m "concurrent change"
git -C "$seed" push --quiet origin main
moved="$(remote_ref main)"
if (cd "$work/run3" && bash "$script" publish-commit 0.5.6 "$base2" "$epoch" 2>/dev/null); then
    fail "publish-commit should refuse when main moved"
elif [ "$(remote_ref main)" = "$moved" ] && [ -z "$(remote_ref refs/tags/v0.5.6)" ]; then
    pass "publish-commit refuses when main moved, leaving origin unchanged"
else
    fail "publish-commit changed origin after main moved"
fi

# A tag that already exists at some other commit must not be adopted.
git -C "$origin" tag v0.5.7 "$base"
release_checkout "$work/run4" 0.5.7 "$moved"
if (cd "$work/run4" && bash "$script" publish-commit 0.5.7 "$moved" "$epoch" 2>/dev/null); then
    fail "publish-commit should refuse a foreign tag"
elif [ "$(remote_ref main)" = "$moved" ]; then
    pass "publish-commit refuses a tag that points elsewhere"
else
    fail "publish-commit moved main despite a foreign tag"
fi

echo
if [ "$failures" -ne 0 ]; then
    echo "$failures test(s) failed" >&2
    exit 1
fi
echo "all release.sh tests passed"
