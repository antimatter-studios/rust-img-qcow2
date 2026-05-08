/*
 * am-img-qcow2 C ABI — opens a QCOW2 image and returns a generic
 * FsCoreDevice handle. Once opened, all further interaction goes
 * through fs_core.h's device API.
 *
 * Link with libam_img_qcow2.a and include this header alongside fs_core.h.
 *
 * MIT license. (c) 2026 Antimatter Studios.
 */

#ifndef AM_IMG_QCOW2_H
#define AM_IMG_QCOW2_H

#include "fs_core.h"

#ifdef __cplusplus
extern "C" {
#endif

/*
 * Open `path` (NUL-terminated UTF-8) as a QCOW2 image. Returns a generic
 * device handle; free via `fs_core_device_close`.
 *
 * On failure returns NULL and `fs_core_last_error_message()` has detail.
 *
 * Currently supported features (matching the Rust reader):
 *   - QCOW2 v2 and v3
 *   - Uncompressed and zlib-compressed clusters
 *   - Sparse / v3 zero-flagged clusters
 *   - Backing-file chains (recursive, depth-limited)
 *
 * `qcow2_open` opens read-only — `fs_core_device_write_at` returns
 * FS_CORE_READ_ONLY.
 *
 * `qcow2_open_rw` opens read-write under Phase A constraints: writes
 * succeed only against already-allocated, single-reference,
 * uncompressed clusters; everything else (allocation, decompress-rewrite,
 * snapshot CoW) returns FS_CORE_CUSTOM with detail in
 * fs_core_last_error_message().
 */
FsCoreDevice *qcow2_open(const char *path);
FsCoreDevice *qcow2_open_rw(const char *path);

#ifdef __cplusplus
} /* extern "C" */
#endif

#endif /* AM_IMG_QCOW2_H */
