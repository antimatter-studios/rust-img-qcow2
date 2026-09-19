#!/usr/bin/env bash
# Rebuild fuzz/corpus from images qemu-img wrote.
#
# The seeds are qcow2 files the reference implementation produced, not
# bytes this crate wrote: a random byte string is refused by the magic
# on the first line of the header parser and never reaches the cluster
# arithmetic underneath, which is where every field of interest is used.
#
# One image per shape that changes the read path: cluster sizes either
# side of the default (the cluster size is a shift, and everything is
# multiplied and divided by it), a version 2 file, a compressed one, one with a
# backing file, and one with enough data to need more than a single L2
# table.
#
# Usage: scripts/make-fuzz-corpus.sh
set -euo pipefail

here="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
work="$(mktemp -d "${TMPDIR:-/tmp}/qcow2-fuzz-corpus.XXXXXX")"
trap 'rm -rf "$work"' EXIT

command -v qemu-img >/dev/null || {
    echo "qemu-img not found; install qemu-utils" >&2
    exit 1
}

rm -rf "$here/fuzz/corpus"
mkdir -p "$here/fuzz/corpus"/{image,header}

# Something to write into the images so the L1 and L2 tables have
# entries rather than being all-zero: a run of data, a hole, and more
# data, which is the shape that produces both allocated and unallocated
# clusters in one file.
payload="$work/payload"
python3 - "$payload" <<'PY'
import sys
with open(sys.argv[1], 'wb') as f:
    # Small: the committed corpus is mutated tens of thousands of
    # times per run, and a larger virtual disk buys no new structure --
    # two allocated runs either side of a hole is already an L1 entry,
    # two L2 tables' worth of entries, and both cluster states.
    f.write(b'A' * 8_000)
    f.seek(500_000)
    f.write(b'B' * 8_000)
PY

build() {
    local name="$1"; shift
    local img="$here/fuzz/corpus/image/$name.qcow2"
    qemu-img convert -f raw -O qcow2 "$@" "$payload" "$img" 2>/dev/null || {
        echo "qemu-img could not build the '$name' image" >&2
        exit 1
    }
}

# A large cluster size is built but NOT committed as an image. qcow2
# gives the refcount table and each level of the mapping tables a whole
# cluster of their own, so a 256 KiB cluster size makes a 1.8 MB file
# out of two data clusters -- most of a corpus, for one variant. Its
# header is worth having, since the cluster-size shift is what
# everything else is multiplied and divided by, so that is what gets
# kept.
header_only() {
    local name="$1"; shift
    qemu-img convert -f raw -O qcow2 "$@" "$payload" "$work/$name.qcow2" 2>/dev/null || {
        echo "qemu-img could not build the '$name' image" >&2
        exit 1
    }
    dd if="$work/$name.qcow2" of="$here/fuzz/corpus/header/$name.bin" \
        bs=2048 count=1 status=none
}

build default
build cluster-1k  -o cluster_size=1024
build v2          -o compat=0.10
build compressed  -c
header_only cluster-256k -o cluster_size=262144

# A backing file: the header carries an offset and a length for the
# backing file name, which is a length read out of the image and used
# to size a read.
# Relative, and created from inside the directory, so the committed
# file does not carry the absolute path of whoever last rebuilt it.
# Nothing resolves it in any case: the fuzz targets open a device, not
# a path, so a backing file is a name and a length in the header and
# never a second file -- which is the part worth fuzzing.
(cd "$here/fuzz/corpus/image" && qemu-img create -f qcow2 -b default.qcow2 -F qcow2 \
    backed.qcow2 >/dev/null 2>&1)

python3 - "$here/fuzz/corpus" <<'PY'
import os, struct, sys

root = sys.argv[1]
MAGIC = b'QFI\xfb'

for img_name in sorted(os.listdir(os.path.join(root, 'image'))):
    stem = img_name[:-len('.qcow2')]
    img = open(os.path.join(root, 'image', img_name), 'rb').read()
    assert img[:4] == MAGIC, f"{img_name}: not a qcow2 file"

    # The v3 header is 104 bytes before the extension area; taking a
    # whole cluster's worth covers the header extensions behind it --
    # the feature-name table and the backing-file format string are
    # both there, and both are length-prefixed.
    with open(os.path.join(root, 'header', f'{stem}.bin'), 'wb') as f:
        f.write(img[:2048])
PY

echo "corpus rebuilt under fuzz/corpus:"
find "$here/fuzz/corpus" -type f | sort | sed "s#$here/##"
echo "total: $(find "$here/fuzz/corpus" -type f | wc -l) seeds, $(du -sh "$here/fuzz/corpus" | cut -f1)"
