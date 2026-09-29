# `write --offset` puts exactly the bytes on stdin at exactly that offset,
# into every kind of cluster -- allocated, never written, zero-flagged,
# compressed, shared with an internal snapshot, still in the backing file --
# and the oracle agrees: after each image's writes `qemu-img check` finds no
# errors and no leaks, and `qemu-img convert -O raw` is the image as it was
# with those bytes laid over it by dd, and nothing else changed. The
# snapshot still holds what it held, and the backing file is untouched.
source "$(dirname "$0")/lib.sh"
source "$(dirname "$0")/images.sh"

cd "$SANDBOX" || exit 1
make_images

# overlay FILE OFFSET INPUT: INPUT's bytes over FILE at OFFSET.
overlay() {
    dd if="$3" of="$1" bs=1 seek="$2" conv=notrunc 2>/dev/null
}

# poke FILE OFFSET BYTE: overwrite one byte.
poke() {
    printf "\\$(printf '%03o' "$3")" | dd of="$1" bs=1 seek="$2" conv=notrunc 2>/dev/null
}

random() {
    head -c "$2" /dev/urandom >"$1"
}

# The writes, as OFFSET LENGTH: inside the first written run (an allocated
# cluster), across the 64 KiB cluster boundary, inside the zero cluster at
# 2 MiB, in the never-written 3 MiB, and the last byte of the disk.
WRITES="4100 700
$((64 * 1024 - 300)) 5000
$((2 * MiB + 1000)) 3000
$((3 * MiB + 17)) 9000
$((SIZE - 1)) 1"

cp base.qcow2 base.before
for img in v3.qcow2 v2.qcow2 small.qcow2 zlib.qcow2 zstd.qcow2 child.qcow2 snap.qcow2; do
    qemu-img convert -f qcow2 -O raw "$img" "$img.want"
    i=0
    while read -r offset length; do
        i=$((i + 1))
        random "$img.in$i" "$length"
        if [ $((i % 2)) -eq 0 ]; then
            cat "$img.in$i" | img.qcow2 "$img" write --offset "$offset" >"$img.w$i.json"
            check "$img: a piped write of $length at $offset exits 0" test "${PIPESTATUS[1]}" -eq 0
        else
            img.qcow2 "$img" write --offset "$offset" <"$img.in$i" >"$img.w$i.json"
            check "$img: a write of $length at $offset exits 0" test $? -eq 0
        fi
        jq_check "$img: write reports its offset and count" \
            ".offset == $offset and .bytes == $length" "$img.w$i.json"
        overlay "$img.want" "$offset" "$img.in$i"
        img.qcow2 "$img" read --offset "$offset" --length "$length" >"$img.back"
        same "$img: read at $offset returns what was written" "$img.back" "$img.in$i"
    done <<<"$WRITES"

    qemu-img check -f qcow2 "$img" >"$img.check" 2>&1
    check "$img: qemu-img check finds no errors and no leaks after our writes: $(tr '\n' ' ' <"$img.check" | head -c 300)" \
        test "${PIPESTATUS[0]}" -eq 0
    qemu-img convert -f qcow2 -O raw "$img" "$img.qemu.raw"
    same "$img: qemu-img reads exactly the bytes written, where written, and nothing else changed" \
        "$img.qemu.raw" "$img.want"
    img.qcow2 "$img" read >"$img.ours.raw"
    same "$img: our whole-disk read agrees with qemu-img" "$img.ours.raw" "$img.want"
done

# The snapshot still holds the disk as it was when it was taken, which was
# v3.qcow2 before any of this file's writes.
qemu-img create -q -f qcow2 fresh.qcow2 "$SIZE"
fill fresh.qcow2
qemu-img convert -f qcow2 -O raw fresh.qcow2 fresh.raw
qemu-img convert -f qcow2 -O raw -l snapshot.name=before snap.qcow2 snap-before.raw
same "the snapshot still holds what it held before the writes" snap-before.raw fresh.raw
same "the child's writes never reached its backing file" base.qcow2 base.before

# Nothing on stdin writes nothing.
img.qcow2 v3.qcow2 write --offset 0 </dev/null >empty.json
jq_check "an empty write reports 0 bytes" '.bytes == 0' empty.json

# Input that would run past the end is refused before anything is written,
# from a file and from a pipe; so is an offset past the end, and the image
# as its own input.
cp v3.qcow2 before.qcow2
random past.bin 1024
expect_error "a file running past the end" 1 img.qcow2 v3.qcow2 write --offset $((SIZE - 512)) <past.bin
cat past.bin | img.qcow2 v3.qcow2 write --offset $((SIZE - 512)) >/dev/null 2>past.json
check "a pipe running past the end exits 1" test "${PIPESTATUS[1]}" -eq 1
expect_error "an offset past the end" 1 img.qcow2 v3.qcow2 write --offset $((SIZE + 1)) </dev/null
expect_error "the image as its own input" 1 img.qcow2 v3.qcow2 write --offset 0 <v3.qcow2
same "no refused write changed the image" v3.qcow2 before.qcow2
expect_error "write without --offset" 2 img.qcow2 v3.qcow2 write </dev/null

# An image flagged dirty or corrupt (incompatible feature bits 0 and 1, the
# last byte of the big-endian field at 72) is read but not written: the
# library does not repair refcounts, and says so.
for flag in "dirty 1" "corrupt 2"; do
    set -- $flag
    qemu-img create -q -f qcow2 "$1.qcow2" 8M
    poke "$1.qcow2" 79 "$2"
    cp "$1.qcow2" "$1.before"
    printf 'x' | img.qcow2 "$1.qcow2" write --offset 0 >/dev/null 2>"$1.json"
    check "a $1 image refuses the write with exit 3" test "${PIPESTATUS[1]}" -eq 3
    jq_check "a $1 image's refusal says not implemented, and why" \
        "(.error | startswith(\"not implemented\")) and (.error | test(\"$1\"))" "$1.json"
    same "the refused $1 image is as it was" "$1.qcow2" "$1.before"
    jq_check "a $1 image still reports itself" '.format == "qcow2"' <(img.qcow2 "$1.qcow2" info)
done

finish
