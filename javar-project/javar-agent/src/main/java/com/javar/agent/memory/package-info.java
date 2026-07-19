/**
 * Zero-copy off-heap access to memory owned by the Rust {@code javar_core} library.
 * <ul>
 *   <li>Java 22+: {@code PanamaOffHeapBridge} (Project Panama / FFM)</li>
 *   <li>Java 8–21: {@link com.javar.agent.memory.JniOffHeapBridge}</li>
 * </ul>
 * Selection is automatic via {@link com.javar.agent.memory.OffHeapBridgeFactory}.
 */
package com.javar.agent.memory;
