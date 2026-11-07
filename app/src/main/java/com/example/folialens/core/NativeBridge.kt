package com.example.folialens.core

import java.nio.ByteBuffer

object NativeBridge {
    init {
        System.loadLibrary("atlas_core")
    }

    /**
     * Direct JNI health check (returns "Zero-Copy JNI Bridge: OK").
     */
    external fun jniHealthCheck(): String

    /**
     * Resamples and converts YUV420 camera planes directly into a pre-allocated DirectByteBuffer
     * ready for LiteRT neural network input, bypassing JVM heap allocations.
     */
    external fun preprocessCameraFrameZeroCopy(
        yPlane: ByteBuffer,
        uPlane: ByteBuffer,
        vPlane: ByteBuffer,
        srcWidth: Int,
        srcHeight: Int,
        yRowStride: Int,
        uvRowStride: Int,
        uvPixelStride: Int,
        outTfliteInputBuffer: ByteBuffer,
        targetSize: Int,
    )
}
