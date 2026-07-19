/**
 * JavaR off-heap memory C ABI — consumed by Project Panama and other FFIs.
 * Keep in sync with javar-core/src/memory/ffi.rs
 */
#ifndef JAVAR_MEM_H
#define JAVAR_MEM_H

#include <stddef.h>
#include <stdint.h>

#ifdef __cplusplus
extern "C" {
#endif

uint64_t javar_mem_alloc(size_t size, size_t align);
int      javar_mem_free(uint64_t id);
void*    javar_mem_ptr(uint64_t id);
size_t   javar_mem_len(uint64_t id);
int      javar_mem_write(uint64_t id, size_t offset, const uint8_t* src, size_t len);
int      javar_mem_read(uint64_t id, size_t offset, uint8_t* dst, size_t len);
uint64_t javar_mem_managed_bytes(void);
uint32_t javar_mem_abi_version(void);

#ifdef __cplusplus
}
#endif

#endif /* JAVAR_MEM_H */
