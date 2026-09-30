# Working in rust-img-qcow2 (agent guide)

Pure-Rust QCOW2 reader and writer, validated against `qemu-img` — qemu being the format's reference implementation. This file is the fast path
for an agent picking up work here, so the workflow does not have to be
re-derived each time. It points at the existing docs rather than duplicating
them:

- **README** → what the crate does, how it is built, and what does not work yet.
- **`chores.yml`** → every task named below, and what each one actually runs.
- **`.github-guard`** → what must pass before `main` takes a merge.

The section between the BEGIN/END markers below is **shared, byte-identical,
with every repository in this family**. Do not edit it here: change the
canonical copy and propagate it, or `scripts/agents-core-check.sh` will fail.
Everything after the END marker is specific to this repository.

<!-- BEGIN SHARED BLOCK: agent-core v2 sha256:38af4d2c5377d38ab382baa4eab4aa679841e2b4eba4f4d01dacd255ffa7d32e -->
## Claiming work

Several agents work these repositories at the same time. Before you start on
an issue, claim it, so nobody else spends a session on what you are already
doing. The lock is a **GitHub label**, because labels are shared state that
every agent can read and change without posting comments into the thread.

**Before starting.** Check, claim, then read back:

```sh
gh issue view <N> --json labels                      # holds `claimed`? pick another
gh issue edit <N> --add-label claimed --add-label claim/<session>
gh issue view <N> --json labels                      # read back and confirm
```

`<session>` is your session name — `agent-<random4>-<isodate>`, e.g.
`agent-3f7c-2026-09-22`. Create the `claim/<session>` label if it does not
exist.

**Resolving a race.** Adding a label is not compare-and-swap: two agents can
both add `claimed` and both believe they won. That is what the read-back is
for. If it shows more than one `claim/*` label, the **lexically lowest**
session keeps the issue; every other agent removes its own `claim/*` label and
picks different work. Each racer computes the same answer independently, so no
further coordination is needed.

**When you finish or stop.** Remove both labels — on merge, or the moment you
abandon the work:

```sh
gh issue edit <N> --remove-label claimed --remove-label claim/<session>
```

Delete your `claim/<session>` label from the repository at the end of your
session so they do not accumulate.

**Reclaiming a stale claim.** An agent that dies holding a claim would block an
issue forever. If `claimed` was applied more than 12 hours ago and the holder's
branch has no commits since, any agent may take it: remove the stale `claim/*`,
add your own, and say so in the issue.

**This is a convention, not a fence.** Nothing enforces it. An agent that
ignores it duplicates work; it cannot corrupt anything. Honour it anyway.

## Work in a worktree

Every working copy is a **git worktree** of an existing checkout, made with
`git worktree add`. Never `git clone` a second, unlinked copy — not for a
branch, a PR, a review, or a sibling you need at another ref:

```sh
git -C <checkout> fetch origin
git -C <checkout> worktree add <path> -b <type>/<name> origin/main   # new work
git -C <checkout> worktree add --detach <path> <tag>                 # a sibling at a pinned ref
git -C <checkout> worktree remove <path>                             # when done
```

A worktree shares the checkout's objects and remotes, and `git worktree list`
shows it to every agent on the machine, so nobody else mistakes it for
abandoned work or loses track of it. An unlinked clone copies all the history
again, is invisible to that list, and gets left behind in `/tmp` long after the
work that made it is merged. Remove your worktree when you finish.

## Skills to use

- **`dev-loop`** — the required loop for any non-trivial change: baseline the
  full suite → change → re-run (no baseline test may regress) → enhance tests →
  vet. Always run it.
- **`commit`** / **`pr`** — for grouping commits and opening pull requests.

Each repository names any further skills of its own below.

## A bug fix starts with a red

**Prove it is broken first** — a failing check or test — *then* fix it, *then*
prove that same check is green, *then* confirm the full baseline still passes.
Never write the fix before you have a red. A fix with no failing test to its
name is a claim, not a result.

## Nothing skips

A test that cannot run **fails**, naming the task that would provide what it
needed. Never add an early return for a missing fixture, tool or VM: a skipped
test reads exactly like a passing one, and a suite that quietly declines to run
is indistinguishable from a suite that passes.

Where a tier reports skips or ignored tests, that is a gate, not a note.

## Validate against something that is not us

A driver's own readers share its interpretation of the format, so they cannot
catch a misreading: the mistake is baked into the fixture *and* the parser, and
they agree with each other while disagreeing with every real filesystem. Unit
tests over self-built fixtures prove self-consistency, not correctness.

Every structure that is parsed or written gets a cross-validation test against
an **independent oracle** — the platform's own tools, a real kernel, or a third
implementation — before it is considered done. Each repository names its
oracles below.

## Output is budgeted

Test tiers run through `scripts/tier.sh`, which runs the suite **quietly**: the
whole run goes to `tmp/logs/<tier>.log`, a pass prints one verdict line naming
that log, and a failure prints the verdict, the command's status and the log's
path — `--tail N`, or `OUTPUT_BUDGET_FAIL_TAIL=N`, prints the tail for whoever
is watching. **Read the log**: a failing tier names it and does not recite it.
CI keeps the logs as an artifact, so the detail is always retrievable.

The budget caps the log, not merely what is shown, and every number in the
table was measured. A run that passes but prints more than its budget **fails**.

The reader who pays most for a noisy suite is an agent that re-reads its whole
transcript on every step, and so pays for one loud run many times over. If a
tier legitimately grows, raise its row **with the measurement that justifies
it**. Do not silence output to fit, and do not route around `tier.sh`.

## Commits and branches

- Branches are `<type>/<name>`, matching the commit type: `fix/`, `feat/`,
  `ci/`, `docs/`, `chore/`, `test/`.
- A commit is a subject plus flat one-sentence bullets. Subjects are
  declarative, not imperative: "the run-end bound is checked", not "check the
  run-end bound".
- **No AI attribution and no co-author trailers**, in commits or in pull
  request descriptions.
- `main` takes **squash merges only**.

## Project rules

- **No GPL/LGPL/AGPL dependencies.** Permissive only (MIT/BSD/Apache).
  Shelling out to a copyleft CLI as a *test oracle* is fine — linking or
  copying it is not.
- **Each of these is a standalone project.** Never mention a consuming
  application in the README, the source, or CLI help.
<!-- END SHARED BLOCK: agent-core v2 -->
## What this is

Pure-Rust QCOW2 reader and writer over `am-fs-core`, linked into the app as a
staticlib.

## Running tests

```sh
chore test          # the suite
chore testqemu      # against qemu-img
chore testrelease   # release profile
chore testlib32     # the 32-bit build
chore lint          # fmt, the agent-core check, clippy
chore staticlib     # what the app links
```

CI runs `test`, `test-32bit`, `test-release`, `qemu-validation`, `fmt`,
aggregated by `ci-ok`.

**`test-32bit` is not decoration.** QCOW2 offsets are 64-bit on disk; a
`usize` that is 32 bits wide truncates them, and no 64-bit job can see it.

## The oracle is qemu-img

QCOW2 is qemu's own format, so the oracle is the reference implementation. An
image this crate writes must be one `qemu-img` reads identically. If `qemu-img`
is missing the job **fails**; it does not skip.

## L1/L2 entries are packed

An L1/L2 entry carries **COPIED (bit 63)** and **COMPRESSED (bit 62)** above the
host offset. `OFFSET_MASK` (`src/reader.rs:37`) is applied at every lookup —
`:1085`, `:1238`, `:1253`. Masking at a new lookup site you forgot is how a flag
bit gets read as part of an address.

## A feature-gated test target runs only with its feature

`tests/feature_gated_targets.rs` refuses a feature gate the scan cannot read,
rather than skipping it (#50). A suite that silently selects nothing is
indistinguishable from a suite that passes.

## The pin that could not be bumped, and how it was

This crate depends on `am-fs-core` at **`v0.2.14`**. It sat at `v0.2.10` for a
while, deliberately, and the reasoning is kept because a spent caution that
reads as a live one is how a repository ends up several releases behind without
anyone deciding to be.

`4e19fc9` (rust-fs-core#75) made a write past the end of a `FileDevice` a
refusal rather than an implicit extension. It was right to — `size_bytes()`
reported the construction-time length while the file grew underneath it, so
`CachingDevice` could serve bytes no cached read could reach (#70). But writing
past the end was **the only way this format allocated**: append a cluster, then
record where it went. Measured against core `main` at the time: vhd 7 failures,
qcow2 3, vhdx 1, vmdk 1; zero against `v0.2.10`.

The replacement is `BlockDevice::set_len` plus `can_grow()`, released in
`v0.2.12`, and #113 adopted it here. `Qcow2Reader::dev_grow_for_cluster` asks
the device for room before the allocator hands a cluster out, at both of
`allocate_cluster`'s return paths. It only ever grows — `set_len` *sets*, and a
smaller length would truncate — so a writable but fixed-length device keeps
working for every allocation that fits inside it.

**It is not in `dev_write`, on purpose.** That funnel is also the in-place and
metadata path, and growing there would extend the file on any stray offset,
which dissolves the bounds check #70 exists to provide. Only the allocator knows
a cluster is supposed to be new.

Two things went with it:

- **The second checkout of core is gone (#114).** `scripts/tier.sh` reads
  rust-fs-core's `scripts/output-budget.sh` at runtime — this repository does
  not carry a copy — and that needed `v0.2.13`, while the dependency was held at
  `v0.2.10`. Two lower bounds meant two clones of the same repository at two
  refs. The higher pin satisfies both, so `FS_CORE_ROOT` points at
  `../rust-fs-core` and there is one checkout again.
- **A wrapper device has to forward growth.** `set_len` and `can_grow` are
  *defaulted* on `BlockDevice`, to `Err(ReadOnly)` and `false`. A passthrough
  that omits them looks writable and refuses to grow, and every allocating test
  through it fails with a bare `ReadOnly` — which is what `CountingWrites` in
  `tests/synthetic.rs` did until it forwarded both.

`Qcow2Reader`'s own `BlockDevice` impl still answers `can_grow() == false`, and
that is a decision rather than an omission: the guest-visible length there is
the header's `size` field, and changing it is resizing the virtual disk, not
letting the host file get longer.

One practical note that survives all of this: `pre-commit.d/rust-clippy.sh`
runs clippy without `--locked`, so a `../rust-fs-core` checkout that is
semver-ahead of the pin rewrites your unstaged `Cargo.lock` and
`rust-deps-pinned.sh` then blocks the commit over a file the commit never
contained (agent-skills#64). The answer is to move the pin or restore the lock,
not `--no-verify`, which disables every guard at once.

## What gates a merge

One required check, `ci-ok`, declared in `.github-guard` and aggregating every
job in `ci.yml`. `fuzz.yml` (nightly cron plus dispatch) and `release.yml`
(tag-driven) never report on a pull request and must never be required.

`chore check:ci-gate` holds both halves of that mechanically — every job in
`ci.yml` must appear in `ci-ok`'s `needs:`, and `.github-guard` must require
`ci-ok` and nothing else. The task names `scripts/ci-gate.sh` and nothing else,
so the script is what can be tested, reviewed and run without `chore` at all.
It replaced `tests/ci_aggregate_gate.rs`: that parsed a YAML file and compared
strings, exercising nothing this crate ships, and as a `cargo test` it counted
towards the executed-test floor the gate itself enforces.

Judging mergeability from check **conclusions** is unreliable: an in-progress
`CheckRun` reports its conclusion as an empty string, and a `StatusContext` has
no conclusion field at all. Read `mergeStateStatus` and
`statusCheckRollup.state`.

## Never grow a shared tool to solve a problem here

**Never grow a shared tool to solve a problem in this repository.** `chore` is
a general-purpose task runner this project merely consumes; the same goes for
`github-guard` and the agent-skills hooks. If something needed here looks like
it belongs inside one of them, it does not. Solve it here, or ask first. The
tell is a release: if a shared tool needs a new version cut whose only purpose
is to unblock this project, the code is in the wrong repository.
