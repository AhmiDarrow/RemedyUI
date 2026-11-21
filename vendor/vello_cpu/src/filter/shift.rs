// Copyright 2025 the Vello Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Shared helpers for pixel-space filter operations.

use vello_common::color::palette::css::TRANSPARENT;
#[cfg(not(feature = "std"))]
use vello_common::kurbo::common::FloatFuncs as _;
use vello_common::peniko::color::PremulRgba8;
use vello_common::pixmap::Pixmap;

/// Shift all pixels in a pixmap by the given offset.
///
/// This implements the offset operation in-place by copying pixels to their new positions.
/// The iteration order is carefully chosen based on shift direction to avoid overwriting
/// source pixels before they're read. Areas that become exposed (due to the shift) are
/// filled with transparent black. Pixels that would move outside the bounds are discarded.
pub(crate) fn offset_pixels(pixmap: &mut Pixmap, dx: f32, dy: f32) {
    let dx_pixels = dx.round() as i32;
    let dy_pixels = dy.round() as i32;

    // Early return if no offset
    if dx_pixels == 0 && dy_pixels == 0 {
        return;
    }

    let width = pixmap.width();
    let height = pixmap.height();
    let transparent = TRANSPARENT.premultiply().to_rgba8();

    // Process pixels in the correct order to avoid overwriting source data.
    // Key insight: iterate away from the direction of movement.
    // This allows us to move pixels in-place without a temporary buffer.
    //
    // Use match to eliminate Box<dyn Iterator> allocation overhead and enable
    // better compiler optimization through static dispatch.
    match (dx_pixels >= 0, dy_pixels >= 0) {
        (true, true) => {
            // Shift right+down: iterate bottom-to-top, right-to-left
            for y in (0..height).rev() {
                for x in (0..width).rev() {
                    process_offset_pixel(pixmap, x, y, dx_pixels, dy_pixels, width, height, transparent);
                }
            }
        }
        (true, false) => {
            // Shift right+up: iterate top-to-bottom, right-to-left
            for y in 0..height {
                for x in (0..width).rev() {
                    process_offset_pixel(pixmap, x, y, dx_pixels, dy_pixels, width, height, transparent);
                }
            }
        }
        (false, true) => {
            // Shift left+down: iterate bottom-to-top, left-to-right
            for y in (0..height).rev() {
                for x in 0..width {
                    process_offset_pixel(pixmap, x, y, dx_pixels, dy_pixels, width, height, transparent);
                }
            }
        }
        (false, false) => {
            // Shift left+up: iterate top-to-bottom, left-to-right
            for y in 0..height {
                for x in 0..width {
                    process_offset_pixel(pixmap, x, y, dx_pixels, dy_pixels, width, height, transparent);
                }
            }
        }
    }
}

/// Process a single pixel during offset operation.
///
/// This moves the pixel to its new position (if in bounds) and clears the source
/// position if it's in the exposed region.
#[inline(always)]
fn process_offset_pixel(
    pixmap: &mut Pixmap,
    x: u16,
    y: u16,
    dx_pixels: i32,
    dy_pixels: i32,
    width: u16,
    height: u16,
    transparent: PremulRgba8,
) {
    // RemedyUI patch: compare in i64. With a shift as large as the pixmap or larger,
    // `width + dx_pixels` is negative, and casting it to `u16` wrapped it to a huge bound,
    // so nothing was cleared and the "shifted" pixmap kept its unshifted pixels. (And
    // `x + dx_pixels` could overflow `i32` for a shift near `i32::MAX`.)
    let (xi, yi) = (i64::from(x), i64::from(y));
    let (dx, dy) = (i64::from(dx_pixels), i64::from(dy_pixels));
    let (w, h) = (i64::from(width), i64::from(height));
    let new_x = xi + dx;
    let new_y = yi + dy;

    if new_x >= 0 && new_x < w && new_y >= 0 && new_y < h {
        let pixel = pixmap.sample(x, y);
        pixmap.set_pixel(new_x as u16, new_y as u16, pixel);
    }

    // Clear the source pixel if it's in the exposed region
    let should_clear =
        (dx > 0 && xi < dx) || (dx < 0 && xi >= w + dx) || (dy > 0 && yi < dy) || (dy < 0 && yi >= h + dy);

    if should_clear {
        pixmap.set_pixel(x, y, transparent);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Test `offset_pixels` with positive offset (right and down).
    #[test]
    fn test_offset_pixels_positive() {
        let mut pixmap = Pixmap::new(4, 4);
        // Set center pixel to white
        pixmap.set_pixel(1, 1, PremulRgba8 { r: 255, g: 255, b: 255, a: 255 });

        offset_pixels(&mut pixmap, 1.0, 1.0);

        // White pixel should have moved from (1,1) to (2,2)
        let moved = pixmap.sample(2, 2);
        assert_eq!(moved.a, 255);

        // Original position should be cleared (in exposed region)
        let cleared = pixmap.sample(1, 1);
        assert_eq!(cleared.a, 0);
    }

    /// Test `offset_pixels` with negative offset (left and up).
    #[test]
    fn test_offset_pixels_negative() {
        let mut pixmap = Pixmap::new(4, 4);
        // Set pixel at (2,2) to white
        pixmap.set_pixel(2, 2, PremulRgba8 { r: 255, g: 255, b: 255, a: 255 });

        offset_pixels(&mut pixmap, -1.0, -1.0);

        // White pixel should have moved from (2,2) to (1,1)
        let moved = pixmap.sample(1, 1);
        assert_eq!(moved.a, 255);

        // Original position should be cleared (in exposed region)
        let cleared = pixmap.sample(2, 2);
        assert_eq!(cleared.a, 0);
    }

    /// Test `offset_pixels` with fractional offset (should round).
    #[test]
    fn test_offset_pixels_fractional() {
        let mut pixmap = Pixmap::new(4, 4);
        pixmap.set_pixel(1, 1, PremulRgba8 { r: 255, g: 255, b: 255, a: 255 });

        // 0.6 rounds to 1, -0.4 rounds to 0
        offset_pixels(&mut pixmap, 0.6, -0.4);

        // Should move by (1, 0): from (1,1) to (2,1)
        let moved = pixmap.sample(2, 1);
        assert_eq!(moved.a, 255);
    }

    /// Test `offset_pixels` with out-of-bounds offset (should clip).
    #[test]
    fn test_offset_pixels_out_of_bounds() {
        let mut pixmap = Pixmap::new(4, 4);
        pixmap.set_pixel(1, 1, PremulRgba8 { r: 255, g: 255, b: 255, a: 255 });

        // Large offset that moves pixel outside bounds
        offset_pixels(&mut pixmap, 10.0, 10.0);

        // Pixel moves outside, so (1,1) should be cleared
        let cleared = pixmap.sample(1, 1);
        assert_eq!(cleared.a, 0);

        // All pixels should be cleared (entire image is exposed region)
        for y in 0..4 {
            for x in 0..4 {
                assert_eq!(pixmap.sample(x, y).a, 0);
            }
        }
    }
}
