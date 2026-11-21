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

/// A ring of one or two independently presented RGB buffers. With two buffers,
/// the inactive slot replays damage missed since it was last presented before
/// becoming the front buffer. Immutable Slint Image snapshots stay valid; a
/// copy-on-write can still occur if Slint retains an older image.
pub(crate) struct BufferedRgbSurface {
    slots: Vec<RgbSurface>,
    front: usize,
    pending_for_other: Vec<browser_contracts::FramePatch>,
}

impl BufferedRgbSurface {
    pub fn from_frame(
        frame: PageFrame,
        buffers: browser_contracts::BufferCount,
    ) -> Option<(Self, AppliedUpdate)> {
        let (surface, update) = RgbSurface::from_frame(frame)?;
        let mut slots = vec![surface];
        if buffers.get() == 2 {
            slots.push(RgbSurface {
                pixels: slots[0].pixels.clone(),
            });
        }
        Some((
            Self {
                slots,
                front: 0,
                pending_for_other: Vec::new(),
            },
            update,
        ))
    }

    pub fn matches_size(&self, frame: &PageFrame) -> bool {
        self.slots[self.front].matches_size(frame)
    }

    pub fn apply(&mut self, frame: PageFrame) -> Option<AppliedUpdate> {
        if !self.matches_size(&frame) {
            return None;
        }
        if self.slots.len() == 1 {
            return self.slots[0].apply(frame);
        }
        let target = (self.front + 1) % 2;
        match frame.payload {
            FramePayload::Full(pixels) => {
                let frame = PageFrame {
                    tab: frame.tab,
                    width: frame.width,
                    height: frame.height,
                    payload: FramePayload::Full(pixels),
                };
                let (surface, update) = Self::from_frame(frame, "2".parse().ok()?)?;
                *self = surface;
                Some(update)
            }
            FramePayload::Patches(patches) => {
                if patches.is_empty() {
                    return None;
                }
                // Do not consume backlog until all patches have been validated.
                let mut update_patches = self.pending_for_other.clone();
                update_patches.extend_from_slice(&patches);
                let update = self.slots[target].apply(PageFrame {
                    tab: frame.tab,
                    width: frame.width,
                    height: frame.height,
                    payload: FramePayload::Patches(update_patches),
                })?;
                self.pending_for_other = patches;
                self.front = target;
                Some(update)
            }
        }
    }

    pub fn image(&self) -> Image {
        self.slots[self.front].image()
    }
}

#[cfg(test)]
mod buffering_tests {
    use super::*;
    use browser_contracts::{DamageRect, FramePatch, TabId};
    use std::sync::Arc;
    fn full() -> PageFrame {
        PageFrame {
            tab: TabId(1),
            width: 3,
            height: 1,
            payload: FramePayload::Full(Arc::new([0, 0, 0, 255].repeat(3))),
        }
    }
    fn patch(x: u32, red: u8) -> PageFrame {
        PageFrame {
            tab: TabId(1),
            width: 3,
            height: 1,
            payload: FramePayload::Patches(vec![FramePatch {
                rect: DamageRect {
                    x,
                    y: 0,
                    width: 1,
                    height: 1,
                },
                bgra: vec![0, 0, red, 255],
            }]),
        }
    }
    #[test]
    fn single_and_double_keep_all_intervening_pixels() {
        for buffers in ["1", "2"] {
            let (mut ring, _) =
                BufferedRgbSurface::from_frame(full(), buffers.parse().unwrap()).unwrap();
            let first = ring.image();
            ring.apply(patch(0, 20)).unwrap();
            ring.apply(patch(1, 30)).unwrap();
            ring.apply(patch(2, 40)).unwrap();
            let image = ring.image().to_rgb8().unwrap();
            assert_eq!(&image.as_bytes()[..], &[20, 0, 0, 30, 0, 0, 40, 0, 0]);
            // The previously presented image must remain unchanged.
            assert_eq!(&first.to_rgb8().unwrap().as_bytes()[..], &[0; 9]);
        }
    }
}
