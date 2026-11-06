//! Regenerates assets/RSnap.ico: viewfinder corners and a red crosshair on a dark rounded tile.
//! Usage: cargo run --example make_icon -- assets/RSnap.ico [preview.png]

use image::ImageEncoder;
use image::codecs::png::PngEncoder;

const SIZES: [u32; 9] = [16, 20, 24, 32, 40, 48, 64, 128, 256];
const SUPERSAMPLE: u32 = 4;

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let output = args
        .get(1)
        .expect("usage: make_icon <output.ico> [preview.png]");
    let pngs: Vec<Vec<u8>> = SIZES.iter().map(|&s| png(&render(s), s)).collect();
    if let Some(preview) = args.get(2) {
        std::fs::write(preview, pngs.last().unwrap()).unwrap();
    }

    let mut ico = Vec::new();
    ico.extend_from_slice(&[0, 0, 1, 0]);
    ico.extend_from_slice(&(SIZES.len() as u16).to_le_bytes());
    let mut offset = 6 + 16 * SIZES.len() as u32;
    for (size, data) in SIZES.iter().zip(&pngs) {
        let dim = if *size == 256 { 0 } else { *size as u8 };
        ico.extend_from_slice(&[dim, dim, 0, 0, 1, 0, 32, 0]);
        ico.extend_from_slice(&(data.len() as u32).to_le_bytes());
        ico.extend_from_slice(&offset.to_le_bytes());
        offset += data.len() as u32;
    }
    for data in &pngs {
        ico.extend_from_slice(data);
    }
    std::fs::write(output, ico).unwrap();
}

fn png(rgba: &[u8], size: u32) -> Vec<u8> {
    let mut out = Vec::new();
    PngEncoder::new(&mut out)
        .write_image(rgba, size, size, image::ExtendedColorType::Rgba8)
        .unwrap();
    out
}

fn render(size: u32) -> Vec<u8> {
    let mut out = Vec::with_capacity((size * size * 4) as usize);
    let samples = (SUPERSAMPLE * SUPERSAMPLE) as f32;
    for y in 0..size {
        for x in 0..size {
            let mut sum = [0f32; 4];
            for sy in 0..SUPERSAMPLE {
                for sx in 0..SUPERSAMPLE {
                    let u = (x as f32 + (sx as f32 + 0.5) / SUPERSAMPLE as f32) / size as f32;
                    let v = (y as f32 + (sy as f32 + 0.5) / SUPERSAMPLE as f32) / size as f32;
                    let c = shade(u, v, size);
                    sum[0] += c[0] * c[3];
                    sum[1] += c[1] * c[3];
                    sum[2] += c[2] * c[3];
                    sum[3] += c[3];
                }
            }
            let alpha = sum[3] / samples;
            let channel = |i: usize| {
                if sum[3] > 0.0 {
                    (sum[i] / sum[3] * 255.0).round() as u8
                } else {
                    0
                }
            };
            out.extend_from_slice(&[
                channel(0),
                channel(1),
                channel(2),
                (alpha * 255.0).round() as u8,
            ]);
        }
    }
    out
}

fn rounded_box(u: f32, v: f32, half_w: f32, half_h: f32, radius: f32, cx: f32, cy: f32) -> f32 {
    let dx = (u - cx).abs() - half_w + radius;
    let dy = (v - cy).abs() - half_h + radius;
    let outside = (dx.max(0.0).powi(2) + dy.max(0.0).powi(2)).sqrt();
    outside + dx.max(dy).min(0.0) - radius
}

fn shade(u: f32, v: f32, size: u32) -> [f32; 4] {
    if rounded_box(u, v, 0.5, 0.5, 0.22, 0.5, 0.5) > 0.0 {
        return [0.0; 4];
    }
    // Small icons need heavier strokes to stay legible.
    let weight = if size <= 24 { 0.095 } else { 0.07 };
    let (a, b) = (0.2, 0.8);
    let arm = 0.2;
    let mut bracket = false;
    for &(cx, sx) in &[(a, 1.0), (b, -1.0)] {
        for &(cy, sy) in &[(a, 1.0), (b, -1.0)] {
            let along_x = (u - cx) * sx;
            let along_y = (v - cy) * sy;
            let horizontal = (0.0..=arm).contains(&along_x) && (0.0..=weight).contains(&along_y);
            let vertical = (0.0..=arm).contains(&along_y) && (0.0..=weight).contains(&along_x);
            bracket |= horizontal || vertical;
        }
    }
    if bracket {
        return [1.0, 1.0, 1.0, 1.0];
    }
    let half_len = 0.17;
    let half_w = weight / 2.0;
    let (du, dv) = ((u - 0.5).abs(), (v - 0.5).abs());
    if (du <= half_len && dv <= half_w) || (dv <= half_len && du <= half_w) {
        return [0.96, 0.23, 0.21, 1.0];
    }
    let t = (u + v) / 2.0;
    [0.09 + 0.06 * t, 0.11 + 0.07 * t, 0.17 + 0.1 * t, 1.0]
}
