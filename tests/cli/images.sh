# tests/cli/images.sh -- the images the suite reads, every one made by the
# oracle: this library has no creator, and an image of our own making would
# check our reader against our writer's reading of the format.
#
# make_images builds, in the working directory, and lists in $IMAGES:
#
#   v3.qcow2         version 3, 64 KiB clusters, patterns across a cluster
#                    boundary, a zero cluster (`write -z`), a cluster never
#                    written, and the last sector
#   v2.qcow2         the same writes in a version 2 image (`compat=0.10`),
#                    which has no zero flag, so `write -z` allocates zeros
#   small.qcow2      version 3 with 4 KiB clusters
#   zlib.qcow2       v3.qcow2 compressed (`convert -c`), zlib
#   zstd.qcow2       v3.qcow2 compressed with `compression_type=zstd`
#   child.qcow2      a child of base.qcow2 (`-b base.qcow2 -F qcow2`) with
#                    its own writes over part of the base's
#   snap.qcow2       v3.qcow2 with an internal snapshot, then written after it
#
# Sourced by the test files that need them; lib.sh has already been.

MiB=1048576
SIZE=$((8 * MiB))

# fill IMAGE: the shared pattern of writes.
fill() {
    qemu-io -f qcow2 \
        -c "write -P 0x5a 4096 8192" \
        -c "write -P 0xc3 $((64 * 1024 - 1000)) 3000" \
        -c "write -P 0x3c $((1 * MiB)) $((256 * 1024))" \
        -c "write -z $((2 * MiB)) $((128 * 1024))" \
        -c "write -P 0x7e $((SIZE - 512)) 512" \
        "$1" >/dev/null
}

make_images() {
    qemu-img create -q -f qcow2 v3.qcow2 "$SIZE"
    fill v3.qcow2
    qemu-img create -q -f qcow2 -o compat=0.10 v2.qcow2 "$SIZE"
    fill v2.qcow2
    qemu-img create -q -f qcow2 -o cluster_size=4096 small.qcow2 "$SIZE"
    fill small.qcow2
    qemu-img convert -c -f qcow2 -O qcow2 v3.qcow2 zlib.qcow2
    qemu-img convert -c -f qcow2 -O qcow2 -o compression_type=zstd v3.qcow2 zstd.qcow2
    qemu-img create -q -f qcow2 base.qcow2 "$SIZE"
    fill base.qcow2
    qemu-img create -q -f qcow2 -b base.qcow2 -F qcow2 child.qcow2
    qemu-io -f qcow2 -c "write -P 0xee $((1 * MiB + 4096)) 70000" -c "write -P 0xdd $((5 * MiB)) 512" \
        child.qcow2 >/dev/null
    cp v3.qcow2 snap.qcow2
    qemu-img snapshot -c before snap.qcow2
    qemu-io -f qcow2 -c "write -P 0x99 4096 512" snap.qcow2 >/dev/null
    IMAGES="v3.qcow2 v2.qcow2 small.qcow2 zlib.qcow2 zstd.qcow2 base.qcow2 child.qcow2 snap.qcow2"
}
