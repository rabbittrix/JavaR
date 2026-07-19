//! Off-heap memory managed by Rust — shared zero-copy with the JVM via
//! Project Panama (Java 22+) or JNI DirectByteBuffer (Java 8+).
//!
//! Heavy data structures live outside the Java heap. The JVM holds only an
//! opaque region id + a view (`MemorySegment` / `ByteBuffer`) into Rust memory.

mod ffi;
mod jni_api;

pub use ffi::*;

use dashmap::DashMap;
use parking_lot::Mutex;
use std::alloc::{alloc, dealloc, Layout};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::OnceLock;

/// Opaque handle returned to the JVM / agent.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[repr(transparent)]
pub struct RegionId(pub u64);

impl RegionId {
    pub const INVALID: RegionId = RegionId(0);

    pub fn is_valid(self) -> bool {
        self.0 != 0
    }
}

#[derive(Debug)]
struct Region {
    ptr: *mut u8,
    layout: Layout,
    len: usize,
}

// SAFETY: regions are only accessed through synchronized APIs; pointer is owned.
unsafe impl Send for Region {}
unsafe impl Sync for Region {}

/// Off-heap arena registry — GC-bypass foundation for Phase 2.
pub struct OffHeapMemory {
    next_id: AtomicU64,
    regions: DashMap<RegionId, Mutex<Region>>,
    managed_bytes: AtomicU64,
}

impl OffHeapMemory {
    pub fn new() -> Self {
        Self {
            next_id: AtomicU64::new(1),
            regions: DashMap::new(),
            managed_bytes: AtomicU64::new(0),
        }
    }

    pub fn allocate(&self, size: usize, align: usize) -> Option<RegionId> {
        if size == 0 {
            return None;
        }
        let align = align.max(1).next_power_of_two();
        let layout = Layout::from_size_align(size, align).ok()?;
        let ptr = unsafe { alloc(layout) };
        if ptr.is_null() {
            return None;
        }
        // Zero-fill so Java/Panama views see defined memory.
        unsafe {
            std::ptr::write_bytes(ptr, 0, size);
        }
        let id = RegionId(self.next_id.fetch_add(1, Ordering::Relaxed));
        self.regions.insert(
            id,
            Mutex::new(Region {
                ptr,
                layout,
                len: size,
            }),
        );
        self.managed_bytes.fetch_add(size as u64, Ordering::Relaxed);
        Some(id)
    }

    pub fn free(&self, id: RegionId) -> bool {
        if !id.is_valid() {
            return false;
        }
        if let Some((_, region)) = self.regions.remove(&id) {
            let region = region.into_inner();
            unsafe { dealloc(region.ptr, region.layout) };
            self.managed_bytes
                .fetch_sub(region.len as u64, Ordering::Relaxed);
            true
        } else {
            false
        }
    }

    pub fn ptr_len(&self, id: RegionId) -> Option<(*mut u8, usize)> {
        let region = self.regions.get(&id)?;
        let region = region.lock();
        Some((region.ptr, region.len))
    }

    pub fn len(&self, id: RegionId) -> Option<usize> {
        self.ptr_len(id).map(|(_, len)| len)
    }

    /// Zero-copy write into a region (bounds-checked).
    pub fn write(&self, id: RegionId, offset: usize, data: &[u8]) -> bool {
        let Some(region) = self.regions.get(&id) else {
            return false;
        };
        let region = region.lock();
        if offset
            .checked_add(data.len())
            .map(|e| e > region.len)
            .unwrap_or(true)
        {
            return false;
        }
        unsafe {
            std::ptr::copy_nonoverlapping(data.as_ptr(), region.ptr.add(offset), data.len());
        }
        true
    }

    /// Zero-copy read from a region into `dst` (bounds-checked).
    pub fn read(&self, id: RegionId, offset: usize, dst: &mut [u8]) -> bool {
        let Some(region) = self.regions.get(&id) else {
            return false;
        };
        let region = region.lock();
        if offset
            .checked_add(dst.len())
            .map(|e| e > region.len)
            .unwrap_or(true)
        {
            return false;
        }
        unsafe {
            std::ptr::copy_nonoverlapping(region.ptr.add(offset), dst.as_mut_ptr(), dst.len());
        }
        true
    }

    pub fn managed_bytes(&self) -> u64 {
        self.managed_bytes.load(Ordering::Relaxed)
    }
}

impl Default for OffHeapMemory {
    fn default() -> Self {
        Self::new()
    }
}

impl Drop for OffHeapMemory {
    fn drop(&mut self) {
        let ids: Vec<_> = self.regions.iter().map(|e| *e.key()).collect();
        for id in ids {
            self.free(id);
        }
    }
}

/// Process-wide off-heap registry shared by C ABI, JNI, and the sidecar.
static GLOBAL: OnceLock<OffHeapMemory> = OnceLock::new();

pub fn global() -> &'static OffHeapMemory {
    GLOBAL.get_or_init(OffHeapMemory::new)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn alloc_write_read_free() {
        let mem = OffHeapMemory::new();
        let id = mem.allocate(64, 8).expect("alloc");
        assert!(mem.write(id, 0, b"javar"));
        let mut buf = [0u8; 5];
        assert!(mem.read(id, 0, &mut buf));
        assert_eq!(&buf, b"javar");
        assert_eq!(mem.managed_bytes(), 64);
        assert!(mem.free(id));
        assert_eq!(mem.managed_bytes(), 0);
    }

    #[test]
    fn ffi_symbols_roundtrip() {
        let id = ffi::javar_mem_alloc(32, 8);
        assert_ne!(id, 0);
        assert!(!ffi::javar_mem_ptr(id).is_null());
        assert_eq!(ffi::javar_mem_len(id), 32);
        let src = b"panama";
        let ok = unsafe { ffi::javar_mem_write(id, 0, src.as_ptr(), src.len()) };
        assert_eq!(ok, 1);
        assert_eq!(ffi::javar_mem_free(id), 1);
        assert_eq!(ffi::javar_mem_abi_version(), 1);
    }
}
