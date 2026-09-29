# Damaged images fail with a structured error and nothing on stdout -- a
# bad magic, an L1 entry pointing past the end of the file, a truncated
# file -- and the oracle agrees each one is damaged: `qemu-img check`
# reports it, or refuses to open it.
source "$(dirname "$0")/lib.sh"

cd "$SANDBOX" || exit 1

# poke FILE OFFSET BYTE: overwrite one byte.
poke() {
    printf "\\$(printf '%03o' "$3")" | dd of="$1" bs=1 seek="$2" conv=notrunc 2>/dev/null
}
be64() {
    od -An -tx1 -j "$2" -N 8 "$1" | tr -d ' \n'
}

# qemu_rejects DESCRIPTION IMAGE: `qemu-img check` exits non-zero on it.
qemu_rejects() {
    if qemu-img check -f qcow2 "$2" >"$SANDBOX/check.out" 2>&1; then
        fail "$1: qemu-img check calls it clean: $(head -c 300 "$SANDBOX/check.out")"
    else
        ok
    fi
}

qemu-img create -q -f qcow2 good.qcow2 8M
qemu-io -f qcow2 -c "write -P 0x42 0 65536" good.qcow2 >/dev/null
check "qemu-img check calls the undamaged image clean" qemu-img check -q -f qcow2 good.qcow2

# The magic is not QFI\xfb.
cp good.qcow2 bad-magic.qcow2
poke bad-magic.qcow2 0 0
expect_error "a bad magic" 1 img.qcow2 bad-magic.qcow2 info
expect_error "a bad magic, read" 1 img.qcow2 bad-magic.qcow2 read
qemu_rejects "a bad magic" bad-magic.qcow2

# The first L1 entry points a long way past the end of the file.
l1="$((16#$(be64 good.qcow2 40)))"
cp good.qcow2 l1-past-end.qcow2
poke l1-past-end.qcow2 $((l1 + 1)) 0x7f
expect_error "an L1 entry past the end of the file" 1 img.qcow2 l1-past-end.qcow2 read --length 512
qemu_rejects "an L1 entry past the end of the file" l1-past-end.qcow2

# Cut short: the header survives, the tables and data do not.
head -c 1024 good.qcow2 >truncated.qcow2
expect_error "a truncated image" 1 img.qcow2 truncated.qcow2 read
qemu_rejects "a truncated image" truncated.qcow2

# Not an image at all, and an empty file.
head -c 65536 /dev/urandom >noise.bin
expect_error "a file of noise" 1 img.qcow2 noise.bin info
: >empty.qcow2
expect_error "an empty file" 1 img.qcow2 empty.qcow2 info
expect_error "a file that is not there" 1 img.qcow2 missing.qcow2 info

finish
