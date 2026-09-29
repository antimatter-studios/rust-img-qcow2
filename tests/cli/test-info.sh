# `info` and `get` on images the oracle made: every canonical key, with the
# right type, and the values `qemu-img info` reports for the same file --
# virtual size, cluster size, version, backing file, compression type,
# refcount width, the dirty and corrupt flags, the snapshot count.
source "$(dirname "$0")/lib.sh"
source "$(dirname "$0")/images.sh"

cd "$SANDBOX" || exit 1
make_images

for img in $IMAGES; do
    img.qcow2 "$img" info >"$img.json"
    check "info on $img exits 0" test $? -eq 0
    qemu-img info -f qcow2 --output=json "$img" >"$img.qemu.json"

    jq_check "$img: the canonical keys, in order" \
        '[keys_unsorted[]] == ["format","virtual_size","block_size","backing","dirty","qcow2"]' "$img.json"
    jq_check "$img: format is qcow2" '.format == "qcow2"' "$img.json"
    jq_check "$img: the sizes are numbers" \
        '[.virtual_size, .block_size, .qcow2.version, .qcow2.l1_size, .qcow2.snapshot_count] | all(type == "number")' "$img.json"
    jq_check "$img: dirty and corrupt are booleans" \
        '[.dirty, .qcow2.corrupt, .qcow2.encrypted] | all(type == "boolean")' "$img.json"
    jq_check "$img: backing is a string or null" '.backing | type == "string" or . == null' "$img.json"

    # The oracle's view of the same file.
    jq_check "$img: virtual_size is qemu-img's" \
        --slurpfile q "$img.qemu.json" '.virtual_size == $q[0]."virtual-size"' "$img.json"
    jq_check "$img: block_size is qemu-img's cluster size" \
        --slurpfile q "$img.qemu.json" '.block_size == $q[0]."cluster-size" and .qcow2.cluster_size == .block_size' "$img.json"
    jq_check "$img: the version is qemu-img's compat level" \
        --slurpfile q "$img.qemu.json" \
        '.qcow2.version == ({"0.10": 2, "1.1": 3}[$q[0]."format-specific".data.compat])' "$img.json"
    jq_check "$img: backing is qemu-img's backing-filename" \
        --slurpfile q "$img.qemu.json" '.backing == $q[0]."backing-filename"' "$img.json"
    jq_check "$img: refcount_bits is qemu-img's" \
        --slurpfile q "$img.qemu.json" '.qcow2.refcount_bits == $q[0]."format-specific".data."refcount-bits"' "$img.json"
    jq_check "$img: corrupt is qemu-img's" \
        --slurpfile q "$img.qemu.json" '.qcow2.corrupt == ($q[0]."format-specific".data.corrupt // false)' "$img.json"
    jq_check "$img: dirty is qemu-img's dirty-flag" \
        --slurpfile q "$img.qemu.json" '.dirty == $q[0]."dirty-flag"' "$img.json"
    jq_check "$img: the snapshot count is qemu-img's" \
        --slurpfile q "$img.qemu.json" '.qcow2.snapshot_count == ($q[0].snapshots // [] | length)' "$img.json"
    # A version 2 image has no compression type field, and qemu-img names
    # none; it is zlib, the only one version 2 knows.
    jq_check "$img: the compression type is qemu-img's" \
        --slurpfile q "$img.qemu.json" \
        '.qcow2.compression_type == ($q[0]."format-specific".data."compression-type" // "zlib")' "$img.json"
done

jq_check "zstd.qcow2 is zstd" '.qcow2.compression_type == "zstd"' zstd.qcow2.json
jq_check "v2.qcow2 is version 2" '.qcow2.version == 2' v2.qcow2.json
jq_check "small.qcow2 has 4 KiB clusters" '.block_size == 4096' small.qcow2.json
jq_check "child.qcow2 names base.qcow2" '.backing == "base.qcow2"' child.qcow2.json
jq_check "snap.qcow2 holds one snapshot" '.qcow2.snapshot_count == 1' snap.qcow2.json

# get KEY answers one key, as an object; --text answers the bare value.
got="$(img.qcow2 v3.qcow2 get virtual_size --text)"
check "get virtual_size --text is $SIZE (got '$got')" test "$got" = "$SIZE"
img.qcow2 zstd.qcow2 get qcow2.compression_type >key.json
jq_check "get qcow2.compression_type is one key" \
    'keys == ["qcow2.compression_type"] and .["qcow2.compression_type"] == "zstd"' key.json

# info and get are the same verb.
img.qcow2 v3.qcow2 get >get.json
img.qcow2 v3.qcow2 info >info.json
same "get and info report the same thing" get.json info.json

# --text is key: value lines, nested keys dotted.
img.qcow2 v3.qcow2 info --text >info.txt
check "info --text carries format: qcow2" grep -qx 'format: qcow2' info.txt
check "info --text dots the nested keys" grep -qx 'qcow2.version: 3' info.txt

expect_error "get of an unknown key" 2 img.qcow2 v3.qcow2 get no_such_key

finish
