pub mod db;
pub mod geo;

use std::sync::Arc;
use jni::JNIEnv;
use jni::objects::{JByteBuffer, JClass};
use jni::sys::jstring;

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

#[derive(uniffi::Object)]
pub struct AtlasEngine {
    db: db::Database,
}

#[uniffi::export]
impl AtlasEngine {
    #[uniffi::constructor]
    pub fn new_in_memory_demo() -> Result<Arc<Self>, AtlasError> {
        let db = db::Database::new_in_memory_demo()?;
        Ok(Arc::new(Self { db }))
    }

    #[uniffi::constructor]
    pub fn new_from_file(db_path: String) -> Result<Arc<Self>, AtlasError> {
        let db = db::Database::new_from_file(&db_path)?;
        Ok(Arc::new(Self { db }))
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
}

#[no_mangle]
pub extern "system" fn Java_com_example_folialens_core_NativeBridge_jniHealthCheck(
    env: JNIEnv,
    _class: JClass,
) -> jstring {
    let output = env
        .new_string("Zero-Copy JNI Bridge: OK")
        .expect("Couldn't create java string!");
    output.into_raw()
}

#[no_mangle]
pub extern "C" fn Java_com_example_folialens_core_NativeBridge_preprocessCameraFrameZeroCopy(
    _env: JNIEnv,
    _class: JClass,
    _y_plane: JByteBuffer,
    _u_plane: JByteBuffer,
    _v_plane: JByteBuffer,
    _src_width: i32,
    _src_height: i32,
    _y_row_stride: i32,
    _uv_row_stride: i32,
    _uv_pixel_stride: i32,
    _out_tflite_input_buffer: JByteBuffer,
    _target_size: i32,
) {
}
