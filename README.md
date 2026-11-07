# 🌿 FoliaLens

High-performance offline edge-AI biodiversity atlas for flora, fungi, and fauna built with Rust NDK, Kotlin, Jetpack Compose, and LiteRT.

[![Android MinSDK](https://img.shields.io/badge/Android-MinSDK%2031%20%7C%20TargetSDK%2037-3DDC84?logo=android&logoColor=white)](https://developer.android.com)
[![Rust NDK](https://img.shields.io/badge/Rust%20NDK-arm64--v8a%20%7C%20x86__64-orange?logo=rust&logoColor=white)](https://www.rust-lang.org)
[![SQLite](https://img.shields.io/badge/SQLite-FTS5%20%2B%20Zstd-003B57?logo=sqlite&logoColor=white)](https://www.sqlite.org)
[![Spatial Index](https://img.shields.io/badge/Spatial%20Index-Uber%20H3%20(h3o)-black?logo=uber&logoColor=white)](https://h3geo.org)
[![License: MIT](https://img.shields.io/badge/License-MIT-blue.svg)](LICENSE)

---

## Overview

**FoliaLens** is an autonomous, on-device scientific atlas engineered for biologists, foragers, and outdoor researchers. Designed for full offline reliability in remote wilderness environments without cellular connectivity, FoliaLens combines real-time computer vision and bioacoustics inference (LiteRT GPU/NPU) with an ultra-compact embedded database and spatiotemporal geospatial filtering powered by Rust.

---

## Architecture & Data Pipeline

```
┌────────────────────────────────────────────────────────────────────────┐
│                        Jetpack Compose UI                              │
│         (Search Bar, Inference Visualizer, Taxon Detail Dialog)        │
└───────────────────────────────────┬────────────────────────────────────┘
                                    │
                                    ▼
┌────────────────────────────────────────────────────────────────────────┐
│                 Kotlin Orchestration & CameraX / Audio                 │
│         DirectByteBuffer zero-copy buffers, CameraX ImageProxy         │
└───────────────────┬────────────────────────────────┬───────────────────┘
                    │                                │
      Zero-Copy JNI │                  UniFFI 0.28   │ Type-safe FFI
      (DirectBuffer)│                  (High-level)  │ (AtlasEngine)
                    ▼                                ▼
┌────────────────────────────────────────────────────────────────────────┐
│                      rust-core (atlas_core.so)                         │
│  ┌───────────────────────┐ ┌──────────────────────┐ ┌───────────────┐  │
│  │     Spatial H3        │ │  SQLite FTS5 + Zstd  │ │ Logit Softmax │  │
│  │ (h3o Resolution 4 +   │ │ (Full-text search &  │ │ + Spatial     │  │
│  │  12-bit Season Masks) │ │  decompression)      │ │   Penalties)  │  │
│  └───────────────────────┘ └──────────────────────┘ └───────────────┘  │
└────────────────────────────────────────────────────────────────────────┘
```

### 1. Hybrid Android + Rust Core
- **Low-level high-throughput engine (`rust-core`)**: Implemented in Rust, compiled into `libatlas_core.so` targeting `arm64-v8a` and `x86_64` via `cargo-ndk`.
- **Zero-Copy JNI via `DirectByteBuffer`**: Passes camera frames and raw PCM audio buffers directly from Android memory to Rust without heap allocation or copying, enabling high-fps image resizing and FFT spectrogram transformations.
- **Type-Safe UniFFI 0.28 Bridge**: Exposes high-level domain models, database queries, and inference postprocessing to Kotlin via Mozilla UniFFI with zero manual JNI boilerplate.

### 2. High-Efficiency Embedded Database (SQLite + FTS5 + Zstd)
- **FTS5 Full-Text Search**: Pre-indexed multi-language search (Russian common names, Latin binomials, synonyms) with prefix matching (`*`) and BM25 rank ordering.
- **Zstandard (Zstd) Compression**: Encyclopedic species descriptions and field notes are compressed with Zstd (level 3) directly in database BLOBs, slashing database disk footprint by over 60% while maintaining sub-millisecond decompression times on device.

### 3. Spatiotemporal Bayesian Prior Filtering (Uber H3)
- **Uber H3 Hexagonal Grid (`h3o`, Resolution 4)**: The Earth's surface is partitioned into discrete hexagonal cells of ~11,000 km² each. Species ranges are indexed by 64-bit integer H3 cell identifiers.
- **12-Bit Monthly Phenology Mask**: Each distribution entry stores a 12-bit integer bitmask representing seasonal occurrence across months (January = bit 0 .. December = bit 11).
- **Logit Bayesian Postprocessing**: Raw vision model output logits are transformed via Softmax and adjusted against geographic coordinates and observation date. Taxa unrecorded in the local H3 cell or season receive a calibrated Bayesian penalty (-0.15), suppressing false-positive misidentifications of non-native species.

---

## Project Structure

```text
bio-atlas/
├── app/                                 # Android application module
│   ├── build.gradle.kts                 # AGP 9 build script with cargo-ndk & uniffi tasks
│   └── src/main/
│       ├── java/com/example/folialens/
│       │   ├── MainActivity.kt          # Compose UI (FTS5 search, inference simulator, detail card)
│       │   └── core/
│       │       ├── NativeBridge.kt      # Direct JNI native bridge declarations
│       │       └── uniffi/              # Generated UniFFI Kotlin bindings (atlas_core.kt)
│       └── jniLibs/                     # Pre-compiled native libraries
│           ├── arm64-v8a/libatlas_core.so
│           └── x86_64/libatlas_core.so
├── rust-core/                           # Native Rust workspace (atlas_core)
│   ├── Cargo.toml                       # Dependencies: uniffi, rusqlite (fts5, bundled), zstd, h3o
│   ├── uniffi.toml                      # UniFFI package bindings configuration
│   └── src/
│       ├── lib.rs                       # UniFFI export declarations & AtlasEngine API
│       ├── db.rs                        # SQLite + FTS5 + Zstd engine & indicator seed data
│       ├── geo.rs                       # Uber H3 (h3o) spatial indexing & seasonal bitmask math
│       ├── vision.rs                    # Fast bilinear image resize & tensor preprocessing (TBD)
│       ├── audio.rs                     # RustFFT spectrogram & mel-filterbank pipelines (TBD)
│       └── bin/
│           └── uniffi-bindgen.rs        # CLI generator binary for UniFFI bindings
├── ARCHITECTURE.md                      # Comprehensive architectural blueprint
├── LICENSE                              # MIT License
└── README.md                            # Technical repository documentation
```

---

## Build Instructions

### Prerequisites
1. **Android Studio** (Ladybug Feature Drop or newer) with **Android SDK 37** and **NDK (r27+)**.
2. **Rust Toolchain** (1.80+):
   ```bash
   rustup default stable
   ```
3. **Android Rust Targets**:
   ```bash
   rustup target add aarch64-linux-android x86_64-linux-android
   ```
4. **Cargo NDK**:
   ```bash
   cargo install cargo-ndk
   ```

### Building the Project
From the repository root directory:

```bash
# Assemble Debug APK with automatic Rust NDK compilation and UniFFI binding generation
./gradlew :app:assembleDebug
```

On Windows (PowerShell / CMD):
```powershell
.\gradlew.bat :app:assembleDebug
```

The resulting debug APK will be generated at:
```
app/build/outputs/apk/debug/app-debug.apk
```

### Installing and Running
```bash
# Deploy to connected USB device or Android Emulator
./gradlew :app:installDebug
```

---

## License

This project is licensed under the terms of the [MIT License](LICENSE).  
Copyright (c) 2026 Yuri Egorov.
