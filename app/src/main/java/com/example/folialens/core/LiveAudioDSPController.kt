package com.example.folialens.core

import android.annotation.SuppressLint
import android.content.Context
import android.graphics.Bitmap
import android.media.AudioFormat
import android.media.AudioRecord
import android.media.MediaRecorder
import com.example.folialens.core.uniffi.AtlasEngine
import com.example.folialens.core.uniffi.TaxonSummary
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.Job
import kotlinx.coroutines.isActive
import kotlinx.coroutines.launch
import java.nio.ByteBuffer
import java.nio.ByteOrder

data class LiveAudioAnalysisResult(
    val spectrogramBitmap: Bitmap,
    val topTaxa: List<TaxonSummary>,
    val dspMs: Double,
    val inferenceMs: Double,
    val engineMode: String,
)

/**
 * Audio capture and streaming DSP controller.
 *
 * Captures 48 kHz mono float PCM via AudioRecord, feeds a 3-second sliding ring buffer
 * (144,000 samples) into Rust NDK via zero-copy DirectByteBuffer, computes Magma Mel-spectrogram
 * and classifies bioacoustic signatures via LiteRT.
 */
class LiveAudioDSPController(
    private val context: Context,
    private val atlasEngine: AtlasEngine,
    private val classifier: LiteRtClassifier,
    private val onResult: (LiveAudioAnalysisResult) -> Unit,
) : AutoCloseable {

    companion object {
        const val SAMPLE_RATE = 48000
        const val WINDOW_SECONDS = 3.0f
        const val NUM_SAMPLES = (SAMPLE_RATE * WINDOW_SECONDS).toInt() // 144,000 samples
        const val CHANNEL_CONFIG = AudioFormat.CHANNEL_IN_MONO
        const val AUDIO_FORMAT = AudioFormat.ENCODING_PCM_FLOAT
    }

    private val dims: List<UInt> = atlasEngine.getSpectrogramDimensions(NUM_SAMPLES.toUInt())
    val nMels: Int = dims[0].toInt()     // 96
    val nFrames: Int = dims[1].toInt()   // 278
    private val melElements = nMels * nFrames

    // Reusable DirectByteBuffers for Zero-Copy JNI
    private val pcmDirectBuffer: ByteBuffer = ByteBuffer.allocateDirect(NUM_SAMPLES * 4)
        .order(ByteOrder.nativeOrder())
    private val melDirectBuffer: ByteBuffer = ByteBuffer.allocateDirect(melElements * 4)
        .order(ByteOrder.nativeOrder())
    private val rgbaDirectBuffer: ByteBuffer = ByteBuffer.allocateDirect(melElements * 4)
        .order(ByteOrder.nativeOrder())

    // In-memory ring buffer (144,000 float samples)
    private val pcmRingBuffer = FloatArray(NUM_SAMPLES)

    // Reusable spectrogram bitmap
    private val spectrogramBitmap: Bitmap = Bitmap.createBitmap(nFrames, nMels, Bitmap.Config.ARGB_8888)

    private var audioRecord: AudioRecord? = null
    private var recordingJob: Job? = null
    var isRecording: Boolean = false
        private set

    /**
     * Starts continuous background audio recording and live Mel-spectrogram streaming.
     */
    @SuppressLint("MissingPermission")
    fun startListening(scope: CoroutineScope): Boolean {
        if (isRecording) return true

        val minBufferSize = AudioRecord.getMinBufferSize(SAMPLE_RATE, CHANNEL_CONFIG, AUDIO_FORMAT)
        if (minBufferSize <= 0) return false

        try {
            val record = AudioRecord(
                MediaRecorder.AudioSource.MIC,
                SAMPLE_RATE,
                CHANNEL_CONFIG,
                AUDIO_FORMAT,
                maxOf(minBufferSize, 8192 * 4)
            )

            if (record.state != AudioRecord.STATE_INITIALIZED) {
                record.release()
                return false
            }

            record.startRecording()
            audioRecord = record
            isRecording = true

            recordingJob = scope.launch(Dispatchers.Default) {
                val chunkBuffer = FloatArray(2048)
                while (isActive && isRecording) {
                    val readCount = record.read(chunkBuffer, 0, chunkBuffer.size, AudioRecord.READ_BLOCKING)
                    if (readCount > 0) {
                        // Shift ring buffer
                        System.arraycopy(pcmRingBuffer, readCount, pcmRingBuffer, 0, NUM_SAMPLES - readCount)
                        System.arraycopy(chunkBuffer, 0, pcmRingBuffer, NUM_SAMPLES - readCount, readCount)

                        processCurrentBuffer()
                    }
                }
            }
            return true
        } catch (_: Throwable) {
            stopListening()
            return false
        }
    }

    fun stopListening() {
        isRecording = false
        recordingJob?.cancel()
        recordingJob = null
        try {
            audioRecord?.stop()
            audioRecord?.release()
        } catch (_: Throwable) {}
        audioRecord = null
    }

    /**
     * Runs single-shot analysis on synthetic 3-second nightingale chirp (ideal for emulators).
     */
    fun processSyntheticChirp() {
        val syntheticChirp = atlasEngine.generateSyntheticBirdChirp(WINDOW_SECONDS)
        val copyLen = minOf(syntheticChirp.size, NUM_SAMPLES)
        syntheticChirp.take(copyLen).forEachIndexed { idx, value ->
            pcmRingBuffer[idx] = value
        }
        processCurrentBuffer()
    }

    private fun processCurrentBuffer() {
        // 1. Copy PCM float array into direct buffer
        pcmDirectBuffer.rewind()
        val floatView = pcmDirectBuffer.asFloatBuffer()
        floatView.put(pcmRingBuffer)
        pcmDirectBuffer.rewind()

        // 2. Compute Mel-spectrogram in Rust (rustfft + 96-bin Mel filterbank + Magma colormap)
        val t0 = System.nanoTime()
        val success = NativeBridge.computeMelSpectrogramZeroCopy(
            pcmFloatBuffer = pcmDirectBuffer,
            numSamples = NUM_SAMPLES,
            outMelFloatBuffer = melDirectBuffer,
            outRgbaBitmapBuffer = rgbaDirectBuffer,
        )
        val dspMs = (System.nanoTime() - t0) / 1_000_000.0

        if (!success) return

        // 3. Copy pixels directly to Bitmap
        rgbaDirectBuffer.rewind()
        spectrogramBitmap.copyPixelsFromBuffer(rgbaDirectBuffer)

        // 4. LiteRT Audio Classification
        val t1 = System.nanoTime()
        val (logits, engineMode) = classifier.runAudioInference(melDirectBuffer, numClasses = 5)
        val inferenceMs = (System.nanoTime() - t1) / 1_000_000.0

        // 5. Postprocess through AtlasEngine
        val topTaxa = atlasEngine.postprocessVisionLogits(
            logits = logits.toList(),
            lat = 59.93,
            lon = 30.33,
            month = 7u,
            topK = 3u,
        )

        onResult(
            LiveAudioAnalysisResult(
                spectrogramBitmap = spectrogramBitmap,
                topTaxa = topTaxa,
                dspMs = dspMs,
                inferenceMs = inferenceMs,
                engineMode = engineMode,
            )
        )
    }

    override fun close() {
        stopListening()
    }
}
