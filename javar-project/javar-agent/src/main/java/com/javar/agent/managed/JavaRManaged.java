package com.javar.agent.managed;

import java.lang.annotation.Documented;
import java.lang.annotation.ElementType;
import java.lang.annotation.Retention;
import java.lang.annotation.RetentionPolicy;
import java.lang.annotation.Target;

/**
 * Marks a class for transparent off-heap field storage.
 * <p>
 * The JavaR agent rewrites {@code GETFIELD}/{@code PUTFIELD} for primitive
 * instance fields so values live in Rust-managed memory ({@code javar_mem_*}).
 * The Java object becomes a thin shell (header + region id), keeping weight
 * out of the GC.
 *
 * <pre>
 * &#64;JavaRManaged
 * public class Sensor {
 *     private int temperature; // stored off-heap
 *     private long timestamp;  // stored off-heap
 * }
 * </pre>
 *
 * @author Roberto de Souza (rabbittrix@hotmail.com)
 */
@Documented
@Retention(RetentionPolicy.RUNTIME)
@Target(ElementType.TYPE)
public @interface JavaRManaged {

    /**
     * When {@code true} (default), primitive instance fields are moved off-heap.
     * Reference fields always remain on the Java shell.
     */
    boolean primitives() default true;
}
