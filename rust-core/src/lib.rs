use jni::JNIEnv;
use jni::objects::{JByteBuffer, JClass};
use jni::sys::jstring;

uniffi::setup_scaffolding!();

#[uniffi::export]
pub fn rust_health_check() -> String {
    "FoliaLens Rust Core Online: Ready for FFT & CNN!".to_string()
}

#[derive(uniffi::Enum, Debug, Clone)]
pub enum Kingdom {
    Plantae,
    Fungi,
    Animalia,
}

#[derive(uniffi::Enum, Debug, Clone)]
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

#[derive(uniffi::Error, thiserror::Error, Debug)]
pub enum AtlasError {
    #[error("Database error: {0}")]
    DbError(String),
    #[error("Invalid buffer dimensions")]
    BufferError,
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
