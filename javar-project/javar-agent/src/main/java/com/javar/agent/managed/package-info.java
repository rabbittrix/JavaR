/**
 * Transparent off-heap field mapping for {@link com.javar.agent.managed.JavaRManaged}.
 * <p>
 * {@link com.javar.agent.managed.ManagedClassTransformer} rewrites primitive
 * field access to {@link com.javar.agent.managed.JavaRManagedRuntime}, which
 * stores payloads via the Panama/JNI {@code OffHeapBridge}.
 */
package com.javar.agent.managed;
