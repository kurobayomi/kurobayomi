//! `in_sample_size` and `target_dim` for downsampled decode.

use crate::options::{RequestSize, Scale};

/// One side of a request size (Coil's `Dimension`). `Undefined` is a view that has no size yet.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Dimension {
    Undefined,
    Pixel(i32),
}

/// Coil's `Size`. `Original` asks for the source size, so nothing is sampled away.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Size {
    Original,
    Dims { width: Dimension, height: Dimension },
}

impl From<RequestSize> for Size {
    fn from(size: RequestSize) -> Self {
        match size {
            RequestSize::Original => Size::Original,
            RequestSize::Pixels { width, height } => Size::Dims {
                width: Dimension::Pixel(width.min(i32::MAX as u32) as i32),
                height: Dimension::Pixel(height.min(i32::MAX as u32) as i32),
            },
        }
    }
}

impl Dimension {
    /// `Dimension.toPx(scale)`: a pixel value as is, the sentinel for `Undefined`.
    fn to_px(self, scale: Scale) -> i32 {
        match self {
            Dimension::Pixel(n) => n,
            Dimension::Undefined => match scale {
                Scale::Fill => i32::MIN,
                Scale::Fit => i32::MAX,
            },
        }
    }
}

/// The destination `(width, height)` in pixels for a source of `src`.
pub fn target_dim(size: Size, scale: Scale, src: (i32, i32)) -> (i32, i32) {
    match size {
        Size::Original => src,
        Size::Dims { width, height } => (width.to_px(scale), height.to_px(scale)),
    }
}

/// Kotlin's `Int.takeHighestOneBit`: only the highest set bit stays, and a negative number keeps the sign bit.
fn highest_one_bit(x: i32) -> i32 {
    if x == 0 {
        0
    } else {
        x & (((i32::MIN as u32) >> x.leading_zeros()) as i32)
    }
}

/// Coil's `DecodeUtils.calculateInSampleSize`: integer division, highest bit, the
/// smaller of the two for `Fill` and the larger for `Fit`, never below 1. A zero destination side would divide
/// by zero in Kotlin, so it counts as undefined here (edge E-IMG-7).
pub fn in_sample_size(
    src_width: i32,
    src_height: i32,
    dst_width: i32,
    dst_height: i32,
    scale: Scale,
) -> i32 {
    if dst_width == 0 || dst_height == 0 {
        return 1;
    }
    let width = highest_one_bit(src_width / dst_width);
    let height = highest_one_bit(src_height / dst_height);
    match scale {
        Scale::Fill => width.min(height),
        Scale::Fit => width.max(height),
    }
    .max(1)
}

/// The sample size for a source of `src` and a request, the one call the decoder needs.
pub fn sample_for(size: RequestSize, scale: Scale, src: (u32, u32)) -> i32 {
    let src = (
        src.0.min(i32::MAX as u32) as i32,
        src.1.min(i32::MAX as u32) as i32,
    );
    let (dst_width, dst_height) = target_dim(size.into(), scale, src);
    in_sample_size(src.0, src.1, dst_width, dst_height, scale)
}

/// Output side of a decode at `sample`: the source side divided and rounded up, at least 1.
pub fn sampled_side(side: u32, sample: i32) -> u32 {
    side.div_ceil(sample.max(1) as u32).max(1)
}

#[cfg(test)]
mod tests {
    use super::*;
    use Scale::{Fill, Fit};

    const UNDEF: Dimension = Dimension::Undefined;

    fn sample(src: (i32, i32), dst: (i32, i32), scale: Scale) -> i32 {
        in_sample_size(src.0, src.1, dst.0, dst.1, scale)
    }

    #[test]
    fn g_sample_table() {
        type Row = ((i32, i32), (i32, i32), Scale, i32);
        let rows: &[Row] = &[
            ((1000, 1500), (500, 500), Fit, 2),
            ((1000, 1500), (500, 500), Fill, 2),
            ((1000, 1500), (300, 400), Fit, 2),
            ((4000, 6000), (500, 750), Fit, 8),
            ((4000, 6000), (500, 750), Fill, 8),
            ((100, 100), (100, 100), Fit, 1),
            ((1000, 1000), (600, 400), Fit, 2),
            ((1000, 1000), (600, 400), Fill, 1),
            ((5000, 100), (1000, 1000), Fit, 4),
            ((5000, 100), (1000, 1000), Fill, 1),
            ((2048, 2048), (1024, 1024), Fit, 2),
            ((4000, 6000), (0, 0), Fit, 1),
        ];
        for (src, dst, scale, want) in rows {
            assert_eq!(
                sample(*src, *dst, *scale),
                *want,
                "{src:?} -> {dst:?} {scale:?}"
            );
        }
    }

    #[test]
    fn g_sample_undefined_and_original() {
        for scale in [Fill, Fit] {
            let (w, h) = target_dim(
                Size::Dims {
                    width: UNDEF,
                    height: UNDEF,
                },
                scale,
                (4000, 6000),
            );
            assert_eq!(sample((4000, 6000), (w, h), scale), 1);
            let (w, h) = target_dim(Size::Original, scale, (4000, 6000));
            assert_eq!((w, h), (4000, 6000));
            assert_eq!(sample((4000, 6000), (w, h), scale), 1);
        }
    }

    #[test]
    fn undefined_maps_to_the_sentinel_of_the_scale() {
        let size = Size::Dims {
            width: UNDEF,
            height: Dimension::Pixel(300),
        };
        assert_eq!(target_dim(size, Fill, (1, 1)), (i32::MIN, 300));
        assert_eq!(target_dim(size, Fit, (1, 1)), (i32::MAX, 300));
    }

    #[test]
    fn highest_one_bit_matches_kotlin() {
        assert_eq!(highest_one_bit(0), 0);
        assert_eq!(highest_one_bit(3), 2);
        assert_eq!(highest_one_bit(5), 4);
        assert_eq!(highest_one_bit(6), 4);
        assert_eq!(highest_one_bit(8), 8);
        assert_eq!(highest_one_bit(-1), i32::MIN);
    }

    #[test]
    fn negative_destination_is_clamped_to_one() {
        assert_eq!(sample((1000, 1000), (-1, -1), Fit), 1);
        assert_eq!(sample((1000, 1000), (-1, -1), Fill), 1);
    }

    #[test]
    fn request_size_converts_and_samples() {
        assert_eq!(sample_for(RequestSize::Original, Fit, (4000, 6000)), 1);
        let px = RequestSize::Pixels {
            width: 500,
            height: 750,
        };
        assert_eq!(sample_for(px, Fit, (4000, 6000)), 8);
        let zero = RequestSize::Pixels {
            width: 0,
            height: 0,
        };
        assert_eq!(sample_for(zero, Fit, (4000, 6000)), 1);
    }

    #[test]
    fn sampled_side_rounds_up() {
        assert_eq!(sampled_side(1000, 1), 1000);
        assert_eq!(sampled_side(1001, 2), 501);
        assert_eq!(sampled_side(3, 8), 1);
    }
}
