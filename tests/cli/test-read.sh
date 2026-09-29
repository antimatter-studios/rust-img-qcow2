# `read` on images the oracle made: the whole disk is byte-identical to
# `qemu-img convert -O raw` for every image -- compressed ones and a backing
# chain included -- and every range is the same slice of that raw image:
# straddling a cluster boundary, inside a zero cluster, inside a cluster
# never written, through to the backing file, the last byte.
source "$(dirname "$0")/lib.sh"
source "$(dirname "$0")/images.sh"

cd "$SANDBOX" || exit 1
make_images

# slice FILE OFFSET LENGTH: those bytes of FILE, on stdout.
slice() {
    dd if="$1" bs=1 skip="$2" count="$3" 2>/dev/null
}

for img in $IMAGES; do
    qemu-img convert -f qcow2 -O raw "$img" "$img.qemu.raw"
    img.qcow2 "$img" read >"$img.ours.raw"
    check "read of the whole $img exits 0" test $? -eq 0
    same "the whole of $img, read, is qemu-img's raw image" "$img.ours.raw" "$img.qemu.raw"

    for range in "4096 8192" "$((64 * 1024 - 1000)) 3000" "$((2 * MiB + 1000)) 4096" \
        "$((3 * MiB)) 65536" "$((1 * MiB + 4000)) 70200" "$((5 * MiB)) 512" \
        "$((SIZE - 1)) 1" "$((SIZE - 512)) 512"; do
        set -- $range
        img.qcow2 "$img" read --offset "$1" --length "$2" >"$img.range"
        slice "$img.qemu.raw" "$1" "$2" >"$img.want"
        same "$img: read --offset $1 --length $2" "$img.range" "$img.want"
    done
done

img.qcow2 zstd.qcow2 read -o zstd.o.raw
same "read -o writes the same bytes as read to stdout" zstd.o.raw zstd.qcow2.qemu.raw
check "read -o leaves no .partial file" test ! -e zstd.o.raw.partial

# --offset alone reads to the end; --length alone reads from 0.
img.qcow2 v3.qcow2 read --offset $((SIZE - 512)) >tail.bin
slice v3.qcow2.qemu.raw $((SIZE - 512)) 512 >want.bin
same "read --offset alone reads to the end" tail.bin want.bin
img.qcow2 v3.qcow2 read --length 16K >head.bin
slice v3.qcow2.qemu.raw 0 16384 >want.bin
same "read --length alone reads from 0, and takes a suffix" head.bin want.bin

# A range past the end is refused whole, before a byte is written.
expect_error "a range past the end" 1 img.qcow2 v3.qcow2 read --offset $((SIZE - 512)) --length 513
expect_error "an offset past the end" 1 img.qcow2 v3.qcow2 read --offset $((SIZE + 1))

# The child cannot be read without its backing file, and says so.
mv base.qcow2 base.moved
expect_error "a child whose backing file is gone" 1 img.qcow2 child.qcow2 read
mv base.moved base.qcow2

# A closed pipe is the reader's choice, not a failure.
img.qcow2 v3.qcow2 read 2>pipe.err | head -c 100 >/dev/null
check "read into a pipe closed early exits 0" test "${PIPESTATUS[0]}" -eq 0
check "read into a pipe closed early says nothing on stderr" test ! -s pipe.err

finish
