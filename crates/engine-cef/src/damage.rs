//! CPU-side incremental CEF image cache. Contains no CEF or UI toolkit types.
//! Chromium's BGRA buffer is always a full image; damage limits comparisons
//! and updates, not the extent of that borrowed source buffer.
use browser_contracts::{DamageRect, FramePatch, FramePayload, PageFrame, TabId};
use std::sync::Arc;

const MAX_RECTS: usize = 64;
const FULL_THRESHOLD_PERCENT: u64 = 70;

#[derive(Default)]
pub(crate) struct DamageSurface {
    width: u32,
    height: u32,
    pixels: Arc<Vec<u8>>,
    pending_full: bool,
    pending: Vec<DamageRect>,
}

#[derive(Default)]
pub(crate) struct PaintResult {
    pub compared_bytes: u64,
    pub copied_bytes: u64,
    pub dirty_pixels: u64,
    pub total_pixels: u64,
    pub unchanged: bool,
    pub coalesced: bool,
}

impl DamageSurface {
    /// Called on CEF's UI thread while `source` is still borrowed from OnPaint.
    pub fn paint(
        &mut self,
        source: &[u8],
        width: u32,
        height: u32,
        damage: &[DamageRect],
    ) -> Option<PaintResult> {
        let count = usize::try_from(width.checked_mul(height)?.checked_mul(4)?).ok()?;
        if width == 0 || height == 0 || source.len() != count {
            return None;
        }
        let mut result = PaintResult {
            total_pixels: u64::from(width) * u64::from(height),
            ..Default::default()
        };
        if self.width != width || self.height != height || self.pixels.len() != count {
            self.width = width;
            self.height = height;
            self.pixels = Arc::new(source.to_vec());
            self.pending_full = true;
            self.pending.clear();
            result.copied_bytes = count as u64;
            result.dirty_pixels = result.total_pixels;
            return Some(result);
        }

        // An empty list cannot be treated as a blank screen: CEF documents
        // the complete image buffer even when the damage list is unavailable.
        let fallback = [DamageRect {
            x: 0,
            y: 0,
            width,
            height,
        }];
        let rects = if damage.is_empty() {
            &fallback[..]
        } else {
            damage
        };
        let was_pending = self.pending_full || !self.pending.is_empty();
        let mut changed_any = false;
        for &rect in rects {
            let Some(rect) = rect.clipped(width, height) else {
                continue;
            };
            result.dirty_pixels = result.dirty_pixels.saturating_add(rect.area());
            let mut first_changed_row = None;
            let mut last_changed_row = 0;
            for y in rect.y..rect.y + rect.height {
                let start = (u64::from(y) * u64::from(width) + u64::from(rect.x)) as usize * 4;
                let end = start + rect.width as usize * 4;
                let incoming = &source[start..end];
                result.compared_bytes += (end - start) as u64;
                if &self.pixels[start..end] != incoming {
                    // COW is only needed if a Full snapshot is still in flight.
                    if Arc::strong_count(&self.pixels) > 1 {
                        result.copied_bytes += self.pixels.len() as u64;
                    }
                    Arc::make_mut(&mut self.pixels)[start..end].copy_from_slice(incoming);
                    result.copied_bytes += (end - start) as u64;
                    changed_any = true;
                    first_changed_row.get_or_insert(y);
                    last_changed_row = y;
                }
            }
            if let Some(top) = first_changed_row {
                if !self.pending_full {
                    self.pending.push(DamageRect {
                        x: rect.x,
                        y: top,
                        width: rect.width,
                        height: last_changed_row - top + 1,
                    });
                    if self.pending.len() > MAX_RECTS {
                        self.pending.clear();
                        self.pending_full = true;
                    }
                }
            }
        }
        result.unchanged = !changed_any;
        result.coalesced = changed_any && was_pending;
        Some(result)
    }

    /// Returns at most one coherent update per tab per frontend poll.
    /// Pending dirty rectangles accumulate across multiple CEF paints.
    pub fn take_frame(&mut self, tab: TabId) -> Option<PageFrame> {
        if !self.pending_full && self.pending.is_empty() {
            return None;
        }
        let full_area = u64::from(self.width) * u64::from(self.height);
        let pending_area = self.pending.iter().map(|r| r.area()).sum::<u64>();
        let full = self.pending_full || pending_area * 100 >= full_area * FULL_THRESHOLD_PERCENT;
        let payload = if full {
            FramePayload::Full(Arc::clone(&self.pixels))
        } else {
            let patches = self
                .pending
                .iter()
                .map(|&rect| {
                    let mut bgra = Vec::with_capacity(rect.area() as usize * 4);
                    for y in rect.y..rect.y + rect.height {
                        let start =
                            (u64::from(y) * u64::from(self.width) + u64::from(rect.x)) as usize * 4;
                        bgra.extend_from_slice(
                            &self.pixels[start..start + rect.width as usize * 4],
                        );
                    }
                    FramePatch { rect, bgra }
                })
                .collect();
            FramePayload::Patches(patches)
        };
        self.pending_full = false;
        self.pending.clear();
        Some(PageFrame {
            tab,
            width: self.width,
            height: self.height,
            payload,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fill(width: usize, height: usize, color: [u8; 4]) -> Vec<u8> {
        color.repeat(width * height)
    }
    fn rect(x: u32, y: u32, width: u32, height: u32) -> DamageRect {
        DamageRect {
            x,
            y,
            width,
            height,
        }
    }
    #[test]
    fn unchanged_pixels_require_no_copy_or_presentation() {
        let mut surface = DamageSurface::default();
        let data = fill(4, 3, [1, 2, 3, 255]);
        surface.paint(&data, 4, 3, &[]).unwrap();
        assert!(matches!(
            surface.take_frame(TabId(1)).unwrap().payload,
            FramePayload::Full(_)
        ));
        let result = surface.paint(&data, 4, 3, &[rect(1, 1, 2, 1)]).unwrap();
        assert!(result.unchanged);
        assert_eq!(result.copied_bytes, 0);
        assert!(surface.take_frame(TabId(1)).is_none());
    }
    #[test]
    fn partial_updates_accumulate_across_paints_without_losing_damage() {
        let mut surface = DamageSurface::default();
        let mut data = fill(6, 4, [0, 0, 0, 255]);
        surface.paint(&data, 6, 4, &[]).unwrap();
        surface.take_frame(TabId(3));
        data[(1 * 6 + 1) * 4] = 42;
        let first = surface.paint(&data, 6, 4, &[rect(1, 1, 1, 1)]).unwrap();
        assert_eq!(first.copied_bytes, 4);
        data[(2 * 6 + 4) * 4 + 1] = 91;
        let second = surface.paint(&data, 6, 4, &[rect(4, 2, 1, 1)]).unwrap();
        assert_eq!(second.copied_bytes, 4);
        assert!(second.coalesced);
        let update = surface.take_frame(TabId(3)).unwrap();
        let FramePayload::Patches(patches) = update.payload else {
            panic!("expected patches")
        };
        assert_eq!(patches.len(), 2);
        assert_eq!(patches[0].bgra.as_slice(), &[42, 0, 0, 255]);
        assert_eq!(patches[1].bgra.as_slice(), &[0, 91, 0, 255]);
        assert!(surface.take_frame(TabId(3)).is_none());
    }
    #[test]
    fn resize_always_forces_full_frame() {
        let mut surface = DamageSurface::default();
        surface
            .paint(&fill(2, 2, [3, 4, 5, 255]), 2, 2, &[])
            .unwrap();
        surface.take_frame(TabId(1));
        surface
            .paint(&fill(3, 1, [3, 4, 5, 255]), 3, 1, &[rect(0, 0, 1, 1)])
            .unwrap();
        let updated = surface.take_frame(TabId(1)).unwrap();
        assert_eq!((updated.width, updated.height), (3, 1));
        assert!(matches!(updated.payload, FramePayload::Full(_)));
    }
    #[test]
    fn out_of_bounds_damage_is_clipped_without_invalid_reads() {
        let mut surface = DamageSurface::default();
        let mut data = fill(3, 2, [0, 0, 0, 255]);
        surface.paint(&data, 3, 2, &[]).unwrap();
        surface.take_frame(TabId(1));
        data[(1 * 3 + 2) * 4] = 99;
        let result = surface.paint(&data, 3, 2, &[rect(2, 1, 100, 100)]).unwrap();
        assert_eq!(result.copied_bytes, 4);
        let FramePayload::Patches(patches) = surface.take_frame(TabId(1)).unwrap().payload else {
            panic!("expected patches")
        };
        assert_eq!(patches[0].rect, rect(2, 1, 1, 1));
    }
    #[test]
    fn overlapping_damage_keeps_final_pixel_values() {
        let mut surface = DamageSurface::default();
        let mut data = fill(4, 3, [10, 10, 10, 255]);
        surface.paint(&data, 4, 3, &[]).unwrap();
        surface.take_frame(TabId(1));
        data[(1 * 4 + 1) * 4] = 66;
        surface
            .paint(&data, 4, 3, &[rect(0, 1, 3, 1), rect(1, 1, 3, 1)])
            .unwrap();
        let frame = surface.take_frame(TabId(1)).unwrap();
        assert!(matches!(frame.payload, FramePayload::Patches(_)));
    }
    #[test]
    fn shared_full_snapshot_is_immutable_when_subsequent_paint_arrives() {
        let mut surface = DamageSurface::default();
        surface
            .paint(&fill(2, 2, [1, 1, 1, 255]), 2, 2, &[])
            .unwrap();
        let snapshot = surface.take_frame(TabId(7)).unwrap();
        let FramePayload::Full(before) = snapshot.payload else {
            panic!("expected full snapshot")
        };
        let mut new_data = fill(2, 2, [1, 1, 1, 255]);
        new_data[0] = 9;
        let report = surface.paint(&new_data, 2, 2, &[rect(0, 0, 1, 1)]).unwrap();
        assert_eq!(before[0], 1);
        assert_eq!(report.copied_bytes, 2 * 2 * 4 + 4);
        assert!(matches!(
            surface.take_frame(TabId(7)).unwrap().payload,
            FramePayload::Patches(_)
        ));
    }
    #[test]
    fn pending_resize_supersedes_earlier_partial_frames() {
        let mut surface = DamageSurface::default();
        let mut data = fill(3, 3, [10, 10, 10, 255]);
        surface.paint(&data, 3, 3, &[]).unwrap();
        surface.take_frame(TabId(1));
        data[0] = 15;
        surface.paint(&data, 3, 3, &[rect(0, 0, 1, 1)]).unwrap();
        surface
            .paint(&fill(2, 2, [4, 5, 6, 255]), 2, 2, &[])
            .unwrap();
        assert!(matches!(
            surface.take_frame(TabId(1)).unwrap().payload,
            FramePayload::Full(_)
        ));
    }
}
