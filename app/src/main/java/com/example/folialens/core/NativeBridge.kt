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
     * Converts a CameraX `YUV_420_888` frame straight into a pre-allocated LiteRT input
     * tensor, bypassing JVM heap allocations.
     *
     * Pipeline: center square crop (`min(srcWidth, srcHeight)`) → ITU-R BT.601 YUV→RGB →
     * SIMD resize to `targetSize × targetSize` → NHWC interleaved RGB.
     *
     * All buffers must be direct (`ByteBuffer.allocateDirect` or `ImageProxy` planes);
     * their positions are ignored. Pass plane strides exactly as reported by
     * `ImageProxy.PlaneProxy` (`uvPixelStride` 1 = I420, 2 = NV12/NV21).
     *
     * Required capacity of [outTfliteInputBuffer]:
     * - `normalizeFp32 = false`: `targetSize * targetSize * 3` bytes (`uint8` RGB);
     * - `normalizeFp32 = true`: `targetSize * targetSize * 3 * 4` bytes (little-endian
     *   `float32`, ImageNet mean/std normalized) — use `ByteOrder.nativeOrder()` to read.
     *
     * @return `true` on success; `false` (output untouched) for invalid geometry,
     *   undersized or non-direct buffers, or an output buffer overlapping an input plane.
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
        normalizeFp32: Boolean,
    ): Boolean

    /**
     * Computes a normalized (`0.0..1.0`) log-Mel spectrogram from 48 kHz mono float PCM.
     *
     * Size the outputs with `AtlasEngine.getSpectrogramDimensions(numSamples)` →
     * `[nMels, nFrames]` (defaults: 96 Mel bands, `nFrames = (numSamples - 2048) / 512 + 1`).
     *
     * @param pcmFloatBuffer direct buffer holding at least [numSamples] `float32` samples
     *   in native byte order (`ByteBuffer.order(ByteOrder.nativeOrder())`), 4-byte aligned.
     * @param outMelFloatBuffer direct, 4-byte aligned, at least `nMels * nFrames * 4`
     *   bytes; receives native-order `float32` laid out mel-major (`[nMels][nFrames]`).
     * @param outRgbaBitmapBuffer optional direct buffer of at least `nMels * nFrames * 4`
     *   bytes; receives a Magma-colormapped RGBA8888 image (width `nFrames`, height
     *   `nMels`, low frequencies at the bottom) for `Bitmap.copyPixelsFromBuffer` on an
     *   `ARGB_8888` bitmap. Pass `null` to skip rendering.
     * @return `true` on success; `false` for invalid, undersized, misaligned, non-direct
     *   or overlapping buffers, or `numSamples <= 0`.
     */
    external fun computeMelSpectrogramZeroCopy(
        pcmFloatBuffer: ByteBuffer,
        numSamples: Int,
        outMelFloatBuffer: ByteBuffer,
        outRgbaBitmapBuffer: ByteBuffer?,
    ): Boolean
}
