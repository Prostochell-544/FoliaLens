package com.example.folialens.core

import androidx.camera.core.ImageAnalysis
import androidx.camera.core.ImageProxy
import com.example.folialens.core.uniffi.AtlasEngine
import com.example.folialens.core.uniffi.TaxonSummary
import java.nio.ByteBuffer
import java.nio.ByteOrder

data class LiveVisionAnalysisResult(
    val topTaxa: List<TaxonSummary>,
    val rustPreprocessMs: Double,
    val inferenceMs: Double,
    val engineMode: String,
)

/**
 * CameraX ImageAnalysis.Analyzer that connects camera frames directly
 * to rust-core (vision.rs) via Zero-Copy DirectByteBuffer and runs LiteRT inference.
 */
class CameraZeroCopyAnalyzer(
    private val atlasEngine: AtlasEngine,
    private val classifier: LiteRtClassifier,
    private val onResult: (LiveVisionAnalysisResult) -> Unit,
) : ImageAnalysis.Analyzer {

    companion object {
        const val TARGET_SIZE = 224
        // 224 * 224 * 3 (RGB) * 4 bytes (Float32)
        const val TENSOR_BYTE_CAPACITY = TARGET_SIZE * TARGET_SIZE * 3 * 4
    }

    // Reusable DirectByteBuffer for zero-copy LiteRT input tensor
    private val tensorBuffer: ByteBuffer = ByteBuffer.allocateDirect(TENSOR_BYTE_CAPACITY)
        .order(ByteOrder.nativeOrder())

    // Location state for H3 Bayesian filtering (defaults to SPb coordinates, July)
    var latitude: Double = 59.93
    var longitude: Double = 30.33
    var month: UByte = 7u

    override fun analyze(image: ImageProxy) {
        try {
            val planes = image.planes
            if (planes.size < 3) {
                return
            }

            val yPlane = planes[0].buffer
            val uPlane = planes[1].buffer
            val vPlane = planes[2].buffer

            val srcWidth = image.width
            val srcHeight = image.height
            val yRowStride = planes[0].rowStride
            val uvRowStride = planes[1].rowStride
            val uvPixelStride = planes[1].pixelStride

            // 1. Rust NDK Zero-Copy preprocessing (Center-Crop, BT.601, SIMD resize, ImageNet norm)
            val t0 = System.nanoTime()
            val success = NativeBridge.preprocessCameraFrameZeroCopy(
                yPlane = yPlane,
                uPlane = uPlane,
                vPlane = vPlane,
                srcWidth = srcWidth,
                srcHeight = srcHeight,
                yRowStride = yRowStride,
                uvRowStride = uvRowStride,
                uvPixelStride = uvPixelStride,
                outTfliteInputBuffer = tensorBuffer,
                targetSize = TARGET_SIZE,
                normalizeFp32 = true,
            )
            val rustPreprocessMs = (System.nanoTime() - t0) / 1_000_000.0

            if (!success) {
                return
            }

            // 2. LiteRT Inference
            val t1 = System.nanoTime()
            val (logits, engineMode) = classifier.runVisionInference(tensorBuffer, numClasses = 5)
            val inferenceMs = (System.nanoTime() - t1) / 1_000_000.0

            // 3. Spatiotemporal H3 Bayesian prior adjustment
            val topTaxa = atlasEngine.postprocessVisionLogits(
                logits = logits.toList(),
                lat = latitude,
                lon = longitude,
                month = month,
                topK = 3u,
            )

            onResult(
                LiveVisionAnalysisResult(
                    topTaxa = topTaxa,
                    rustPreprocessMs = rustPreprocessMs,
                    inferenceMs = inferenceMs,
                    engineMode = engineMode,
                )
            )
        } catch (_: Throwable) {
            // Ignore transient frame analysis errors to maintain smooth viewfinder stream
        } finally {
            image.close()
        }
    }
}
