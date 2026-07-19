//! Stable C ABI for Project Panama (`Linker.downcallHandle`) and other FFIs.
//!
//! Symbol names are part of the public contract — do not rename without a
//! protocol version bump.
//!
//! ```text
//! javar_mem_alloc(size, align) -> region_id (0 = failure)
//! javar_mem_free(id) -> 1|0
//! javar_mem_ptr(id) -> *mut u8   // for MemorySegment.ofAddress (zero-copy)
//! javar_mem_len(id) -> usize
//! javar_mem_write / javar_mem_read
//! javar_mem_managed_bytes() -> u64
//! ```

use super::{global, RegionId};
use std::os::raw::{c_int, c_void};

/// Allocate an off-heap region. Returns region id, or `0` on failure.
#[no_mangle]
pub extern "C" fn javar_mem_alloc(size: usize, align: usize) -> u64 {
    match global().allocate(size, align) {
        Some(id) => id.0,
        None => 0,
    }
}

/// Free a region previously returned by [`javar_mem_alloc`].
#[no_mangle]
pub extern "C" fn javar_mem_free(id: u64) -> c_int {
    if global().free(RegionId(id)) {
        1
    } else {
        0
    }
}

/// Raw pointer for zero-copy `MemorySegment.ofAddress` / DirectByteBuffer.
/// Returns null if the id is invalid.
#[no_mangle]
pub extern "C" fn javar_mem_ptr(id: u64) -> *mut c_void {
    match global().ptr_len(RegionId(id)) {
        Some((ptr, _)) => ptr.cast(),
        None => std::ptr::null_mut(),
    }
}

/// Byte length of a region, or `0` if invalid.
#[no_mangle]
pub extern "C" fn javar_mem_len(id: u64) -> usize {
    global().len(RegionId(id)).unwrap_or(0)
}

/// Copy `len` bytes from `src` into the region at `offset`.
#[no_mangle]
pub unsafe extern "C" fn javar_mem_write(
    id: u64,
    offset: usize,
    src: *const u8,
    len: usize,
) -> c_int {
    if src.is_null() && len != 0 {
        return 0;
    }
    let data = if len == 0 {
        &[][..]
    } else {
        unsafe { std::slice::from_raw_parts(src, len) }
    };
    if global().write(RegionId(id), offset, data) {
        1
    } else {
        0
    }
}

/// Copy `len` bytes from the region at `offset` into `dst`.
#[no_mangle]
pub unsafe extern "C" fn javar_mem_read(
    id: u64,
    offset: usize,
    dst: *mut u8,
    len: usize,
) -> c_int {
    if dst.is_null() && len != 0 {
        return 0;
    }
    let data = if len == 0 {
        &mut [][..]
    } else {
        unsafe { std::slice::from_raw_parts_mut(dst, len) }
    };
    if global().read(RegionId(id), offset, data) {
        1
    } else {
        0
    }
}

/// Total bytes currently managed off-heap.
#[no_mangle]
pub extern "C" fn javar_mem_managed_bytes() -> u64 {
    global().managed_bytes()
}

/// ABI version for Panama/JNI capability negotiation (`1` = initial).
#[no_mangle]
pub extern "C" fn javar_mem_abi_version() -> u32 {
    1
}
