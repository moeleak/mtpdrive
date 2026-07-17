//! Retina template artwork for the macOS menu bar.

pub(crate) const SIZE: u32 = 36;
const SAMPLES_PER_AXIS: u32 = 4;

#[allow(clippy::cast_precision_loss)]
pub(crate) fn rgba() -> Vec<u8> {
    let mut rgba = Vec::with_capacity((SIZE * SIZE * 4) as usize);
    for y in 0..SIZE {
        for x in 0..SIZE {
            let mut covered = 0_u32;
            for sample_y in 0..SAMPLES_PER_AXIS {
                for sample_x in 0..SAMPLES_PER_AXIS {
                    let sample_x = x as f32 + (sample_x as f32 + 0.5) / SAMPLES_PER_AXIS as f32;
                    let sample_y = y as f32 + (sample_y as f32 + 0.5) / SAMPLES_PER_AXIS as f32;
                    covered += u32::from(glyph_contains(sample_x, sample_y));
                }
            }
            let sample_count = SAMPLES_PER_AXIS * SAMPLES_PER_AXIS;
            let alpha = u8::try_from(covered * 255 / sample_count).unwrap_or(u8::MAX);
            rgba.extend_from_slice(&[0, 0, 0, alpha]);
        }
    }
    rgba
}

fn glyph_contains(x: f32, y: f32) -> bool {
    let head = (ellipse_contains(x, y, 18.0, 15.0, 9.8, 8.5) && y <= 15.0)
        || capsule_contains(x, y, 11.8, 8.2, 8.7, 3.1, 1.15)
        || capsule_contains(x, y, 24.2, 8.2, 27.3, 3.1, 1.15);
    let eyes = circle_contains(x, y, 14.0, 12.0, 1.25) || circle_contains(x, y, 22.0, 12.0, 1.25);

    let disc = circle_contains(x, y, 18.0, 25.0, 8.4) && !circle_contains(x, y, 18.0, 25.0, 2.3);
    (head && !eyes) || disc
}

fn ellipse_contains(
    x: f32,
    y: f32,
    center_x: f32,
    center_y: f32,
    radius_x: f32,
    radius_y: f32,
) -> bool {
    let x = (x - center_x) / radius_x;
    let y = (y - center_y) / radius_y;
    x * x + y * y <= 1.0
}

fn circle_contains(x: f32, y: f32, center_x: f32, center_y: f32, radius: f32) -> bool {
    let x = x - center_x;
    let y = y - center_y;
    x * x + y * y <= radius * radius
}

fn capsule_contains(
    x: f32,
    y: f32,
    start_x: f32,
    start_y: f32,
    end_x: f32,
    end_y: f32,
    radius: f32,
) -> bool {
    let segment_x = end_x - start_x;
    let segment_y = end_y - start_y;
    let length_squared = segment_x * segment_x + segment_y * segment_y;
    let projection = ((x - start_x) * segment_x + (y - start_y) * segment_y) / length_squared;
    let projection = projection.clamp(0.0, 1.0);
    let nearest_x = start_x + projection * segment_x;
    let nearest_y = start_y + projection * segment_y;
    circle_contains(x, y, nearest_x, nearest_y, radius)
}

#[cfg(test)]
#[path = "../tests/unit/tray_template.rs"]
mod tests;
