# A verb the library cannot do still exists and says so: exit status 3, a
# structured error beginning `not implemented`, and the image untouched.
source "$(dirname "$0")/lib.sh"

cd "$SANDBOX" || exit 1
qemu-img create -q -f qcow2 disk.qcow2 8M
cp disk.qcow2 before.qcow2

# not_implemented DESCRIPTION COMMAND...
not_implemented() {
    local what="$1"
    shift
    expect_error "$what" 3 "$@"
    jq_check "$what: the error says not implemented" \
        '.error | startswith("not implemented")' "$SANDBOX/error.json"
}

not_implemented "resize" img.qcow2 disk.qcow2 resize 16M
not_implemented "set" img.qcow2 disk.qcow2 set backing base.qcow2
not_implemented "create" img.qcow2 new.qcow2 create 8M
same "no refused verb changed the image" disk.qcow2 before.qcow2
check "the refused create made no file" test ! -e new.qcow2

# --text turns the error into a line for a person, with the same status.
img.qcow2 disk.qcow2 resize 16M --text 2>resize.txt
check "resize --text exits 3" test $? -eq 3
check "resize --text says not implemented" grep -q 'img.qcow2: not implemented' resize.txt

finish
