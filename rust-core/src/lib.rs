pub mod audio;
pub mod dataset;
pub mod db;
pub mod geo;
pub mod vision;

use std::panic::{catch_unwind, AssertUnwindSafe};
use std::sync::{Arc, OnceLock};
use jni::JNIEnv;
use jni::objects::{JByteBuffer, JClass, JObject};
use jni::sys::{jboolean, jint, jstring, JNI_FALSE, JNI_TRUE};

uniffi::setup_scaffolding!();

#[uniffi::export]
pub fn rust_health_check() -> String {
    "FoliaLens Rust Core Online: Ready for FFT & CNN!".to_string()
}

#[derive(uniffi::Enum, Debug, Clone, PartialEq, Eq)]
pub enum Kingdom {
    Plantae,
    Fungi,
    Animalia,
}

#[derive(uniffi::Enum, Debug, Clone, PartialEq, Eq)]
pub enum SafetyStatus {
    Neutral,
    Edible,
    Toxic,
    Deadly,
    Protected,
}

/// Ecological niche of a taxon, stored as `taxa.habitat_type` (codes 0..=3).
#[derive(uniffi::Enum, Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum HabitatType {
    /// 0 — land (Суша).
    Terrestrial,
    /// 1 — rivers, lakes, ponds (Пресный водоём).
    Freshwater,
    /// 2 — seas and oceans (Морская среда).
    Marine,
    /// 3 — marshes, bogs, shores with emergent vegetation (Водно-болотные угодья).
    Wetland,
}

impl HabitatType {
    pub const ALL: [HabitatType; 4] = [
        HabitatType::Terrestrial,
        HabitatType::Freshwater,
        HabitatType::Marine,
        HabitatType::Wetland,
    ];

    /// Integer code stored in `taxa.habitat_type`.
    pub const fn code(self) -> i64 {
        match self {
            HabitatType::Terrestrial => 0,
            HabitatType::Freshwater => 1,
            HabitatType::Marine => 2,
            HabitatType::Wetland => 3,
        }
    }

    pub fn from_code(code: i64) -> Option<Self> {
        Self::ALL.into_iter().find(|h| h.code() == code)
    }
}

#[derive(uniffi::Record, Debug, Clone)]
pub struct TaxonSummary {
    pub taxon_id: i64,
    pub scientific_name: String,
    pub common_name_ru: String,
    pub kingdom: Kingdom,
    pub safety: SafetyStatus,
    pub confidence: f32,
    pub geo_verified: bool,
}

#[derive(uniffi::Record, Debug, Clone)]
pub struct TaxonDetail {
    pub summary: TaxonSummary,
    pub description_markdown: String,
    pub h3_cell_hex: String,
}

#[derive(uniffi::Error, thiserror::Error, Debug)]
pub enum AtlasError {
    #[error("Database error: {0}")]
    DbError(String),
    #[error("Invalid buffer dimensions")]
    BufferError,
    #[error("Taxon not found: {0}")]
    NotFound(i64),
    #[error("Decompression error: {0}")]
    DecompressError(String),
}

/// Process-wide Mel extractor (default [`audio::AudioConfig`]). Shared by every
/// [`AtlasEngine`] and by the zero-copy JNI audio path, so UniFFI-reported dimensions
/// always match what `computeMelSpectrogramZeroCopy` produces.
static SHARED_MEL_EXTRACTOR: OnceLock<Option<Arc<audio::MelSpectrogramExtractor>>> =
    OnceLock::new();

fn shared_mel_extractor() -> Result<Arc<audio::MelSpectrogramExtractor>, AtlasError> {
    SHARED_MEL_EXTRACTOR
        .get_or_init(|| {
            audio::MelSpectrogramExtractor::new(audio::AudioConfig::default())
                .ok()
                .map(Arc::new)
        })
        .clone()
        .ok_or(AtlasError::BufferError)
}

#[derive(uniffi::Object)]
pub struct AtlasEngine {
    db: db::Database,
    mel: Arc<audio::MelSpectrogramExtractor>,
}

#[uniffi::export]
impl AtlasEngine {
    #[uniffi::constructor]
    pub fn new_in_memory_demo() -> Result<Arc<Self>, AtlasError> {
        let db = db::Database::new_in_memory_demo()?;
        let mel = shared_mel_extractor()?;
        Ok(Arc::new(Self { db, mel }))
    }

    #[uniffi::constructor]
    pub fn new_from_file(db_path: String) -> Result<Arc<Self>, AtlasError> {
        let db = db::Database::new_from_file(&db_path)?;
        let mel = shared_mel_extractor()?;
        Ok(Arc::new(Self { db, mel }))
    }

    /// Synthetic nightingale trill at the engine's sample rate (48 kHz) for testing the
    /// audio pipeline on emulators without a microphone. Durations are clamped to 60 s.
    pub fn generate_synthetic_bird_chirp(&self, duration_sec: f32) -> Vec<f32> {
        audio::generate_synthetic_bird_chirp(self.mel.config().sample_rate, duration_sec)
    }

    /// Returns `[n_mels, n_frames]` for a PCM window of `num_samples` samples — the
    /// spectrogram shape used to size the zero-copy output buffers.
    pub fn get_spectrogram_dimensions(&self, num_samples: u32) -> Vec<u32> {
        let n_mels = self.mel.config().n_mels;
        let n_frames = self.mel.num_frames(num_samples as usize);
        [n_mels, n_frames]
            .into_iter()
            .map(|v| u32::try_from(v).unwrap_or(u32::MAX))
            .collect()
    }

    pub fn search_taxa(&self, query: String, limit: u32) -> Result<Vec<TaxonSummary>, AtlasError> {
        self.db.search_taxa(&query, limit)
    }

    pub fn get_taxon_detail(&self, taxon_id: i64) -> Result<TaxonDetail, AtlasError> {
        self.db.get_taxon_detail(taxon_id)
    }

    pub fn postprocess_vision_logits(
        &self,
        logits: Vec<f32>,
        lat: Option<f64>,
        lon: Option<f64>,
        month: u8,
        top_k: u32,
    ) -> Result<Vec<TaxonSummary>, AtlasError> {
        self.db
            .postprocess_vision_logits(&logits, lat, lon, month, top_k)
    }

    /// Same as [`Self::postprocess_vision_logits`], but keeps only taxa whose habitat is
    /// in `habitats` (empty = no filter), e.g. `[Freshwater, Wetland]` at a pond shore.
    pub fn postprocess_vision_logits_in_habitats(
        &self,
        logits: Vec<f32>,
        lat: Option<f64>,
        lon: Option<f64>,
        month: u8,
        top_k: u32,
        habitats: Vec<HabitatType>,
    ) -> Result<Vec<TaxonSummary>, AtlasError> {
        self.db
            .postprocess_vision_logits_in_habitats(&logits, lat, lon, month, top_k, &habitats)
    }

    /// Lists taxa of one ecosystem ordered by `taxon_id`.
    pub fn list_taxa_by_habitat(
        &self,
        habitat: HabitatType,
        limit: u32,
    ) -> Result<Vec<TaxonSummary>, AtlasError> {
        self.db.list_taxa_by_habitat(habitat, limit)
    }

    /// Habitat of a single taxon (not part of `TaxonSummary` to keep that record stable).
    pub fn get_taxon_habitat(&self, taxon_id: i64) -> Result<HabitatType, AtlasError> {
        self.db.get_taxon_habitat(taxon_id)
    }
}

#[no_mangle]
pub extern "system" fn Java_com_example_folialens_core_NativeBridge_jniHealthCheck(
    env: JNIEnv,
    _class: JClass,
) -> jstring {
    match env.new_string("Zero-Copy JNI Bridge: OK") {
        Ok(output) => output.into_raw(),
        Err(_) => std::ptr::null_mut(),
    }
}

// ---------------------------------------------------------------------------
// Zero-copy JNI bridge helpers
// ---------------------------------------------------------------------------

/// Address and capacity (bytes) of a `java.nio` direct buffer.
#[derive(Clone, Copy)]
struct DirectRegion {
    ptr: *mut u8,
    capacity: usize,
}

impl DirectRegion {
    fn overlaps(self, len: usize, other: DirectRegion, other_len: usize) -> bool {
        let (a, b) = (self.ptr as usize, other.ptr as usize);
        a < b.saturating_add(other_len) && b < a.saturating_add(len)
    }

    fn is_aligned_to<T>(self) -> bool {
        self.ptr.cast::<T>().is_aligned()
    }
}

/// Resolves a direct `ByteBuffer`; `None` for null, heap-backed or invalid buffers.
/// Never leaves a pending Java exception (`GetDirectBuffer*` do not throw).
fn direct_region(env: &JNIEnv, buf: &JByteBuffer) -> Option<DirectRegion> {
    if buf.is_null() {
        return None;
    }
    let ptr = env.get_direct_buffer_address(buf).ok()?;
    let capacity = env.get_direct_buffer_capacity(buf).ok()?;
    if ptr.is_null() {
        return None;
    }
    Some(DirectRegion { ptr, capacity })
}

/// Runs `body` with panics contained (effective in unwinding builds; release uses
/// `panic = "abort"`, where all invariants are validated before any slice is built).
fn guarded(body: impl FnOnce() -> Option<()>) -> jboolean {
    match catch_unwind(AssertUnwindSafe(body)) {
        Ok(Some(())) => JNI_TRUE,
        _ => JNI_FALSE,
    }
}

/// Center-crops, converts (BT.601) and resizes a CameraX `YUV_420_888` frame straight
/// into a LiteRT input tensor buffer. Buffer positions are ignored (base address used).
///
/// Returns `JNI_FALSE` — without touching the output — for null/non-direct buffers,
/// negative or inconsistent geometry, undersized planes or output, or an output buffer
/// that overlaps an input plane.
#[no_mangle]
pub extern "system" fn Java_com_example_folialens_core_NativeBridge_preprocessCameraFrameZeroCopy(
    env: JNIEnv,
    _this: JObject,
    y_plane: JByteBuffer,
    u_plane: JByteBuffer,
    v_plane: JByteBuffer,
    src_width: jint,
    src_height: jint,
    y_row_stride: jint,
    uv_row_stride: jint,
    uv_pixel_stride: jint,
    out_tflite_input_buffer: JByteBuffer,
    target_size: jint,
    normalize_fp32: jboolean,
) -> jboolean {
    guarded(|| {
        let dim = |v: jint| usize::try_from(v).ok();
        let (width, height) = (dim(src_width)?, dim(src_height)?);
        let (y_stride, uv_stride, uv_px_stride) =
            (dim(y_row_stride)?, dim(uv_row_stride)?, dim(uv_pixel_stride)?);
        let target = dim(target_size)?;
        let fp32 = normalize_fp32 != JNI_FALSE;

        let y = direct_region(&env, &y_plane)?;
        let u = direct_region(&env, &u_plane)?;
        let v = direct_region(&env, &v_plane)?;
        let out = direct_region(&env, &out_tflite_input_buffer)?;

        let out_len = vision::output_len(target, fp32)?;
        if out.capacity < out_len {
            return None;
        }
        if [y, u, v]
            .iter()
            .any(|plane| out.overlaps(out_len, *plane, plane.capacity))
        {
            return None;
        }

        // SAFETY: each pointer is a non-null direct-buffer base address valid for
        // `capacity` bytes; the output region is disjoint from all inputs (checked
        // above), and input planes are only read (U/V may alias each other for NV21).
        let (y_s, u_s, v_s, out_s) = unsafe {
            (
                std::slice::from_raw_parts(y.ptr as *const u8, y.capacity),
                std::slice::from_raw_parts(u.ptr as *const u8, u.capacity),
                std::slice::from_raw_parts(v.ptr as *const u8, v.capacity),
                std::slice::from_raw_parts_mut(out.ptr, out_len),
            )
        };
        vision::preprocess_yuv420_frame(
            y_s, u_s, v_s, width, height, y_stride, uv_stride, uv_px_stride, target, fp32, out_s,
        )
        .ok()
    })
}

/// Computes the normalized log-Mel spectrogram of `num_samples` native-order `f32` PCM
/// samples (48 kHz mono) into `out_mel_float_buffer` (`n_mels · n_frames` native-order
/// `f32`, mel-major) and, if `out_rgba_bitmap_buffer` is non-null, an RGBA8888 image
/// (`n_frames` × `n_mels`, low frequencies at the bottom) for
/// `Bitmap.copyPixelsFromBuffer`.
///
/// Returns `JNI_FALSE` for null/non-direct/misaligned float buffers, `num_samples <= 0`,
/// undersized or overlapping buffers.
#[no_mangle]
pub extern "system" fn Java_com_example_folialens_core_NativeBridge_computeMelSpectrogramZeroCopy(
    env: JNIEnv,
    _this: JObject,
    pcm_float_buffer: JByteBuffer,
    num_samples: jint,
    out_mel_float_buffer: JByteBuffer,
    out_rgba_bitmap_buffer: JByteBuffer,
) -> jboolean {
    guarded(|| {
        let extractor = shared_mel_extractor().ok()?;
        let n = usize::try_from(num_samples).ok().filter(|&n| n > 0)?;
        let f32_size = std::mem::size_of::<f32>();

        let pcm = direct_region(&env, &pcm_float_buffer)?;
        let pcm_bytes = n.checked_mul(f32_size)?;
        if pcm.capacity < pcm_bytes || !pcm.is_aligned_to::<f32>() {
            return None;
        }

        let mel_len = extractor.output_len(n)?;
        let mel_bytes = mel_len.checked_mul(f32_size)?;
        let mel = direct_region(&env, &out_mel_float_buffer)?;
        if mel.capacity < mel_bytes
            || !mel.is_aligned_to::<f32>()
            || mel.overlaps(mel_bytes, pcm, pcm_bytes)
        {
            return None;
        }

        let rgba = if out_rgba_bitmap_buffer.is_null() {
            None
        } else {
            let region = direct_region(&env, &out_rgba_bitmap_buffer)?;
            let rgba_bytes = mel_len.checked_mul(4)?;
            if region.capacity < rgba_bytes
                || region.overlaps(rgba_bytes, pcm, pcm_bytes)
                || region.overlaps(rgba_bytes, mel, mel_bytes)
            {
                return None;
            }
            Some((region, rgba_bytes))
        };

        // SAFETY: non-null direct-buffer addresses with sufficient capacity, `f32`
        // alignment verified for the float buffers, and all three regions pairwise
        // disjoint. Every bit pattern is a valid `f32`.
        let (pcm_s, mel_s, rgba_s) = unsafe {
            (
                std::slice::from_raw_parts(pcm.ptr as *const f32, n),
                std::slice::from_raw_parts_mut(mel.ptr as *mut f32, mel_len),
                rgba.map(|(r, len)| std::slice::from_raw_parts_mut(r.ptr, len)),
            )
        };
        extractor.compute(pcm_s, mel_s, rgba_s).ok().map(|_| ())
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn engine_exposes_audio_helpers() {
        let engine = AtlasEngine::new_in_memory_demo().expect("engine");
        assert_eq!(engine.get_spectrogram_dimensions(144_000), vec![96, 278]);
        assert_eq!(engine.get_spectrogram_dimensions(0), vec![96, 1]);
        assert_eq!(engine.generate_synthetic_bird_chirp(1.0).len(), 48_000);
        assert!(engine.generate_synthetic_bird_chirp(0.0).is_empty());
    }

    #[test]
    fn direct_region_overlap_and_alignment() {
        let mut backing = [0u32; 8];
        let base = backing.as_mut_ptr() as *mut u8;
        let r = |offset: usize, capacity: usize| DirectRegion {
            ptr: unsafe { base.add(offset) },
            capacity,
        };
        assert!(r(0, 16).overlaps(16, r(8, 8), 8));
        assert!(!r(0, 8).overlaps(8, r(8, 8), 8));
        assert!(!r(16, 8).overlaps(8, r(0, 16), 16));
        assert!(r(0, 4).is_aligned_to::<f32>());
        assert!(!r(1, 4).is_aligned_to::<f32>());
    }
}
