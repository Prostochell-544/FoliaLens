# FoliaLens Architecture Specification

**FoliaLens** is a high-performance, offline-first Edge-AI field atlas and identification system for flora, fungi, and fauna. It combines modern Android Jetpack Compose UI, on-device neural network acceleration (LiteRT on GPU/NPU), and a native Rust NDK core (`atlas_core`) via UniFFI and zero-copy JNI.

---

## 1. System Overview & Technology Stack

| Layer | Technologies & Libraries | Responsibility |
|---|---|---|
| **Presentation (UI)** | Jetpack Compose, Material 3, Navigation, Coroutines / Flow | Adaptive UI, real-time camera viewfinder, results visualization, catalog search, offline map explorer. |
| **Edge Hardware Interop** | Android CameraX (1.4.0), AudioRecord API, MinSDK 31 (Android 12+) | Zero-overhead hardware sensor streaming (`ImageAnalysis`, PCM audio streaming). |
| **On-Device Inference** | Google LiteRT 1.0.1 (`litert`, `litert-gpu`, `litert-support`) | Accelerated neural inference on GPU/NPU for visual and acoustic feature extraction and classification. |
| **Native Core (`atlas_core`)** | Rust 2021, UniFFI 0.28, JNI 0.21, `thiserror` 2.0 | High-performance signal processing, zero-copy buffer transformations, spatial calculations, database operations. |
| **Spatial Indexing** | Uber H3 (`h3o` 0.6) | Hexagonal hierarchical spatial indexing for offline geolocation verification and species occurrence filtering. |
| **Signal & Image Processing** | `rustfft` 6.2, `fast_image_resize` 4.2 | Real-time audio spectrogram generation (STFT) and SIMD-accelerated YUV/RGB camera frame resampling. |
| **Embedded Storage** | SQLite (`rusqlite` 0.32 with `bundled` + FTS5), `zstd` 0.13 | High-density offline compressed species encyclopedia and instant full-text search. |

---

## 2. Hybrid Data Flow & Zero-Copy Pipeline

```
+-----------------------------------------------------------------------------------+
| Android (Kotlin / Jetpack Compose)                                                |
|                                                                                   |
|  +-------------------+        +--------------------+                              |
|  | CameraX View /    |        | AudioRecord Stream |                              |
|  | ImageAnalysis     |        | (PCM 16-bit 48kHz) |                              |
|  +--------+----------+        +---------+----------+                              |
|           | DirectByteBuffer            | DirectByteBuffer                        |
|           v                             v                                         |
|  +-------------------------------------------------+                              |
|  | NativeBridge (JNI / Zero-Copy DirectByteBuffer) |                              |
+--+--------+-----------------------------+----------+------------------------------+
            |                             |
            v                             v
+-----------------------------------------------------------------------------------+
| Rust NDK Core (`atlas_core`)                                                      |
|                                                                                   |
|  +---------------------------+       +----------------------------+               |
|  | YUV420 to RGB Resample    |       | STFT Spectrogram Matrix    |               |
|  | (fast_image_resize SIMD)  |       | (rustfft 6.2)              |               |
|  +------------+--------------+       +--------------+-------------+               |
|               |                             |                                     |
|               +--------------+--------------+                                     |
|                              |                                                    |
|                              v                                                    |
|               +------------------------------+                                    |
|               | Preprocessed Model Input     |                                    |
+---------------+--------------+---------------+------------------------------------+
                               |
                               v
+-----------------------------------------------------------------------------------+
| Inference & Knowledge Verification                                                |
|                                                                                   |
|  +---------------------------+       +----------------------------+               |
|  | LiteRT GPU/NPU Classifier | ----> | Raw Taxon Candidates       |               |
|  +---------------------------+       +--------------+-------------+               |
|                                                     |                             |
|                                                     v                             |
|  +---------------------------+       +----------------------------+               |
|  | GPS Hex Index (H3 `h3o`)  | ----> | Spatial & Phenology Filter |               |
|  +---------------------------+       +--------------+-------------+               |
|                                                     |                             |
|                                                     v                             |
|  +---------------------------+       +----------------------------+               |
|  | SQLite FTS5 + Zstd        | ----> | Verified TaxonSummary      |               |
|  | Encyclopedia (`atlas.db`) |       | Delivered via UniFFI       |               |
|  +---------------------------+       +----------------------------+               |
+-----------------------------------------------------------------------------------+
```

---

## 3. Database Schema (`atlas.db`)

The database is an embedded, read-only/appendable SQLite database stored in application assets or internal storage, pre-compiled with FTS5 enabled.

### 3.1. `taxa` (Core Species Catalog)
Stores primary taxonomical and identification data.
```sql
CREATE TABLE taxa (
    taxon_id          INTEGER PRIMARY KEY,
    ml_vision_id      INTEGER,                    -- Maps to vision model output logit index
    ml_audio_id       INTEGER,                    -- Maps to audio model output logit index
    kingdom           TEXT NOT NULL CHECK(kingdom IN ('Plantae', 'Fungi', 'Animalia')),
    scientific_name   TEXT NOT NULL,              -- Binomial nomenclature (e.g. 'Boletus edulis')
    common_name_ru    TEXT NOT NULL,              -- Primary Russian name
    safety_status     TEXT NOT NULL CHECK(safety_status IN ('Neutral', 'Edible', 'Toxic', 'Deadly', 'Protected')),
    description_zstd  BLOB NOT NULL               -- High-compression Zstandard article blob
);

CREATE INDEX idx_taxa_ml_vision ON taxa(ml_vision_id);
CREATE INDEX idx_taxa_ml_audio  ON taxa(ml_audio_id);
```

### 3.2. `taxa_fts` (Full-Text Search)
Virtual table leveraging SQLite FTS5 with unicode61 tokenizer for sub-millisecond search across scientific, Russian common names, and morphology.
```sql
CREATE VIRTUAL TABLE taxa_fts USING fts5(
    scientific_name,
    common_name_ru,
    content='taxa',
    content_rowid='taxon_id',
    tokenize='unicode61 remove_diacritics 2'
);
```

### 3.3. `geo_presence` (Uber H3 Geospatial & Seasonality Index)
Spatial presence partitioned by H3 resolution 7 (~1.2 km² per hexagon cell) to quickly verify whether a predicted species is plausible at the user's offline coordinates and current month.
```sql
CREATE TABLE geo_presence (
    h3_index        INTEGER NOT NULL,             -- H3 index (64-bit uint stored as signed int64)
    taxon_id        INTEGER NOT NULL REFERENCES taxa(taxon_id),
    months_bitmask  INTEGER NOT NULL,             -- 12-bit mask (bits 0..11 representing Jan..Dec)
    PRIMARY KEY (h3_index, taxon_id)
);

CREATE INDEX idx_geo_lookup ON geo_presence(h3_index, months_bitmask);
```

### 3.4. `media_blobs` (Optimized Visual Media)
Offline visual representations stored as AVIF image blobs for minimal disk footprint and hardware-decoded presentation.
```sql
CREATE TABLE media_blobs (
    taxon_id    INTEGER PRIMARY KEY REFERENCES taxa(taxon_id),
    avif_bytes  BLOB NOT NULL                     -- AVIF format photo reference
);
```

---

## 4. Native ABI & Bridge Boundaries

- **ABIs Supported**: `arm64-v8a` (primary target for modern 64-bit devices), `x86_64` (emulator and ChromeOS/x86 devices).
- **UniFFI 0.28**: Exposes structured domain models (`TaxonSummary`, `Kingdom`, `SafetyStatus`, `AtlasError`) and high-level query APIs.
- **Direct JNI**: High-throughput vision frame preprocessing (`preprocessCameraFrameZeroCopy`) bypasses UniFFI serialization overhead and operates directly on memory addresses via `java.nio.ByteBuffer.allocateDirect`.
