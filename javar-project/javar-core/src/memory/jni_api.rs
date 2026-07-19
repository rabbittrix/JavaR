//! JNI natives for Java ≤21 — zero-copy via `NewDirectByteBuffer`.

use super::{global, RegionId};
use jni::objects::{JByteArray, JClass, JObject};
use jni::sys::{jboolean, jlong, JNI_FALSE, JNI_TRUE};
use jni::JNIEnv;
use tracing::warn;

/// `long nativeAlloc(long size, long align)`
#[no_mangle]
pub extern "system" fn Java_com_javar_agent_memory_JniOffHeapBridge_nativeAlloc(
    _env: JNIEnv,
    _class: JClass,
    size: jlong,
    align: jlong,
) -> jlong {
    if size <= 0 {
        return 0;
    }
    let align = if align <= 0 { 8 } else { align as usize };
    match global().allocate(size as usize, align) {
        Some(id) => id.0 as jlong,
        None => 0,
    }
}

/// `boolean nativeFree(long id)`
#[no_mangle]
pub extern "system" fn Java_com_javar_agent_memory_JniOffHeapBridge_nativeFree(
    _env: JNIEnv,
    _class: JClass,
    id: jlong,
) -> jboolean {
    if global().free(RegionId(id as u64)) {
        JNI_TRUE
    } else {
        JNI_FALSE
    }
}

/// `long nativeAddress(long id)`
#[no_mangle]
pub extern "system" fn Java_com_javar_agent_memory_JniOffHeapBridge_nativeAddress(
    _env: JNIEnv,
    _class: JClass,
    id: jlong,
) -> jlong {
    match global().ptr_len(RegionId(id as u64)) {
        Some((ptr, _)) => ptr as usize as jlong,
        None => 0,
    }
}

/// `long nativeSize(long id)`
#[no_mangle]
pub extern "system" fn Java_com_javar_agent_memory_JniOffHeapBridge_nativeSize(
    _env: JNIEnv,
    _class: JClass,
    id: jlong,
) -> jlong {
    global()
        .len(RegionId(id as u64))
        .map(|n| n as jlong)
        .unwrap_or(0)
}

/// `long nativeManagedBytes()`
#[no_mangle]
pub extern "system" fn Java_com_javar_agent_memory_JniOffHeapBridge_nativeManagedBytes(
    _env: JNIEnv,
    _class: JClass,
) -> jlong {
    global().managed_bytes() as jlong
}

/// `ByteBuffer nativeAsDirectBuffer(long id)` — zero-copy view of Rust memory.
#[no_mangle]
pub extern "system" fn Java_com_javar_agent_memory_JniOffHeapBridge_nativeAsDirectBuffer<'local>(
    mut env: JNIEnv<'local>,
    _class: JClass<'local>,
    id: jlong,
) -> JObject<'local> {
    let Some((ptr, len)) = global().ptr_len(RegionId(id as u64)) else {
        return JObject::null();
    };
    if len == 0 {
        return JObject::null();
    }
    // SAFETY: region stays alive until nativeFree; Java must not use the buffer after free.
    match unsafe { env.new_direct_byte_buffer(ptr, len) } {
        Ok(buf) => buf.into(),
        Err(err) => {
            warn!(?err, "NewDirectByteBuffer failed");
            let _ = env.throw_new(
                "java/lang/RuntimeException",
                format!("JavaR DirectByteBuffer failed: {err}"),
            );
            JObject::null()
        }
    }
}

/// `boolean nativeWrite(long id, long offset, byte[] data)`
#[no_mangle]
pub extern "system" fn Java_com_javar_agent_memory_JniOffHeapBridge_nativeWrite(
    env: JNIEnv,
    _class: JClass,
    id: jlong,
    offset: jlong,
    data: JByteArray,
) -> jboolean {
    if offset < 0 {
        return JNI_FALSE;
    }
    let Ok(bytes) = env.convert_byte_array(&data) else {
        return JNI_FALSE;
    };
    if global().write(RegionId(id as u64), offset as usize, &bytes) {
        JNI_TRUE
    } else {
        JNI_FALSE
    }
}

/// Called when the native library is loaded into a JVM (`System.loadLibrary`).
#[no_mangle]
pub extern "system" fn JNI_OnLoad(
    vm: jni::JavaVM,
    _reserved: *mut std::os::raw::c_void,
) -> jni::sys::jint {
    #[cfg(feature = "jni-bridge")]
    {
        crate::bridge::jni_bridge::register_jvm(vm);
    }
    #[cfg(not(feature = "jni-bridge"))]
    {
        let _ = vm;
    }
    jni::sys::JNI_VERSION_1_6
}
