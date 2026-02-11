//! Annotation primitives, editor for screenshot markup, and CPU rendering.

use ns_common::PixelBuffer;
use serde::{Deserialize, Serialize};
use std::sync::Arc;

/// Color in RGBA.
#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub struct Color {
    pub r: f32,
    pub g: f32,
    pub b: f32,
    pub a: f32,
}

impl Color {
    pub const RED: Self = Self { r: 1.0, g: 0.0, b: 0.0, a: 1.0 };
    pub const GREEN: Self = Self { r: 0.0, g: 0.75, b: 0.0, a: 1.0 };
    pub const BLUE: Self = Self { r: 0.2, g: 0.4, b: 1.0, a: 1.0 };
    pub const YELLOW: Self = Self { r: 1.0, g: 1.0, b: 0.0, a: 0.5 };
    pub const WHITE: Self = Self { r: 1.0, g: 1.0, b: 1.0, a: 1.0 };
    pub const BLACK: Self = Self { r: 0.0, g: 0.0, b: 0.0, a: 1.0 };
    pub const ORANGE: Self = Self { r: 1.0, g: 0.55, b: 0.0, a: 1.0 };
    pub const CYAN: Self = Self { r: 0.0, g: 0.8, b: 0.9, a: 1.0 };
    pub const PURPLE: Self = Self { r: 0.6, g: 0.2, b: 0.9, a: 1.0 };
    pub const YELLOW_OPAQUE: Self = Self { r: 1.0, g: 1.0, b: 0.0, a: 1.0 };

    /// Palette of stroke colors for the annotation editor.
    pub const PALETTE: [Self; 9] = [
        Self::RED, Self::ORANGE, Self::YELLOW_OPAQUE, Self::GREEN,
        Self::CYAN, Self::BLUE, Self::PURPLE, Self::WHITE, Self::BLACK,
    ];

    /// Derive a highlight color from a stroke color (semi-transparent).
    pub fn as_highlight(&self) -> Self {
        Self { r: self.r, g: self.g, b: self.b, a: 0.35 }
    }
}

impl PartialEq for Color {
    fn eq(&self, other: &Self) -> bool {
        (self.r - other.r).abs() < 0.01
            && (self.g - other.g).abs() < 0.01
            && (self.b - other.b).abs() < 0.01
            && (self.a - other.a).abs() < 0.01
    }
}

/// A 2D point.
#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub struct Point {
    pub x: f32,
    pub y: f32,
}

/// An annotation shape drawn on top of a screenshot.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum Annotation {
    /// Arrow from start to end.
    Arrow {
        start: Point,
        end: Point,
        color: Color,
        thickness: f32,
    },
    /// Rectangle outline.
    Rectangle {
        x: f32,
        y: f32,
        width: f32,
        height: f32,
        color: Color,
        thickness: f32,
    },
    /// Highlight (filled semi-transparent rectangle).
    Highlight {
        x: f32,
        y: f32,
        width: f32,
        height: f32,
        color: Color,
    },
    /// Blur region.
    Blur {
        x: f32,
        y: f32,
        width: f32,
        height: f32,
        radius: f32,
    },
    /// Text label.
    Text {
        position: Point,
        text: String,
        color: Color,
        font_size: f32,
    },
    /// Freehand pen stroke.
    Pen {
        points: Vec<Point>,
        color: Color,
        thickness: f32,
    },
}

/// Collection of annotations on a single screenshot.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct AnnotationLayer {
    pub annotations: Vec<Annotation>,
}

impl AnnotationLayer {
    pub fn new() -> Self {
        Self {
            annotations: Vec::new(),
        }
    }

    pub fn add(&mut self, annotation: Annotation) {
        self.annotations.push(annotation);
    }

    pub fn undo(&mut self) -> Option<Annotation> {
        self.annotations.pop()
    }

    pub fn clear(&mut self) {
        self.annotations.clear();
    }

    pub fn is_empty(&self) -> bool {
        self.annotations.is_empty()
    }
}

/// The annotation tool currently selected.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum AnnotationTool {
    Arrow,
    Rectangle,
    Highlight,
    Blur,
    Text,
    Pen,
}

// ─── CPU annotation rendering ────────────────────────────────────────────────

/// Render annotations onto a PixelBuffer, returning a new buffer with annotations baked in.
/// The input buffer is BGRA format. Annotations are rendered using the same geometry
/// as the D2D editor preview.
pub fn render_annotations(pixels: &PixelBuffer, layer: &AnnotationLayer) -> PixelBuffer {
    let w = pixels.width;
    let h = pixels.height;
    let stride = w * 4; // tight stride for output

    let mut data = vec![0u8; (stride * h) as usize];

    // Copy original pixels (handle stride difference)
    let src_stride = pixels.stride as usize;
    let dst_stride = stride as usize;
    let row_bytes = (w * 4) as usize;
    for row in 0..h as usize {
        let src_start = row * src_stride;
        let dst_start = row * dst_stride;
        data[dst_start..dst_start + row_bytes]
            .copy_from_slice(&pixels.data[src_start..src_start + row_bytes]);
    }

    // Render each annotation in order
    for ann in &layer.annotations {
        render_annotation(&mut data, w, h, stride, ann);
    }

    PixelBuffer {
        data: Arc::new(data),
        width: w,
        height: h,
        stride,
    }
}

fn render_annotation(data: &mut [u8], buf_w: u32, buf_h: u32, stride: u32, ann: &Annotation) {
    match ann {
        Annotation::Arrow {
            start,
            end,
            color,
            thickness,
        } => {
            draw_thick_line(data, buf_w, buf_h, stride, start.x, start.y, end.x, end.y, color, *thickness);
            // Arrowhead
            let dx = end.x - start.x;
            let dy = end.y - start.y;
            let len = (dx * dx + dy * dy).sqrt();
            if len > 5.0 {
                let nx = dx / len;
                let ny = dy / len;
                let head_len = (*thickness * 4.0).min(len * 0.3);
                let head_w = head_len * 0.5;
                let left_x = end.x - nx * head_len + ny * head_w;
                let left_y = end.y - ny * head_len - nx * head_w;
                let right_x = end.x - nx * head_len - ny * head_w;
                let right_y = end.y - ny * head_len + nx * head_w;
                draw_thick_line(data, buf_w, buf_h, stride, end.x, end.y, left_x, left_y, color, *thickness * 0.7);
                draw_thick_line(data, buf_w, buf_h, stride, end.x, end.y, right_x, right_y, color, *thickness * 0.7);
            }
        }

        Annotation::Rectangle {
            x,
            y,
            width: rw,
            height: rh,
            color,
            thickness,
        } => {
            let x0 = *x;
            let y0 = *y;
            let x1 = *x + *rw;
            let y1 = *y + *rh;
            draw_thick_line(data, buf_w, buf_h, stride, x0, y0, x1, y0, color, *thickness);
            draw_thick_line(data, buf_w, buf_h, stride, x1, y0, x1, y1, color, *thickness);
            draw_thick_line(data, buf_w, buf_h, stride, x1, y1, x0, y1, color, *thickness);
            draw_thick_line(data, buf_w, buf_h, stride, x0, y1, x0, y0, color, *thickness);
        }

        Annotation::Highlight {
            x,
            y,
            width: rw,
            height: rh,
            color,
        } => {
            fill_rect_blended(data, buf_w, buf_h, stride, *x, *y, *rw, *rh, color);
        }

        Annotation::Pen {
            points,
            color,
            thickness,
        } => {
            for pair in points.windows(2) {
                draw_thick_line(
                    data, buf_w, buf_h, stride,
                    pair[0].x, pair[0].y, pair[1].x, pair[1].y,
                    color, *thickness,
                );
            }
        }

        Annotation::Blur {
            x,
            y,
            width: rw,
            height: rh,
            radius,
        } => {
            pixelate_region(data, buf_w, buf_h, stride, *x, *y, *rw, *rh, *radius);
        }

        Annotation::Text {
            position,
            text,
            color,
            font_size,
        } => {
            draw_text_simple(data, buf_w, buf_h, stride, position.x, position.y, text, color, *font_size);
        }
    }
}

/// Draw a thick line from (x0,y0) to (x1,y1) using distance²-based rendering (no sqrt).
fn draw_thick_line(
    data: &mut [u8], buf_w: u32, buf_h: u32, stride: u32,
    x0: f32, y0: f32, x1: f32, y1: f32,
    color: &Color, thickness: f32,
) {
    let half_t = thickness / 2.0 + 0.5; // +0.5 for anti-alias margin
    let half_t_sq = (thickness / 2.0) * (thickness / 2.0); // compare dist² against this
    let min_px = ((x0.min(x1) - half_t).floor() as i32).max(0);
    let max_px = ((x0.max(x1) + half_t).ceil() as i32).min(buf_w as i32 - 1);
    let min_py = ((y0.min(y1) - half_t).floor() as i32).max(0);
    let max_py = ((y0.max(y1) + half_t).ceil() as i32).min(buf_h as i32 - 1);

    let dx = x1 - x0;
    let dy = y1 - y0;
    let len_sq = dx * dx + dy * dy;

    let (b, g, r, a) = color_to_bgra(color);

    // Fast path for opaque colors: skip blend_pixel overhead
    let is_opaque = a == 255;
    let stride_usize = stride as usize;

    for py in min_py..=max_py {
        let fy = py as f32 + 0.5;
        let row_base = py as usize * stride_usize;

        for px in min_px..=max_px {
            let fx = px as f32 + 0.5;

            // Distance² from point to line segment (no sqrt!)
            let dist_sq = if len_sq < 0.001 {
                (fx - x0) * (fx - x0) + (fy - y0) * (fy - y0)
            } else {
                let t = ((fx - x0) * dx + (fy - y0) * dy) / len_sq;
                let t = t.clamp(0.0, 1.0);
                let near_x = x0 + t * dx;
                let near_y = y0 + t * dy;
                (fx - near_x) * (fx - near_x) + (fy - near_y) * (fy - near_y)
            };

            if dist_sq <= half_t_sq {
                let off = row_base + px as usize * 4;
                if is_opaque {
                    // Direct write — no blending needed
                    if off + 3 < data.len() {
                        data[off] = b;
                        data[off + 1] = g;
                        data[off + 2] = r;
                        data[off + 3] = 255;
                    }
                } else {
                    blend_pixel(data, stride, px as u32, py as u32, b, g, r, a);
                }
            }
        }
    }
}

/// Fill a rectangle with alpha blending. Fast path for opaque colors.
fn fill_rect_blended(
    data: &mut [u8], buf_w: u32, buf_h: u32, stride: u32,
    x: f32, y: f32, w: f32, h: f32,
    color: &Color,
) {
    let min_px = (x.floor() as i32).max(0);
    let max_px = ((x + w).ceil() as i32).min(buf_w as i32 - 1);
    let min_py = (y.floor() as i32).max(0);
    let max_py = ((y + h).ceil() as i32).min(buf_h as i32 - 1);

    let (b, g, r, a) = color_to_bgra(color);
    let stride_usize = stride as usize;

    if a == 255 {
        // Opaque: direct write, no blending
        let pixel = [b, g, r, 255u8];
        for py in min_py..=max_py {
            let row_base = py as usize * stride_usize;
            for px in min_px..=max_px {
                let off = row_base + px as usize * 4;
                if off + 3 < data.len() {
                    data[off..off + 4].copy_from_slice(&pixel);
                }
            }
        }
    } else {
        for py in min_py..=max_py {
            for px in min_px..=max_px {
                blend_pixel(data, stride, px as u32, py as u32, b, g, r, a);
            }
        }
    }
}

/// Pixelate a region by averaging NxN blocks. Uses row-wise bulk fill.
fn pixelate_region(
    data: &mut [u8], buf_w: u32, buf_h: u32, stride: u32,
    x: f32, y: f32, w: f32, h: f32, radius: f32,
) {
    let block_size = (radius as i32).max(4);
    let x0 = (x.floor() as i32).max(0);
    let y0 = (y.floor() as i32).max(0);
    let x1 = ((x + w).ceil() as i32).min(buf_w as i32);
    let y1 = ((y + h).ceil() as i32).min(buf_h as i32);
    let stride_usize = stride as usize;

    let mut by = y0;
    while by < y1 {
        let mut bx = x0;
        let bh = block_size.min(y1 - by);
        while bx < x1 {
            let bw = block_size.min(x1 - bx);

            // Average the block
            let mut sum_b: u64 = 0;
            let mut sum_g: u64 = 0;
            let mut sum_r: u64 = 0;
            let mut count: u64 = 0;

            for py in by..by + bh {
                let row_base = py as usize * stride_usize;
                for px in bx..bx + bw {
                    let off = row_base + px as usize * 4;
                    sum_b += data[off] as u64;
                    sum_g += data[off + 1] as u64;
                    sum_r += data[off + 2] as u64;
                    count += 1;
                }
            }

            if count > 0 {
                let avg_pixel = [
                    (sum_b / count) as u8,
                    (sum_g / count) as u8,
                    (sum_r / count) as u8,
                    255u8,
                ];

                // Fill each row of the block with bulk writes
                for py in by..by + bh {
                    let row_base = py as usize * stride_usize + bx as usize * 4;
                    let row_end = row_base + bw as usize * 4;
                    for off in (row_base..row_end).step_by(4) {
                        data[off..off + 4].copy_from_slice(&avg_pixel);
                    }
                }
            }

            bx += block_size;
        }
        by += block_size;
    }
}

/// Simple text rendering using a built-in 5x7 bitmap font.
fn draw_text_simple(
    data: &mut [u8], buf_w: u32, buf_h: u32, stride: u32,
    x: f32, y: f32, text: &str,
    color: &Color, font_size: f32,
) {
    let scale = (font_size / 12.0).max(1.0).round() as u32;
    let (b, g, r, a) = color_to_bgra(color);
    let mut cx = x as i32;
    let cy = y as i32;

    for ch in text.chars() {
        let glyph = get_glyph(ch);
        for (row, bits) in glyph.iter().enumerate() {
            for col in 0..5u32 {
                if bits & (1 << (4 - col)) != 0 {
                    // Draw a scale x scale block for each pixel
                    for sy in 0..scale {
                        for sx in 0..scale {
                            let px = cx + (col * scale + sx) as i32;
                            let py = cy + (row as u32 * scale + sy) as i32;
                            if px >= 0 && px < buf_w as i32 && py >= 0 && py < buf_h as i32 {
                                blend_pixel(data, stride, px as u32, py as u32, b, g, r, a);
                            }
                        }
                    }
                }
            }
        }
        cx += (6 * scale) as i32; // 5px char + 1px spacing
    }
}

/// 5x7 bitmap font glyphs for basic ASCII.
fn get_glyph(ch: char) -> [u8; 7] {
    match ch {
        'A' | 'a' => [0b01110, 0b10001, 0b10001, 0b11111, 0b10001, 0b10001, 0b10001],
        'B' | 'b' => [0b11110, 0b10001, 0b10001, 0b11110, 0b10001, 0b10001, 0b11110],
        'C' | 'c' => [0b01110, 0b10001, 0b10000, 0b10000, 0b10000, 0b10001, 0b01110],
        'D' | 'd' => [0b11110, 0b10001, 0b10001, 0b10001, 0b10001, 0b10001, 0b11110],
        'E' | 'e' => [0b11111, 0b10000, 0b10000, 0b11110, 0b10000, 0b10000, 0b11111],
        'F' | 'f' => [0b11111, 0b10000, 0b10000, 0b11110, 0b10000, 0b10000, 0b10000],
        'G' | 'g' => [0b01110, 0b10001, 0b10000, 0b10111, 0b10001, 0b10001, 0b01110],
        'H' | 'h' => [0b10001, 0b10001, 0b10001, 0b11111, 0b10001, 0b10001, 0b10001],
        'I' | 'i' => [0b01110, 0b00100, 0b00100, 0b00100, 0b00100, 0b00100, 0b01110],
        'J' | 'j' => [0b00111, 0b00010, 0b00010, 0b00010, 0b00010, 0b10010, 0b01100],
        'K' | 'k' => [0b10001, 0b10010, 0b10100, 0b11000, 0b10100, 0b10010, 0b10001],
        'L' | 'l' => [0b10000, 0b10000, 0b10000, 0b10000, 0b10000, 0b10000, 0b11111],
        'M' | 'm' => [0b10001, 0b11011, 0b10101, 0b10101, 0b10001, 0b10001, 0b10001],
        'N' | 'n' => [0b10001, 0b11001, 0b10101, 0b10011, 0b10001, 0b10001, 0b10001],
        'O' | 'o' => [0b01110, 0b10001, 0b10001, 0b10001, 0b10001, 0b10001, 0b01110],
        'P' | 'p' => [0b11110, 0b10001, 0b10001, 0b11110, 0b10000, 0b10000, 0b10000],
        'Q' | 'q' => [0b01110, 0b10001, 0b10001, 0b10001, 0b10101, 0b10010, 0b01101],
        'R' | 'r' => [0b11110, 0b10001, 0b10001, 0b11110, 0b10100, 0b10010, 0b10001],
        'S' | 's' => [0b01110, 0b10001, 0b10000, 0b01110, 0b00001, 0b10001, 0b01110],
        'T' | 't' => [0b11111, 0b00100, 0b00100, 0b00100, 0b00100, 0b00100, 0b00100],
        'U' | 'u' => [0b10001, 0b10001, 0b10001, 0b10001, 0b10001, 0b10001, 0b01110],
        'V' | 'v' => [0b10001, 0b10001, 0b10001, 0b10001, 0b01010, 0b01010, 0b00100],
        'W' | 'w' => [0b10001, 0b10001, 0b10001, 0b10101, 0b10101, 0b11011, 0b10001],
        'X' | 'x' => [0b10001, 0b10001, 0b01010, 0b00100, 0b01010, 0b10001, 0b10001],
        'Y' | 'y' => [0b10001, 0b10001, 0b01010, 0b00100, 0b00100, 0b00100, 0b00100],
        'Z' | 'z' => [0b11111, 0b00001, 0b00010, 0b00100, 0b01000, 0b10000, 0b11111],
        '0' => [0b01110, 0b10001, 0b10011, 0b10101, 0b11001, 0b10001, 0b01110],
        '1' => [0b00100, 0b01100, 0b00100, 0b00100, 0b00100, 0b00100, 0b01110],
        '2' => [0b01110, 0b10001, 0b00001, 0b00110, 0b01000, 0b10000, 0b11111],
        '3' => [0b01110, 0b10001, 0b00001, 0b00110, 0b00001, 0b10001, 0b01110],
        '4' => [0b00010, 0b00110, 0b01010, 0b10010, 0b11111, 0b00010, 0b00010],
        '5' => [0b11111, 0b10000, 0b11110, 0b00001, 0b00001, 0b10001, 0b01110],
        '6' => [0b01110, 0b10000, 0b10000, 0b11110, 0b10001, 0b10001, 0b01110],
        '7' => [0b11111, 0b00001, 0b00010, 0b00100, 0b00100, 0b00100, 0b00100],
        '8' => [0b01110, 0b10001, 0b10001, 0b01110, 0b10001, 0b10001, 0b01110],
        '9' => [0b01110, 0b10001, 0b10001, 0b01111, 0b00001, 0b00001, 0b01110],
        ' ' => [0b00000, 0b00000, 0b00000, 0b00000, 0b00000, 0b00000, 0b00000],
        '.' => [0b00000, 0b00000, 0b00000, 0b00000, 0b00000, 0b00000, 0b00100],
        ',' => [0b00000, 0b00000, 0b00000, 0b00000, 0b00000, 0b00100, 0b01000],
        '!' => [0b00100, 0b00100, 0b00100, 0b00100, 0b00100, 0b00000, 0b00100],
        '?' => [0b01110, 0b10001, 0b00010, 0b00100, 0b00100, 0b00000, 0b00100],
        '-' => [0b00000, 0b00000, 0b00000, 0b11111, 0b00000, 0b00000, 0b00000],
        ':' => [0b00000, 0b00100, 0b00000, 0b00000, 0b00000, 0b00100, 0b00000],
        '(' => [0b00010, 0b00100, 0b01000, 0b01000, 0b01000, 0b00100, 0b00010],
        ')' => [0b01000, 0b00100, 0b00010, 0b00010, 0b00010, 0b00100, 0b01000],
        _ => [0b11111, 0b10001, 0b10001, 0b10001, 0b10001, 0b10001, 0b11111], // box for unknown
    }
}

/// Convert Color (f32 RGBA) to BGRA bytes.
fn color_to_bgra(c: &Color) -> (u8, u8, u8, u8) {
    let r = (c.r * 255.0).round().clamp(0.0, 255.0) as u8;
    let g = (c.g * 255.0).round().clamp(0.0, 255.0) as u8;
    let b = (c.b * 255.0).round().clamp(0.0, 255.0) as u8;
    let a = (c.a * 255.0).round().clamp(0.0, 255.0) as u8;
    (b, g, r, a) // BGRA order
}

/// Blend a single pixel with alpha compositing (src-over).
#[inline]
fn blend_pixel(data: &mut [u8], stride: u32, x: u32, y: u32, sb: u8, sg: u8, sr: u8, sa: u8) {
    let off = (y * stride + x * 4) as usize;
    if off + 3 >= data.len() {
        return;
    }

    if sa == 255 {
        // Fully opaque — no blending needed
        data[off] = sb;
        data[off + 1] = sg;
        data[off + 2] = sr;
        data[off + 3] = 255;
    } else if sa > 0 {
        // Alpha blend: result = src * alpha + dst * (1 - alpha)
        let alpha = sa as u16;
        let inv_alpha = 255 - alpha;
        data[off] = ((sb as u16 * alpha + data[off] as u16 * inv_alpha) / 255) as u8;
        data[off + 1] = ((sg as u16 * alpha + data[off + 1] as u16 * inv_alpha) / 255) as u8;
        data[off + 2] = ((sr as u16 * alpha + data[off + 2] as u16 * inv_alpha) / 255) as u8;
        data[off + 3] = 255; // destination is always opaque
    }
}
