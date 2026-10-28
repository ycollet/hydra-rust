use std::sync::OnceLock;

use ab_glyph::{Font, FontArc, PxScale, ScaleFont};

pub struct TextData {
    pub pixels: Vec<u8>,
    pub width: u32,
    pub height: u32,
}

const TEX_SIZE: u32 = 512;
const HACK_FONT: &[u8] = include_bytes!("fonts/Hack-Regular.ttf");

#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum TextMode {
    Fill,
    Stroke,
    FillStroke,
}

fn default_font() -> &'static FontArc {
    static FONT: OnceLock<FontArc> = OnceLock::new();
    FONT.get_or_init(|| FontArc::try_from_slice(HACK_FONT).expect("valid font"))
}

pub(crate) fn rasterize(text: &str, mode: TextMode) -> TextData {
    let font = default_font();
    let mut pixels = vec![0u8; (TEX_SIZE * TEX_SIZE * 4) as usize];

    let lines: Vec<&str> = text.lines().collect();
    let line_count = lines.len().max(1);

    let scale = auto_scale(font, &lines, line_count);
    let scaled = font.as_scaled(scale);
    let line_height = scaled.height();
    let ascent = scaled.ascent();

    let total_height = line_height * line_count as f32;
    let y_start = (TEX_SIZE as f32 - total_height) / 2.0 + ascent;

    for (li, line) in lines.iter().enumerate() {
        let y = y_start + li as f32 * line_height;
        let line_width: f32 = line
            .chars()
            .map(|c| scaled.h_advance(font.glyph_id(c)))
            .sum();
        let x_start = (TEX_SIZE as f32 - line_width) / 2.0;

        let mut x = x_start;
        for ch in line.chars() {
            let gid = font.glyph_id(ch);
            let glyph = gid.with_scale_and_position(scale, ab_glyph::point(x, y));
            if let Some(outlined) = font.outline_glyph(glyph) {
                let bounds = outlined.px_bounds();
                outlined.draw(|px, py, cov| {
                    let a = (cov * 255.0) as u8;
                    if a == 0 {
                        return;
                    }
                    let gx = bounds.min.x as i32 + px as i32;
                    let gy = bounds.min.y as i32 + py as i32;
                    if gx >= 0 && gx < TEX_SIZE as i32 && gy >= 0 && gy < TEX_SIZE as i32 {
                        let i = ((gy as u32 * TEX_SIZE + gx as u32) * 4) as usize;
                        pixels[i] = 255;
                        pixels[i + 1] = 255;
                        pixels[i + 2] = 255;
                        pixels[i + 3] = pixels[i + 3].saturating_add(a);
                    }
                });
            }
            x += scaled.h_advance(gid);
        }
    }

    if mode != TextMode::Fill {
        // A centered outline includes both outer contours and counters (letter holes).
        let radius = (scale.y * 0.01).round().max(1.0) as i32;
        let alpha: Vec<u8> = pixels.chunks(4).map(|p| p[3]).collect();
        for y in 0..TEX_SIZE as i32 {
            for x in 0..TEX_SIZE as i32 {
                let mut low = 255;
                let mut high = 0;
                for dy in -radius..=radius {
                    for dx in -radius..=radius {
                        if dx * dx + dy * dy > radius * radius {
                            continue;
                        }
                        let (nx, ny) = (x + dx, y + dy);
                        let coverage = if (0..TEX_SIZE as i32).contains(&nx)
                            && (0..TEX_SIZE as i32).contains(&ny)
                        {
                            alpha[(ny as u32 * TEX_SIZE + nx as u32) as usize]
                        } else {
                            0
                        };
                        low = low.min(coverage);
                        high = high.max(coverage);
                    }
                }
                let index = (y as u32 * TEX_SIZE + x as u32) as usize;
                let stroke = high - low;
                let coverage = if mode == TextMode::FillStroke {
                    let fill = u16::from(alpha[index]);
                    (fill + u16::from(stroke) * (255 - fill) / 255) as u8
                } else {
                    stroke
                };
                let pixel = if coverage == 0 {
                    [0, 0, 0, 0]
                } else {
                    [255, 255, 255, coverage]
                };
                pixels[index * 4..index * 4 + 4].copy_from_slice(&pixel);
            }
        }
    }

    TextData {
        pixels,
        width: TEX_SIZE,
        height: TEX_SIZE,
    }
}

fn auto_scale(font: &FontArc, lines: &[&str], line_count: usize) -> PxScale {
    let target_w = TEX_SIZE as f32 * 0.9;
    let max_h = TEX_SIZE as f32 / line_count as f32;
    let mut px = max_h * 0.85;

    for line in lines {
        if line.is_empty() {
            continue;
        }
        let scaled = font.as_scaled(PxScale::from(px));
        let w: f32 = line
            .chars()
            .map(|c| scaled.h_advance(font.glyph_id(c)))
            .sum();
        if w > target_w {
            px *= target_w / w;
        }
    }

    PxScale::from(px.min(max_h * 0.85))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rasterize_produces_visible_output() {
        let data = rasterize("HYDRA", TextMode::Fill);
        let non_zero: usize = data.pixels.chunks(4).filter(|px| px[3] > 0).count();
        assert!(non_zero > 100, "rasterizer must produce visible text");
    }

    #[test]
    fn stroke_hollows_glyphs_and_combined_mode_preserves_fill() {
        let fill = rasterize("O", TextMode::Fill);
        let stroke = rasterize("O", TextMode::Stroke);
        let combined = rasterize("O", TextMode::FillStroke);
        let mut hollowed = 0;
        let mut expanded = 0;
        for ((fill, stroke), combined) in fill
            .pixels
            .chunks(4)
            .zip(stroke.pixels.chunks(4))
            .zip(combined.pixels.chunks(4))
        {
            hollowed += usize::from(fill[3] == 255 && stroke[3] == 0);
            expanded += usize::from(fill[3] == 0 && stroke[3] > 0);
            assert!(combined[3] >= fill[3] && combined[3] >= stroke[3]);
        }
        assert!(hollowed > 100, "strokes must not fill glyph interiors");
        assert!(
            expanded > 100,
            "strokes must extend beyond the fill boundary"
        );
    }

    #[test]
    fn empty_text_has_no_outline_artifacts() {
        for mode in [TextMode::Stroke, TextMode::FillStroke] {
            assert!(rasterize("", mode).pixels.iter().all(|value| *value == 0));
        }
    }
}
