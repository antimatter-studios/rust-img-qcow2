#!/usr/bin/env bash
# The release tarball is an install prefix -- bin/rust-img-qcow2, bin/img.qcow2 a
# relative symlink to it, the man pages and completions under share/,
# share/rust-img-qcow2/CAVEATS and LICENSE -- and the tools in it run and
# identify themselves; a build that is wrong in any of the ways below is
# refused, and leaves no tarball.
#
# This runs the real packaging script against stand-in binaries in a
# sandbox: one that behaves, and one for each way a build can be wrong. The
# `cli` job in ci.yml and the release workflow run the same script against
# the real binary, so the checks here are the checks a release makes. A
# gate that cannot fail is indistinguishable from no gate.
set -uo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
PACKAGE="$ROOT/scripts/package-cli.sh"
NAME="test-package-cli"
passed=0
fails=0
ok() { passed=$((passed + 1)); }
fail() {
    echo "FAIL  $NAME: $*" >&2
    fails=$((fails + 1))
}

sandbox="$(mktemp -d)"
trap 'rm -rf "$sandbox"' EXIT

crate="$(sed -n 's/^name = "\(.*\)"$/\1/p' "$ROOT/Cargo.toml" | head -n 1)"
[ "$crate" = "am-img-qcow2" ] && ok || fail "crate name read from Cargo.toml: '$crate'"

# stub DIR VERSION [NAMES] [MAN] [CAVEATS]: a stand-in rust-img-qcow2 in DIR
# that reports VERSION, lists NAMES (default img.qcow2), and writes man pages
# unless MAN is "noman".
stub() {
    local dir="$sandbox/$1" version="$2" names="${3:-img.qcow2}" man="${4:-man}"
    mkdir -p "$dir"
    cat >"$dir/rust-img-qcow2" <<STUB
#!/usr/bin/env bash
me="\$(basename "\$0")"
case "\$1" in
    --help) echo "Usage: \$me"; exit 0 ;;
    --version) echo "\$me ($crate) $version"; exit 0 ;;
    generate)
        case "\$2" in
            names) for n in $names; do echo "\$n"; done ;;
            man)
                [ "$man" = man ] || exit 0
                mkdir -p "\$3/man/man1"
                for n in img.qcow2 rust-img-qcow2 img.qcow2-read; do echo ".TH \$n 1" >"\$3/man/man1/\$n.1"; done ;;
            completions)
                mkdir -p "\$3/zsh/site-functions" "\$3/bash-completion/completions" "\$3/fish/vendor_completions.d"
                for n in img.qcow2 rust-img-qcow2; do
                    echo x >"\$3/zsh/site-functions/_\$n"
                    echo x >"\$3/bash-completion/completions/\$n"
                    echo x >"\$3/fish/vendor_completions.d/\$n.fish"
                done ;;
        esac ;;
    *) exit 2 ;;
esac
STUB
    chmod +x "$dir/rust-img-qcow2"
    printf '%s\n' "$dir"
}

# package ARGS...: run the script in a fresh output directory, print the
# tarball's absolute path; record the directory for the leftover check.
package() {
    local out="$sandbox/out-$RANDOM$RANDOM" name status
    mkdir -p "$out"
    printf '%s\n' "$out" >"$sandbox/package-out"
    [ -z "${STALE:-}" ] || echo stale >"$out/$STALE"
    name="$(cd "$out" && bash "$PACKAGE" "$@" 2>"$sandbox/stderr")"
    status=$?
    if [ "$status" -ne 0 ]; then
        printf '%s' "$name"
        return "$status"
    fi
    printf '%s\n' "$out/$name"
}

[ -f "$PACKAGE" ] && ok || fail "scripts/package-cli.sh exists"

# ---- A good build: the tarball, its name, and exactly its layout. -------
good="$(stub good 9.9.9)"
if tarball="$(package 9.9.9 linux-x86_64 "$good")"; then
    ok
else
    fail "a good build packages: $(cat "$sandbox/stderr")"
    tarball=""
fi
[ "$(basename "$tarball")" = "$crate-9.9.9-linux-x86_64.tar.gz" ] && ok ||
    fail "the tarball is named <crate>-<version>-<label>.tar.gz, got '$tarball'"

# Without the tarball the content checks fail rather than fall silent.
[ -f "$tarball" ] && ok || fail "the packaged tarball exists at '$tarball'"
if [ -f "$tarball" ]; then
    unpacked="$sandbox/unpacked"
    mkdir -p "$unpacked"
    tar -xzf "$tarball" -C "$unpacked"
    members="$(cd "$unpacked" && find . \( -type f -o -type l \) | sed 's|^\./||' | sort | tr '\n' ' ')"
    want="LICENSE bin/img.qcow2 bin/rust-img-qcow2 share/bash-completion/completions/img.qcow2 share/bash-completion/completions/rust-img-qcow2 share/fish/vendor_completions.d/img.qcow2.fish share/fish/vendor_completions.d/rust-img-qcow2.fish share/man/man1/img.qcow2-read.1 share/man/man1/img.qcow2.1 share/man/man1/rust-img-qcow2.1 share/rust-img-qcow2/CAVEATS share/zsh/site-functions/_img.qcow2 share/zsh/site-functions/_rust-img-qcow2 "
    [ "$members" = "$want" ] && ok || fail "the tarball holds exactly the prefix layout, got: $members"
    [ -L "$unpacked/bin/img.qcow2" ] && [ "$(readlink "$unpacked/bin/img.qcow2")" = rust-img-qcow2 ] && ok ||
        fail "bin/img.qcow2 is a relative symlink to rust-img-qcow2"
    cmp -s "$unpacked/bin/rust-img-qcow2" "$good/rust-img-qcow2" && ok ||
        fail "bin/rust-img-qcow2 is the built binary"
    cmp -s "$unpacked/share/rust-img-qcow2/CAVEATS" "$ROOT/packaging/CAVEATS" && ok ||
        fail "share/rust-img-qcow2/CAVEATS is packaging/CAVEATS"
    cmp -s "$unpacked/LICENSE" "$ROOT/LICENSE" && ok || fail "LICENSE is the repository's"
fi
[ "$(wc -l <"$ROOT/packaging/CAVEATS")" -le 4 ] && ok || fail "packaging/CAVEATS is at most four lines"

# ---- Each way a build can be wrong is refused, with no tarball left. -----
refused() {
    local why="$1" out_dir left stdout
    shift
    if stdout="$(package "$@")"; then
        fail "$why is refused, but packaging succeeded: $stdout"
        return
    fi
    ok
    [ -z "$stdout" ] && ok || fail "$why names no tarball on stdout: $stdout"
    out_dir="$(cat "$sandbox/package-out")"
    left="$(find "$out_dir" -maxdepth 1 -name '*.tar.gz')"
    [ -z "$left" ] && ok || fail "$why leaves no tarball behind, found: $left"
}

refused "a missing binary" 9.9.9 linux-x86_64 "$sandbox/nowhere"
refused "a binary reporting a version other than the tag's" 9.9.9 linux-x86_64 "$(stub wrongver 1.0.0)"
refused "a binary that lists a name this layout does not ship" 9.9.9 linux-x86_64 "$(stub extra 9.9.9 'img.qcow2 img.vhd')"
refused "a binary that lists none of its names" 9.9.9 linux-x86_64 "$(stub nonames 9.9.9 ' ')"
refused "a binary that writes no man pages" 9.9.9 linux-x86_64 "$(stub noman 9.9.9 img.qcow2 noman)"
refused "a missing label" 9.9.9 "" "$good"
refused "a missing version" "" linux-x86_64 "$good"
STALE="$crate-9.9.9-linux-x86_64.tar.gz" refused "a failure beside a previous run's tarball" \
    9.9.9 linux-x86_64 "$sandbox/nowhere"

# ---- The release workflow packages through this script, and attests. ----
release="$ROOT/.github/workflows/release.yml"
grep -q 'scripts/package-cli.sh' "$release" && ok ||
    fail "release.yml packages through scripts/package-cli.sh"
grep -q 'cargo build --release --locked --features cli --bin rust-img-qcow2' "$release" && ok ||
    fail "release.yml builds the rust-img-qcow2 target with the cli feature"
grep -qE 'uses: actions/attest-build-provenance@[0-9a-f]{40}' "$release" && ok ||
    fail "release.yml attests build provenance with the action pinned to a commit"
grep -q 'subject-path: dist/\*.tar.gz' "$release" && ok ||
    fail "release.yml attests the tarballs"

if [ "$fails" -gt 0 ]; then
    echo "test result: FAILED. $passed passed; $fails failed"
    exit 1
fi
echo "test result: ok. $passed passed; 0 failed"
echo "$NAME: all checks passed"
