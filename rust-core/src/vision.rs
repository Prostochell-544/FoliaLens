//! Zero-copy camera frame preprocessing: Android `YUV_420_888` → LiteRT input tensor.
//!
//! Pipeline (single pass per stage, no JVM heap involvement):
//! 1. **Center square crop** of `min(src_width, src_height)` pixels.
//! 2. **YUV → RGB8** using ITU-R BT.601 coefficients in Q16 fixed-point arithmetic,
//!    full-range (JFIF) quantization — the dataspace Android camera HALs use for
//!    `YUV_420_888` buffers.
//! 3. **SIMD resize** `crop × crop → target × target` via `fast_image_resize`
//!    (NEON on `arm64-v8a`, SSE4.1/AVX2 on `x86_64`).
//! 4. **Tensor write** straight into the caller's buffer, NHWC interleaved `RGBRGB…`:
//!    - packed `u8` (`target² · 3` bytes), or
//!    - ImageNet-normalized little-endian `f32` (`target² · 3 · 4` bytes).
//!
//! Scratch memory (crop RGB, resized RGB, resizer internals) lives in a reusable
//! [`VisionPreprocessor`]; [`preprocess_yuv420_frame`] keeps one per thread, so after
//! the first frame the hot path performs no allocations.

use std::cell::RefCell;

use fast_image_resize::images::{Image, ImageRef};
use fast_image_resize::{FilterType, PixelType, ResizeAlg, ResizeOptions, Resizer};

use crate::AtlasError;

/// ImageNet per-channel mean (RGB order), applied to values scaled to `0.0..=1.0`.
pub const IMAGENET_MEAN: [f32; 3] = [0.485, 0.456, 0.406];
/// ImageNet per-channel standard deviation (RGB order).
pub const IMAGENET_STD: [f32; 3] = [0.229, 0.224, 0.225];

const RGB_CHANNELS: usize = 3;
const F32_BYTES: usize = std::mem::size_of::<f32>();

// ITU-R BT.601 full-range YCbCr → RGB, Q16 fixed point:
//   R = Y + 1.402    · (Cr − 128)
//   G = Y − 0.344136 · (Cb − 128) − 0.714136 · (Cr − 128)
//   B = Y + 1.772    · (Cb − 128)
// Worst-case magnitude: 255·2^16 + 116_130·128 ≈ 3.2e7, well inside i32.
const Q16_HALF: i32 = 1 << 15;
const CR_TO_R: i32 = 91_881;
const CB_TO_G: i32 = 22_554;
const CR_TO_G: i32 = 46_802;
const CB_TO_B: i32 = 116_130;

/// Number of bytes the output tensor occupies for the given mode.
///
/// Returns `None` on arithmetic overflow.
pub fn output_len(target_size: usize, normalize_fp32: bool) -> Option<usize> {
    let elems = target_size
        .checked_mul(target_size)?
        .checked_mul(RGB_CHANNELS)?;
    if normalize_fp32 {
        elems.checked_mul(F32_BYTES)
    } else {
        Some(elems)
    }
}

#[inline(always)]
fn clamp_u8(v: i32) -> u8 {
    v.clamp(0, 255) as u8
}

/// Converts one BT.601 full-range YCbCr sample to RGB8 with clamping to `0..=255`.
#[inline(always)]
pub fn yuv_to_rgb_bt601(y: u8, u: u8, v: u8) -> [u8; 3] {
    let y = (y as i32) << 16;
    let d = u as i32 - 128;
    let e = v as i32 - 128;
    [
        clamp_u8((y + CR_TO_R * e + Q16_HALF) >> 16),
        clamp_u8((y - CB_TO_G * d - CR_TO_G * e + Q16_HALF) >> 16),
        clamp_u8((y + CB_TO_B * d + Q16_HALF) >> 16),
    ]
}

/// Validated geometry of the center square crop.
#[derive(Debug, Clone, Copy)]
struct CropGeometry {
    x0: usize,
    y0: usize,
    crop: usize,
}

/// Plane layout exactly as reported by `ImageProxy.PlaneProxy`.
#[derive(Debug, Clone, Copy)]
struct PlaneLayout {
    y_row_stride: usize,
    uv_row_stride: usize,
    uv_pixel_stride: usize,
}

/// Checks every invariant up front so the conversion loops can never index out of bounds.
#[allow(clippy::too_many_arguments)]
fn validate_frame(
    y_len: usize,
    u_len: usize,
    v_len: usize,
    src_width: usize,
    src_height: usize,
    layout: PlaneLayout,
    target_size: usize,
    out_len: usize,
    normalize_fp32: bool,
) -> Result<CropGeometry, AtlasError> {
    let err = AtlasError::BufferError;
    let PlaneLayout {
        y_row_stride,
        uv_row_stride,
        uv_pixel_stride,
    } = layout;

    if src_width == 0 || src_height == 0 || target_size == 0 {
        return Err(err);
    }
    // 1 = planar (I420/YV12), 2 = semi-planar (NV12/NV21).
    if !(1..=2).contains(&uv_pixel_stride) {
        return Err(err);
    }
    if y_row_stride < src_width {
        return Err(err);
    }
    let chroma_width = src_width.div_ceil(2);
    let min_uv_row = (chroma_width - 1) * uv_pixel_stride + 1;
    if uv_row_stride < min_uv_row {
        return Err(err);
    }

    let crop = src_width.min(src_height);
    let x0 = (src_width - crop) / 2;
    let y0 = (src_height - crop) / 2;
    let last_row = y0 + crop - 1;
    let last_col = x0 + crop - 1;

    // Android trims trailing row padding from the last row of each plane, so only the
    // bytes the crop actually touches are required.
    let y_needed = last_row
        .checked_mul(y_row_stride)
        .and_then(|v| v.checked_add(last_col + 1))
        .ok_or(AtlasError::BufferError)?;
    let uv_needed = (last_row >> 1)
        .checked_mul(uv_row_stride)
        .and_then(|v| v.checked_add((last_col >> 1) * uv_pixel_stride + 1))
        .ok_or(AtlasError::BufferError)?;
    if y_len < y_needed || u_len < uv_needed || v_len < uv_needed {
        return Err(err);
    }

    let needed_out = output_len(target_size, normalize_fp32).ok_or(AtlasError::BufferError)?;
    if out_len < needed_out {
        return Err(err);
    }
    // fast_image_resize works with u32 dimensions.
    if u32::try_from(crop).is_err() || u32::try_from(target_size).is_err() {
        return Err(err);
    }
    crop.checked_mul(crop)
        .and_then(|v| v.checked_mul(RGB_CHANNELS))
        .ok_or(AtlasError::BufferError)?;

    Ok(CropGeometry { x0, y0, crop })
}

/// Converts the crop window to packed RGB8. `dst.len()` must equal `crop² · 3`.
fn convert_crop_to_rgb(
    y_plane: &[u8],
    u_plane: &[u8],
    v_plane: &[u8],
    layout: PlaneLayout,
    geo: CropGeometry,
    dst: &mut [u8],
) {
    let CropGeometry { x0, y0, crop } = geo;
    let ps = layout.uv_pixel_stride;
    let cx0 = x0 >> 1;
    let uv_span = (((x0 + crop - 1) >> 1) - cx0) * ps + 1;

    for (j, dst_row) in dst.chunks_exact_mut(crop * RGB_CHANNELS).enumerate() {
        let sy = y0 + j;
        let y_start = sy * layout.y_row_stride + x0;
        let y_row = &y_plane[y_start..y_start + crop];
        let uv_start = (sy >> 1) * layout.uv_row_stride + cx0 * ps;
        let u_row = &u_plane[uv_start..uv_start + uv_span];
        let v_row = &v_plane[uv_start..uv_start + uv_span];

        for (i, (px, &y)) in dst_row
            .chunks_exact_mut(RGB_CHANNELS)
            .zip(y_row)
            .enumerate()
        {
            let ci = (((x0 + i) >> 1) - cx0) * ps;
            px.copy_from_slice(&yuv_to_rgb_bt601(y, u_row[ci], v_row[ci]));
        }
    }
}

fn build_imagenet_lut() -> [[f32; 256]; RGB_CHANNELS] {
    let mut lut = [[0.0f32; 256]; RGB_CHANNELS];
    for (c, table) in lut.iter_mut().enumerate() {
        for (v, slot) in table.iter_mut().enumerate() {
            *slot = (v as f32 / 255.0 - IMAGENET_MEAN[c]) / IMAGENET_STD[c];
        }
    }
    lut
}

/// Expands packed RGB8 into ImageNet-normalized little-endian `f32` (12 bytes per pixel).
/// Writes byte-wise, so `out` needs no particular alignment.
fn write_imagenet_f32_le(rgb: &[u8], out: &mut [u8], lut: &[[f32; 256]; RGB_CHANNELS]) {
    for (px, dst) in rgb
        .chunks_exact(RGB_CHANNELS)
        .zip(out.chunks_exact_mut(RGB_CHANNELS * F32_BYTES))
    {
        for c in 0..RGB_CHANNELS {
            let bytes = lut[c][px[c] as usize].to_le_bytes();
            dst[c * F32_BYTES..(c + 1) * F32_BYTES].copy_from_slice(&bytes);
        }
    }
}

/// Reusable preprocessing context: SIMD resizer plus scratch buffers that grow once to
/// the largest frame seen and are then recycled.
pub struct VisionPreprocessor {
    resizer: Resizer,
    resize_options: ResizeOptions,
    rgb_crop: Vec<u8>,
    rgb_resized: Vec<u8>,
    imagenet_lut: [[f32; 256]; RGB_CHANNELS],
}

impl Default for VisionPreprocessor {
    fn default() -> Self {
        Self::new()
    }
}

impl VisionPreprocessor {
    pub fn new() -> Self {
        Self {
            resizer: Resizer::new(),
            // Bilinear convolution (antialiased on downscale) — matches common
            // TF/PyTorch training-time resizing.
            resize_options: ResizeOptions::new()
                .resize_alg(ResizeAlg::Convolution(FilterType::Bilinear)),
            rgb_crop: Vec::new(),
            rgb_resized: Vec::new(),
            imagenet_lut: build_imagenet_lut(),
        }
    }

    /// Center-crops, converts and resizes one `YUV_420_888` frame into `out_buffer`.
    ///
    /// `out_buffer` must hold at least [`output_len`]`(target_size, normalize_fp32)`
    /// bytes; only that prefix is written. All inputs are validated before any work
    /// starts, and invalid geometry yields [`AtlasError::BufferError`] (never a panic).
    #[allow(clippy::too_many_arguments)]
    pub fn process(
        &mut self,
        y_plane: &[u8],
        u_plane: &[u8],
        v_plane: &[u8],
        src_width: usize,
        src_height: usize,
        y_row_stride: usize,
        uv_row_stride: usize,
        uv_pixel_stride: usize,
        target_size: usize,
        normalize_fp32: bool,
        out_buffer: &mut [u8],
    ) -> Result<(), AtlasError> {
        let layout = PlaneLayout {
            y_row_stride,
            uv_row_stride,
            uv_pixel_stride,
        };
        let geo = validate_frame(
            y_plane.len(),
            u_plane.len(),
            v_plane.len(),
            src_width,
            src_height,
            layout,
            target_size,
            out_buffer.len(),
            normalize_fp32,
        )?;
        let out_len = output_len(target_size, normalize_fp32).ok_or(AtlasError::BufferError)?;
        let out = &mut out_buffer[..out_len];
        let rgb_len = target_size * target_size * RGB_CHANNELS;

        // Fast path: crop already has the target size → convert straight into the
        // destination, skipping the resampler entirely.
        if geo.crop == target_size {
            if normalize_fp32 {
                self.rgb_resized.resize(rgb_len, 0);
                convert_crop_to_rgb(y_plane, u_plane, v_plane, layout, geo, &mut self.rgb_resized);
                write_imagenet_f32_le(&self.rgb_resized, out, &self.imagenet_lut);
            } else {
                convert_crop_to_rgb(y_plane, u_plane, v_plane, layout, geo, out);
            }
            return Ok(());
        }

        let crop_len = geo.crop * geo.crop * RGB_CHANNELS;
        self.rgb_crop.resize(crop_len, 0);
        convert_crop_to_rgb(y_plane, u_plane, v_plane, layout, geo, &mut self.rgb_crop);

        // Both values were range-checked in validate_frame.
        let crop_u32 = geo.crop as u32;
        let target_u32 = target_size as u32;
        let src = ImageRef::new(crop_u32, crop_u32, &self.rgb_crop, PixelType::U8x3)
            .map_err(|_| AtlasError::BufferError)?;

        if normalize_fp32 {
            self.rgb_resized.resize(rgb_len, 0);
            let mut dst = Image::from_slice_u8(
                target_u32,
                target_u32,
                &mut self.rgb_resized,
                PixelType::U8x3,
            )
            .map_err(|_| AtlasError::BufferError)?;
            self.resizer
                .resize(&src, &mut dst, &self.resize_options)
                .map_err(|_| AtlasError::BufferError)?;
            write_imagenet_f32_le(&self.rgb_resized, out, &self.imagenet_lut);
        } else {
            let mut dst = Image::from_slice_u8(target_u32, target_u32, out, PixelType::U8x3)
                .map_err(|_| AtlasError::BufferError)?;
            self.resizer
                .resize(&src, &mut dst, &self.resize_options)
                .map_err(|_| AtlasError::BufferError)?;
        }
        Ok(())
    }
}

thread_local! {
    static THREAD_PREPROCESSOR: RefCell<VisionPreprocessor> = RefCell::new(VisionPreprocessor::new());
}

/// Zero-copy entry point used by the JNI bridge.
///
/// Uses a per-thread [`VisionPreprocessor`] (CameraX delivers `ImageAnalysis` frames on
/// a single executor thread), so scratch buffers are allocated once and reused.
#[allow(clippy::too_many_arguments)]
pub fn preprocess_yuv420_frame(
    y_plane: &[u8],
    u_plane: &[u8],
    v_plane: &[u8],
    src_width: usize,
    src_height: usize,
    y_row_stride: usize,
    uv_row_stride: usize,
    uv_pixel_stride: usize,
    target_size: usize,
    normalize_fp32: bool,
    out_buffer: &mut [u8],
) -> Result<(), AtlasError> {
    let mut run = |p: &mut VisionPreprocessor| {
        p.process(
            y_plane,
            u_plane,
            v_plane,
            src_width,
            src_height,
            y_row_stride,
            uv_row_stride,
            uv_pixel_stride,
            target_size,
            normalize_fp32,
            out_buffer,
        )
    };
    // `try_with` / `try_borrow_mut` keep this panic-free even during thread teardown.
    let cached = THREAD_PREPROCESSOR
        .try_with(|cell| cell.try_borrow_mut().ok().map(|mut p| run(&mut p)));
    match cached {
        Ok(Some(result)) => result,
        _ => run(&mut VisionPreprocessor::new()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const GUARD: usize = 64;
    const GUARD_BYTE: u8 = 0xAB;

    /// Synthetic `YUV_420_888` frame mimicking `ImageProxy` plane buffers.
    struct Frame {
        y: Vec<u8>,
        /// Planar: the U plane. Semi-planar: interleaved chroma buffer.
        u: Vec<u8>,
        /// Planar: the V plane. Semi-planar: unused.
        v: Vec<u8>,
        width: usize,
        height: usize,
        y_row_stride: usize,
        uv_row_stride: usize,
        uv_pixel_stride: usize,
    }

    impl Frame {
        /// `yuv(x, y)` gives the sample at full-resolution coordinates; chroma is
        /// taken at even coordinates. Rows carry stride padding filled with garbage.
        fn new(
            width: usize,
            height: usize,
            semi_planar: bool,
            row_padding: usize,
            yuv: impl Fn(usize, usize) -> (u8, u8, u8),
        ) -> Self {
            let cw = width.div_ceil(2);
            let ch = height.div_ceil(2);
            let y_row_stride = width + row_padding;
            let mut y = vec![0xEEu8; y_row_stride * height];
            for row in 0..height {
                for col in 0..width {
                    y[row * y_row_stride + col] = yuv(col, row).0;
                }
            }
            if semi_planar {
                // NV12: U at even offsets, V at odd offsets, like Android's pixelStride=2.
                let uv_row_stride = cw * 2 + row_padding;
                let mut uv = vec![0xEEu8; uv_row_stride * ch];
                for row in 0..ch {
                    for col in 0..cw {
                        let (_, u, v) = yuv(col * 2, row * 2);
                        uv[row * uv_row_stride + col * 2] = u;
                        uv[row * uv_row_stride + col * 2 + 1] = v;
                    }
                }
                Self {
                    y,
                    u: uv,
                    v: Vec::new(),
                    width,
                    height,
                    y_row_stride,
                    uv_row_stride,
                    uv_pixel_stride: 2,
                }
            } else {
                let uv_row_stride = cw + row_padding;
                let mut u = vec![0xEEu8; uv_row_stride * ch];
                let mut v = vec![0xEEu8; uv_row_stride * ch];
                for row in 0..ch {
                    for col in 0..cw {
                        let (_, us, vs) = yuv(col * 2, row * 2);
                        u[row * uv_row_stride + col] = us;
                        v[row * uv_row_stride + col] = vs;
                    }
                }
                Self {
                    y,
                    u,
                    v,
                    width,
                    height,
                    y_row_stride,
                    uv_row_stride,
                    uv_pixel_stride: 1,
                }
            }
        }

        /// Returns (Y, U, V) plane slices exactly as Android exposes them: the last
        /// row is trimmed of its padding, and for semi-planar U/V are offset views
        /// into the same interleaved memory.
        fn planes(&self) -> (&[u8], &[u8], &[u8]) {
            let y_len = (self.height - 1) * self.y_row_stride + self.width;
            let y = &self.y[..y_len];
            if self.uv_pixel_stride == 2 {
                let ch = self.height.div_ceil(2);
                let cw = self.width.div_ceil(2);
                let len = (ch - 1) * self.uv_row_stride + (cw - 1) * 2 + 1;
                (y, &self.u[..len], &self.u[1..1 + len])
            } else {
                (y, &self.u, &self.v)
            }
        }

        fn run(&self, target: usize, fp32: bool) -> Result<Vec<u8>, AtlasError> {
            let len = output_len(target, fp32).unwrap();
            let mut out = vec![GUARD_BYTE; len + GUARD];
            let (y, u, v) = self.planes();
            preprocess_yuv420_frame(
                y,
                u,
                v,
                self.width,
                self.height,
                self.y_row_stride,
                self.uv_row_stride,
                self.uv_pixel_stride,
                target,
                fp32,
                &mut out,
            )?;
            assert!(
                out[len..].iter().all(|&b| b == GUARD_BYTE),
                "wrote past the tensor end"
            );
            out.truncate(len);
            Ok(out)
        }
    }

    fn gradient(x: usize, y: usize) -> (u8, u8, u8) {
        (
            ((x * 255) / 639) as u8,
            ((y * 255) / 479) as u8,
            (255 - (x * 255) / 639) as u8,
        )
    }

    fn decode_f32_le(bytes: &[u8]) -> Vec<f32> {
        bytes
            .chunks_exact(4)
            .map(|b| f32::from_le_bytes([b[0], b[1], b[2], b[3]]))
            .collect()
    }

    #[test]
    fn nv12_640x480_to_224_u8() {
        let frame = Frame::new(640, 480, true, 64, gradient);
        let out = frame.run(224, false).expect("preprocess failed");
        assert_eq!(out.len(), 224 * 224 * 3);
        let (min, max) = out
            .iter()
            .fold((u8::MAX, u8::MIN), |(lo, hi), &v| (lo.min(v), hi.max(v)));
        assert!(max - min > 100, "gradient lost: range {min}..{max}");
        // On the middle row U ≈ 128, so blue tracks the left→right luma ramp.
        let row = &out[112 * 224 * 3..113 * 224 * 3];
        assert!(row[223 * 3 + 2] > row[2] + 100, "left→right gradient not preserved");
    }

    #[test]
    fn i420_640x480_to_224_fp32() {
        let frame = Frame::new(640, 480, false, 32, gradient);
        let out = frame.run(224, true).expect("preprocess failed");
        assert_eq!(out.len(), 224 * 224 * 3 * 4);
        let floats = decode_f32_le(&out);
        assert_eq!(floats.len(), 224 * 224 * 3);
        for (i, &f) in floats.iter().enumerate() {
            let c = i % 3;
            let lo = (0.0 - IMAGENET_MEAN[c]) / IMAGENET_STD[c] - 1e-5;
            let hi = (1.0 - IMAGENET_MEAN[c]) / IMAGENET_STD[c] + 1e-5;
            assert!(f.is_finite() && (lo..=hi).contains(&f), "value {f} out of range at {i}");
        }
    }

    #[test]
    fn fp32_output_matches_u8_output() {
        let frame = Frame::new(640, 480, true, 0, gradient);
        let u8_out = frame.run(224, false).unwrap();
        let f32_out = decode_f32_le(&frame.run(224, true).unwrap());
        let lut = build_imagenet_lut();
        for (i, (&b, &f)) in u8_out.iter().zip(&f32_out).enumerate() {
            assert_eq!(lut[i % 3][b as usize], f, "mismatch at element {i}");
        }
    }

    #[test]
    fn planar_and_semi_planar_agree() {
        let a = Frame::new(640, 480, true, 16, gradient).run(224, false).unwrap();
        let b = Frame::new(640, 480, false, 48, gradient).run(224, false).unwrap();
        assert_eq!(a, b);
    }

    #[test]
    fn center_crop_excludes_side_bands() {
        // Landscape: white bands left/right; portrait: white bands top/bottom.
        let landscape = Frame::new(640, 480, true, 0, |x, _| {
            if !(80..560).contains(&x) { (255, 128, 128) } else { (40, 128, 128) }
        });
        let portrait = Frame::new(480, 640, false, 0, |_, y| {
            if !(80..560).contains(&y) { (255, 128, 128) } else { (40, 128, 128) }
        });
        for frame in [landscape, portrait] {
            for target in [224, 480] {
                let out = frame.run(target, false).unwrap();
                assert!(
                    out.iter().all(|&v| v.abs_diff(40) <= 1),
                    "border leaked into crop (target {target})"
                );
            }
        }
    }

    #[test]
    fn handles_odd_dimensions_and_offsets() {
        // 643x481 → crop 481, x0 = 81 (odd chroma phase), y0 = 0.
        let frame = Frame::new(643, 481, true, 5, |x, y| ((x % 256) as u8, (y % 256) as u8, 77));
        frame.run(224, false).unwrap();
        frame.run(224, true).unwrap();
        let planar = Frame::new(481, 643, false, 3, |x, y| ((y % 256) as u8, 90, (x % 256) as u8));
        planar.run(300, false).unwrap();
    }

    #[test]
    fn bt601_reference_colors() {
        let close = |a: [u8; 3], b: [u8; 3]| a.iter().zip(&b).all(|(x, y)| x.abs_diff(*y) <= 2);
        assert_eq!(yuv_to_rgb_bt601(0, 128, 128), [0, 0, 0]);
        assert_eq!(yuv_to_rgb_bt601(255, 128, 128), [255, 255, 255]);
        assert_eq!(yuv_to_rgb_bt601(128, 128, 128), [128, 128, 128]);
        assert!(close(yuv_to_rgb_bt601(76, 85, 255), [255, 0, 0]));
        assert!(close(yuv_to_rgb_bt601(150, 44, 21), [0, 255, 0]));
        assert!(close(yuv_to_rgb_bt601(29, 255, 107), [0, 0, 255]));
        // Extreme chroma must clamp instead of wrapping.
        assert_eq!(yuv_to_rgb_bt601(255, 255, 255)[0], 255);
        assert_eq!(yuv_to_rgb_bt601(0, 0, 0)[2], 0);
    }

    #[test]
    fn rejects_invalid_input_without_panicking() {
        let frame = Frame::new(640, 480, true, 0, gradient);
        let (y, u, v) = frame.planes();
        let (w, h, ys, uvs) = (640, 480, frame.y_row_stride, frame.uv_row_stride);
        let mut out = vec![0u8; output_len(224, true).unwrap()];
        let call = |y: &[u8], u: &[u8], w, h, ys, uvs, ps, t, fp32, out: &mut [u8]| {
            preprocess_yuv420_frame(y, u, v, w, h, ys, uvs, ps, t, fp32, out).is_err()
        };
        // Output too small for fp32 by one byte.
        let short = out.len() - 1;
        assert!(call(y, u, w, h, ys, uvs, 2, 224, true, &mut out[..short]));
        // u8 tensor too small.
        assert!(call(y, u, w, h, ys, uvs, 2, 224, false, &mut out[..224 * 224 * 3 - 1]));
        // Planes are required only up to the last byte the 480×480 crop (x 80..560) reads:
        // exactly that length works, one byte less is rejected.
        let y_needed = 479 * ys + 560;
        let uv_needed = (479 >> 1) * uvs + (559 >> 1) * 2 + 1;
        assert!(!call(&y[..y_needed], &u[..uv_needed], w, h, ys, uvs, 2, 224, false, &mut out));
        assert!(call(&y[..y_needed - 1], u, w, h, ys, uvs, 2, 224, false, &mut out));
        assert!(call(y, &u[..uv_needed - 1], w, h, ys, uvs, 2, 224, false, &mut out));
        // Unsupported pixel stride, zero geometry, stride narrower than the row.
        assert!(call(y, u, w, h, ys, uvs, 3, 224, false, &mut out));
        assert!(call(y, u, w, h, ys, uvs, 0, 224, false, &mut out));
        assert!(call(y, u, 0, h, ys, uvs, 2, 224, false, &mut out));
        assert!(call(y, u, w, 0, ys, uvs, 2, 224, false, &mut out));
        assert!(call(y, u, w, h, ys, uvs, 2, 0, false, &mut out));
        assert!(call(y, u, w, h, 639, uvs, 2, 224, false, &mut out));
        assert!(call(y, u, w, h, ys, 100, 2, 224, false, &mut out));
        // Absurd sizes must fail validation, not overflow.
        assert!(call(y, u, usize::MAX, h, usize::MAX, uvs, 2, 224, false, &mut out));
        assert!(call(y, u, w, h, ys, uvs, 2, usize::MAX, false, &mut out));
    }
}
