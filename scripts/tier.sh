#!/usr/bin/env bash
# tier.sh LABEL LOG-NAME MAX-LINES MAX-BYTES -- COMMAND [ARG...]
#
# One test tier, run QUIETLY and under a budget. The whole run goes to
# tmp/logs/<LOG-NAME>.log; a pass prints one verdict line naming the log, a
# failure prints a verdict naming the command's status and that log, and a
# run that passed but printed more than its budget fails with status 65.
#
# ONE WRAPPER FOR BOTH CALLERS. chores.yml runs the tiers for a person at a
# terminal and .github/workflows/ci.yml runs them for the gate, and they run
# the SAME command through the SAME budget -- so a tier that has outgrown its
# budget says so here, before the push, rather than in a CI log nobody was
# going to read. tests/ci_profile.rs checks that the two files agree on every
# tier's numbers; the duplication is deliberate (the workflow cannot read
# chores.yml without installing chore on three runner platforms) and it is
# checked rather than trusted.
#
# WHY THE BUDGET IS PART OF THE TASK. A passing run that prints three
# thousand lines hides the twenty that matter, and every reader pays for it:
# a person scrolling, a CI log viewer, and an agent working in the
# repository, which re-reads its whole transcript on each step and so pays
# for one verbose run many times over. Measured across this constellation:
# 4,661M cache-read tokens against 9.5M of output, and command output was the
# largest single contributor a repository controls.
#
# The budgets themselves are in chores.yml, next to the command each one
# bounds, and every one of them was MEASURED -- see the table there. Raise
# one deliberately when a tier grows, the way the executed-test floors are
# raised; a budget nobody can breach measures nothing.
#
# VERBOSE. `OUTPUT_BUDGET_VERBOSE=1`, or `--verbose`/`-v` in the chore
# invocation's CLI_ARGS (`chore test:debug -- --verbose`), streams the run as
# it happens as well as logging it. It does NOT lift the budget: the log is
# the same size either way, and a tier that has outgrown its budget should say
# so whether or not anybody was watching.
#
# THE VARIABLE WAS `FLTH_VERBOSE`, AND THE FAIL TAIL WAS `FLTH_FAIL_TAIL`.
# Both moved to `OUTPUT_BUDGET_*` when the wrapper became rust-fs-core's. A
# rename like that fails SILENTLY where it is not caught -- the old name is
# simply not read, nothing errors, and the run stays quiet -- so it is written
# here, where somebody grepping for the old name lands. The canonical script
# reports an `FLTH_*` variable that is set rather than honouring it.
#
# A FAILURE NO LONGER PRINTS A TAIL BY DEFAULT. The vendored copy this
# replaced printed forty lines; rust-fs-core's prints one line naming the
# status, the size and the log, because the tail is rarely where the assertion
# is and every later step pays to re-read it. CI uploads each tier log as an
# artifact, so the detail is one download away; at a terminal,
# `OUTPUT_BUDGET_FAIL_TAIL=40 chore test` restores the old behaviour for one
# run.
set -euo pipefail

REPO="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"

# WHERE THE WRAPPER COMES FROM, AND WHY NOT FROM HERE.
#
# `scripts/output-budget.sh` used to be a committed copy of somebody else's
# script. A committed copy is a copy that drifts: the family had several,
# reached four different ways, each repository internally consistent and
# nothing comparing them. The canonical copy lives in rust-fs-core and is
# resolved at RUNTIME, so there is exactly one of it.
#
# THE ORDER IS SIBLING FIRST, CARGO SECOND, AND THE FIRST ONE IS THE ONE THAT
# MATTERS HERE. This suite runs on windows-latest under Git Bash, and a path
# out of `cargo metadata` on that runner is a Windows path -- `C:\Users\...`
# -- which Git Bash cannot `[ -f ]` or `cp`. The sibling path is POSIX on
# every runner because it is built from this script's own location. Reading
# the sibling first also means a coordinated local change to the wrapper is
# actually exercised rather than shadowed by a registry copy.
#
# FS_CORE_ROOT OVERRIDES BOTH, AND IS AUTHORITATIVE. When it is set, that
# directory is the ONLY place looked in: an override that silently falls
# through to something else is not an override, and the tests that prove this
# resolver refuses a missing or wrong core depend on there being no second
# chance. ci.yml sets it, because this crate's am-fs-core is pinned to a
# release older than the one that carries the wrapper -- see the comment
# beside the checkout there.
#
# NO CHECKSUM IS PINNED. A digest recorded in seven repositories has to be
# updated in seven repositories every time core touches the script, which is
# exactly the lockstep this migration removes. The contract is the API string
# the script prints for `--version`; a copy that answers it is the copy we
# asked for, and a copy that does not is FATAL rather than something to fall
# back from.
SCRIPT_REL="scripts/output-budget.sh"
EXPECTED_API="rust-fs-core-output-budget 1"
MIN_CORE_VERSION="v0.2.13"
SIBLING_ROOT="$REPO/../rust-fs-core"

die() {
    echo "tier.sh: $*" >&2
    echo "         The output budget wrapper is rust-fs-core's, at $SCRIPT_REL." >&2
    echo "         Expected: bash \$core/$SCRIPT_REL --version  ->  $EXPECTED_API" >&2
    echo "         Sibling looked for: $SIBLING_ROOT/$SCRIPT_REL" >&2
    echo "         Minimum rust-fs-core release carrying it: $MIN_CORE_VERSION." >&2
    echo "         Set FS_CORE_ROOT to a checkout of it, or check one out beside this one." >&2
    exit 1
}

# A copy is accepted on its answer to --version and nothing else.
insist_canonical() {
    local path="$1" found
    [ -f "$path" ] || die "$path does not exist."
    found="$(bash "$path" --version 2>/dev/null || true)"
    [ "$found" = "$EXPECTED_API" ] || \
        die "$path answered --version with '$found', not '$EXPECTED_API'."
}

if [ -n "${FS_CORE_ROOT:-}" ]; then
    # A RELATIVE FS_CORE_ROOT IS RELATIVE TO THIS REPOSITORY, not to whatever
    # directory the caller happened to be in, so `bash scripts/tier.sh` means
    # the same thing from anywhere -- which is already true of the sibling
    # path below, because that is built from this script's own location.
    case "$FS_CORE_ROOT" in
        /*) ;;
        *) FS_CORE_ROOT="$REPO/$FS_CORE_ROOT" ;;
    esac
    SOURCE="$FS_CORE_ROOT/$SCRIPT_REL"
    insist_canonical "$SOURCE"
elif [ -f "$SIBLING_ROOT/$SCRIPT_REL" ]; then
    SOURCE="$SIBLING_ROOT/$SCRIPT_REL"
    insist_canonical "$SOURCE"
else
    # THE REGISTRY COPY, for a checkout with no sibling beside it. Cargo has
    # already resolved am-fs-core, so it is the thing that knows where the
    # package was unpacked; nothing here guesses at CARGO_HOME's layout.
    # This branch is not reached on windows-latest -- the sibling above wins
    # there -- which is why it may depend on python3 for the JSON.
    command -v python3 >/dev/null 2>&1 || \
        die "no sibling rust-fs-core, and python3 is needed to read cargo metadata."
    # `|| true` on both halves: a cargo that refuses -- an out-of-date
    # Cargo.lock under --locked is the usual reason -- must reach the message
    # below rather than killing this script with cargo's own status and no
    # explanation.
    METADATA="$(cargo metadata --format-version 1 --locked \
        --manifest-path "$REPO/Cargo.toml" 2>/dev/null || true)"
    CORE_DIR="$(printf '%s' "$METADATA" | python3 -c '
import json, sys
try:
    packages = json.load(sys.stdin)["packages"]
except Exception:
    sys.exit(0)
print(next((p["manifest_path"].rsplit("/", 1)[0]
            for p in packages if p["name"] == "am-fs-core"), ""))
' || true)"
    [ -n "$CORE_DIR" ] || die "cargo could not say where am-fs-core is."
    SOURCE="$CORE_DIR/$SCRIPT_REL"
    insist_canonical "$SOURCE"
fi

# THE RUN GETS ITS OWN COPY, AND GIVES IT BACK. Core is a checkout somebody
# else may be moving while this runs; a private copy means a tier cannot have
# the script changed underneath it half way through. tmp/ is gitignored and is
# where the tier logs already live. $$ keeps two concurrent tiers apart.
mkdir -p "$REPO/tmp"
BUDGET_REL="tmp/output-budget.$$.sh"
BUDGET="$REPO/$BUDGET_REL"
cp "$SOURCE" "$BUDGET"
trap 'rm -f "$BUDGET"' EXIT

# The tier's own command can see which copy it was given. It is a RELATIVE
# name because the one thing that must not appear in an environment variable
# on windows-latest is an absolute path: Git Bash's `pwd` answers `/d/a/...`
# and the Windows tools in the same run answer `D:\a\...`. Relative to the
# repository root, which is where every tier is invoked from, both agree.
# tests/output_budget.rs uses it to prove the copy is there during the run and
# gone afterwards without racing a concurrent tier's copy.
export TIER_OUTPUT_BUDGET="$BUDGET_REL"

[ $# -ge 5 ] || { echo "tier.sh: usage: tier.sh LABEL LOG MAX-LINES MAX-BYTES -- CMD..." >&2; exit 2; }
LABEL="$1"; LOG_NAME="$2"; MAX_LINES="$3"; MAX_BYTES="$4"; shift 4
[ "${1:-}" = "--" ] && shift
[ $# -gt 0 ] || { echo "tier.sh: no command" >&2; exit 2; }

# `chore test:debug -- --verbose` arrives as CLI_ARGS. output-budget.sh reads
# OUTPUT_BUDGET_VERBOSE itself, so mapping the flag onto it is all that is
# needed -- and it means the environment variable and the flag cannot
# disagree.
case " ${CLI_ARGS:-} " in
    *" --verbose "*|*" -v "*) export OUTPUT_BUDGET_VERBOSE=1 ;;
esac

# `bash "$BUDGET"` rather than running it directly: a Windows checkout arrives
# without the executable bit, and this suite runs on windows-latest. It is not
# `exec`ed either -- that would replace this shell and the EXIT trap above
# would never fire, leaving the copy behind. `set -e` hands the command's own
# status on, which is the status this script must exit with.
bash "$BUDGET" \
    --log "$REPO/tmp/logs/$LOG_NAME.log" \
    --max-lines "$MAX_LINES" \
    --max-bytes "$MAX_BYTES" \
    --label "$LABEL" \
    -- "$@"
