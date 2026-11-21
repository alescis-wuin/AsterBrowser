//! Incremental BGRA-to-RGB conversion for Slint, with a persistent image cache.
//! The final `SharedPixelBuffer` may be copy-on-write because Slint still owns
//! the previously presented image; partial GPU uploads are NOT supported here.
use browser_contracts::{FramePayload, PageFrame};
use slint::{Image, Rgb8Pixel, SharedPixelBuffer};

pub(crate) struct RgbSurface {
    pixels: SharedPixelBuffer<Rgb8Pixel>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum UpdateKind {
    Full,
    Partial,
}

pub(crate) struct AppliedUpdate {
    pub kind: UpdateKind,
    pub converted_pixels: u64,
}

impl RgbSurface {
    pub fn from_frame(frame: PageFrame) -> Option<(Self, AppliedUpdate)> {
        let FramePayload::Full(bgra) = frame.payload else {
            return None;
        };
        let pixels = frame.width.checked_mul(frame.height)?;
        let expected = usize::try_from(pixels.checked_mul(4)?).ok()?;
        if pixels == 0 || bgra.len() != expected {
            return None;
        }
        let mut output = SharedPixelBuffer::<Rgb8Pixel>::new(frame.width, frame.height);
        for (src, dst) in bgra
            .chunks_exact(4)
            .zip(output.make_mut_bytes().chunks_exact_mut(3))
        {
            dst.copy_from_slice(&[src[2], src[1], src[0]]);
        }
        Some((
            Self { pixels: output },
            AppliedUpdate {
                kind: UpdateKind::Full,
                converted_pixels: u64::from(pixels),
            },
        ))
    }

    pub fn apply(&mut self, frame: PageFrame) -> Option<AppliedUpdate> {
        if frame.width != self.pixels.width() || frame.height != self.pixels.height() {
            return None;
        }
        match frame.payload {
            FramePayload::Full(bgra) => {
                let expected = frame.width.checked_mul(frame.height)?.checked_mul(4)? as usize;
                if bgra.len() != expected {
                    return None;
                }
                let dst = self.pixels.make_mut_bytes();
                for (src, dst) in bgra.chunks_exact(4).zip(dst.chunks_exact_mut(3)) {
                    dst.copy_from_slice(&[src[2], src[1], src[0]]);
                }
                Some(AppliedUpdate {
                    kind: UpdateKind::Full,
                    converted_pixels: u64::from(frame.width) * u64::from(frame.height),
                })
            }
            FramePayload::Patches(patches) => {
                if patches.is_empty() {
                    return None;
                }
                // Validate all coordinates and payload lengths before mutating anything.
                for patch in &patches {
                    if patch.rect.clipped(frame.width, frame.height) != Some(patch.rect) {
                        return None;
                    }
                    let expected = patch.rect.area().checked_mul(4)? as usize;
                    if patch.bgra.len() != expected {
                        return None;
                    }
                }
                let dst = self.pixels.make_mut_bytes();
                let mut pixels = 0;
                for patch in patches {
                    let rect = patch.rect;
                    pixels += rect.area();
                    for row in 0..rect.height as usize {
                        let source_start = row * rect.width as usize * 4;
                        let dest_start =
                            ((row + rect.y as usize) * frame.width as usize + rect.x as usize) * 3;
                        let from =
                            &patch.bgra[source_start..source_start + rect.width as usize * 4];
                        let to = &mut dst[dest_start..dest_start + rect.width as usize * 3];
                        for (bgra, rgb) in from.chunks_exact(4).zip(to.chunks_exact_mut(3)) {
                            rgb.copy_from_slice(&[bgra[2], bgra[1], bgra[0]]);
                        }
                    }
                }
                Some(AppliedUpdate {
                    kind: UpdateKind::Partial,
                    converted_pixels: pixels,
                })
            }
        }
    }

    pub fn matches_size(&self, frame: &PageFrame) -> bool {
        self.pixels.width() == frame.width && self.pixels.height() == frame.height
    }
    pub fn image(&self) -> Image {
        Image::from_rgb8(self.pixels.clone())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use browser_contracts::{DamageRect, FramePatch, TabId};
    use std::sync::Arc;
    fn full(width: u32, height: u32, bgra: Vec<u8>) -> PageFrame {
        PageFrame {
            tab: TabId(1),
            width,
            height,
            payload: FramePayload::Full(Arc::new(bgra)),
        }
    }
    fn patch(width: u32, height: u32, rect: DamageRect, bgra: Vec<u8>) -> PageFrame {
        PageFrame {
            tab: TabId(1),
            width,
            height,
            payload: FramePayload::Patches(vec![FramePatch { rect, bgra }]),
        }
    }
    #[test]
    fn patch_updates_only_the_changed_rgb_pixel() {
        let (mut surface, _) =
            RgbSurface::from_frame(full(2, 2, [1, 2, 3, 255].repeat(4))).unwrap();
        let original = surface.image();
        let result = surface
            .apply(patch(
                2,
                2,
                DamageRect {
                    x: 1,
                    y: 0,
                    width: 1,
                    height: 1,
                },
                vec![5, 6, 7, 255],
            ))
            .unwrap();
        assert_eq!(result.kind, UpdateKind::Partial);
        assert_eq!(result.converted_pixels, 1);
        assert_eq!(
            surface.pixels.as_bytes(),
            &[3, 2, 1, 7, 6, 5, 3, 2, 1, 3, 2, 1]
        );
        // Copy-on-write protects Slint's previous frame while the new image is constructed.
        let preserved = original.to_rgb8().unwrap();
        assert_eq!(&preserved.as_bytes()[3..6], &[3, 2, 1]);
    }
    #[test]
    fn invalid_patch_does_not_partially_mutate_surface() {
        let (mut surface, _) =
            RgbSurface::from_frame(full(2, 1, [1, 2, 3, 255].repeat(2))).unwrap();
        let before = surface.pixels.as_bytes().to_vec();
        assert!(
            surface
                .apply(patch(
                    2,
                    1,
                    DamageRect {
                        x: 1,
                        y: 0,
                        width: 2,
                        height: 1
                    },
                    vec![0; 8]
                ))
                .is_none()
        );
        assert_eq!(surface.pixels.as_bytes(), before.as_slice());
    }
    #[test]
    fn dimension_change_requires_a_new_full_frame() {
        let (mut surface, _) =
            RgbSurface::from_frame(full(2, 2, [1, 2, 3, 255].repeat(4))).unwrap();
        assert!(
            surface
                .apply(patch(
                    3,
                    2,
                    DamageRect {
                        x: 0,
                        y: 0,
                        width: 1,
                        height: 1
                    },
                    vec![0; 4]
                ))
                .is_none()
        );
    }
}
