//! TRserial's original mark, a white T on a blue square. The tray uses the
//! pixels directly and `build.rs` turns them into the Windows executable icon,
//! so both stay identical. No game assets are bundled.

const BLUE: [u8; 3] = [37, 99, 235];
const WHITE: [u8; 3] = [255, 255, 255];
/// Rectangles on a 32-unit grid as `[x0, x1, y0, y1)`.
const SQUARE: [f32; 4] = [2., 30., 2., 30.];
const BAR: [f32; 4] = [7., 25., 8., 13.];
const STEM: [f32; 4] = [13., 19., 8., 25.];
const SAMPLES: u32 = 4;

fn inside(r: [f32; 4], x: f32, y: f32) -> bool {
    (r[0]..r[1]).contains(&x) && (r[2]..r[3]).contains(&y)
}

/// Straight-alpha RGBA rows, top first. Each pixel averages a 4×4 grid so
/// sizes that are not multiples of 32 keep smooth edges; at 32 px every edge
/// falls on a pixel boundary and the result is exact.
pub fn rgba(size: u32) -> Vec<u8> {
    let scale = 32. / size as f32;
    let mut pixels = Vec::with_capacity((size * size * 4) as usize);
    for py in 0..size {
        for px in 0..size {
            let (mut covered, mut white) = (0, 0);
            for sy in 0..SAMPLES {
                for sx in 0..SAMPLES {
                    let x = (px as f32 + (sx as f32 + 0.5) / SAMPLES as f32) * scale;
                    let y = (py as f32 + (sy as f32 + 0.5) / SAMPLES as f32) * scale;
                    if inside(SQUARE, x, y) {
                        covered += 1;
                        white += u32::from(inside(BAR, x, y) || inside(STEM, x, y));
                    }
                }
            }
            if covered == 0 {
                pixels.extend_from_slice(&[0; 4]);
                continue;
            }
            let mix = |c: usize| {
                ((u32::from(BLUE[c]) * (covered - white) + u32::from(WHITE[c]) * white) / covered)
                    as u8
            };
            let alpha = (covered * 255 / (SAMPLES * SAMPLES)) as u8;
            pixels.extend_from_slice(&[mix(0), mix(1), mix(2), alpha]);
        }
    }
    pixels
}

/// A Windows `.ico` with one 32-bit bitmap per size, so the shell never has to
/// scale a different size.
#[cfg_attr(not(test), allow(dead_code, reason = "used by build.rs"))]
pub fn ico(sizes: &[u32]) -> Vec<u8> {
    let images: Vec<Vec<u8>> = sizes.iter().map(|&size| bitmap(size)).collect();
    let mut out = Vec::new();
    for value in [0u16, 1, sizes.len() as u16] {
        out.extend_from_slice(&value.to_le_bytes());
    }
    let mut offset = 6 + 16 * sizes.len();
    for (&size, image) in sizes.iter().zip(&images) {
        // A zero dimension means 256 in the icon directory.
        let dimension = if size >= 256 { 0 } else { size as u8 };
        out.extend_from_slice(&[dimension, dimension, 0, 0]);
        out.extend_from_slice(&1u16.to_le_bytes());
        out.extend_from_slice(&32u16.to_le_bytes());
        out.extend_from_slice(&(image.len() as u32).to_le_bytes());
        out.extend_from_slice(&(offset as u32).to_le_bytes());
        offset += image.len();
    }
    images.into_iter().for_each(|image| out.extend(image));
    out
}

/// BITMAPINFOHEADER, bottom-up BGRA rows, then an empty AND mask (alpha wins).
#[cfg_attr(not(test), allow(dead_code, reason = "used by build.rs"))]
fn bitmap(size: u32) -> Vec<u8> {
    let rgba = rgba(size);
    let mut out = Vec::new();
    // Icon bitmaps declare twice the height to cover the AND mask.
    for value in [40, size, size * 2] {
        out.extend_from_slice(&value.to_le_bytes());
    }
    out.extend_from_slice(&1u16.to_le_bytes());
    out.extend_from_slice(&32u16.to_le_bytes());
    out.extend_from_slice(&[0; 24]);
    for row in rgba.chunks_exact(size as usize * 4).rev() {
        for p in row.chunks_exact(4) {
            out.extend_from_slice(&[p[2], p[1], p[0], p[3]]);
        }
    }
    let mask_row = size.div_ceil(32) * 4;
    out.resize(out.len() + (mask_row * size) as usize, 0);
    out
}

#[cfg(test)]
mod tests {
    #[test]
    fn icon_directory_points_at_each_complete_bitmap() {
        let sizes = [16, 48, 256];
        let ico = super::ico(&sizes);
        let u16_at = |i: usize| u16::from_le_bytes([ico[i], ico[i + 1]]);
        let u32_at = |i: usize| u32::from_le_bytes(ico[i..i + 4].try_into().unwrap());
        assert_eq!((u16_at(0), u16_at(2), u16_at(4)), (0, 1, 3));
        let mut end = 6 + 16 * sizes.len();
        for (n, size) in sizes.into_iter().enumerate() {
            let entry = 6 + 16 * n;
            let (len, offset) = (u32_at(entry + 8) as usize, u32_at(entry + 12) as usize);
            assert_eq!(ico[entry], if size == 256 { 0 } else { size as u8 });
            assert_eq!(offset, end, "bitmaps are stored back to back");
            assert_eq!((u32_at(offset + 4), u32_at(offset + 8)), (size, size * 2));
            let pixels = size * size * 4 + size.div_ceil(32) * 4 * size;
            assert_eq!(len, 40 + pixels as usize);
            end = offset + len;
        }
        assert_eq!(end, ico.len());
    }
}
