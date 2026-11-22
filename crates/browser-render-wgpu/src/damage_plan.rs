//! Deterministic GPU damage accounting, independent of the graphics device.
//! Adaptive mode computes the exact union of axis-aligned damage rectangles.
use browser_contracts::{DamageRect, GpuDamageMode, GpuOptions};
use std::collections::BTreeMap;

pub const MAX_INPUT_RECTS: usize = 256;
const MAX_DRAW_RECTS: usize = 96;

#[derive(Debug)]
pub struct DamagePlan {
    pub regions: Vec<DamageRect>,
    pub full: bool,
    pub raw_pixels: u64,
    pub unique_pixels: u64,
    pub incoming_rects: usize,
    pub reason: &'static str,
}

/// Exact, non-overlapping union representation. The optional vertical merge
/// avoids per-scanline draw calls when a rectangle spans multiple strips.
fn disjoint_union(rects: &[DamageRect]) -> Vec<DamageRect> {
    let mut ys: Vec<u32> = Vec::with_capacity(rects.len() * 2);
    for r in rects {
        ys.push(r.y);
        ys.push(r.y + r.height);
    }
    ys.sort_unstable();
    ys.dedup();
    let mut regions: Vec<DamageRect> = Vec::new();
    let mut previous: BTreeMap<(u32, u32), usize> = BTreeMap::new();
    for band in ys.windows(2) {
        let (top, bottom) = (band[0], band[1]);
        if top == bottom {
            continue;
        }
        let mut intervals: Vec<(u32, u32)> = rects
            .iter()
            .filter(|r| r.y <= top && r.y + r.height >= bottom)
            .map(|r| (r.x, r.x + r.width))
            .collect();
        intervals.sort_unstable();
        let mut merged: Vec<(u32, u32)> = Vec::new();
        for (start, end) in intervals {
            if let Some(last) = merged.last_mut() {
                if start <= last.1 {
                    last.1 = last.1.max(end);
                    continue;
                }
            }
            merged.push((start, end));
        }
        let mut next = BTreeMap::new();
        for (left, right) in merged {
            if let Some(&index) = previous.get(&(left, right)) {
                regions[index].height += bottom - top;
                next.insert((left, right), index);
            } else {
                let index = regions.len();
                regions.push(DamageRect {
                    x: left,
                    y: top,
                    width: right - left,
                    height: bottom - top,
                });
                next.insert((left, right), index);
            }
        }
        previous = next;
        // Avoid excessive allocations for pathological damage sets.
        if regions.len() > MAX_INPUT_RECTS {
            break;
        }
    }
    regions
}

pub fn plan_damage(
    dirty: &[DamageRect],
    width: u32,
    height: u32,
    options: GpuOptions,
) -> DamagePlan {
    let incoming_rects = dirty.len();
    let full_pixels = u64::from(width) * u64::from(height);
    let full_region = DamageRect {
        x: 0,
        y: 0,
        width,
        height,
    };
    let full = |reason| DamagePlan {
        regions: vec![full_region],
        full: true,
        raw_pixels: full_pixels,
        unique_pixels: full_pixels,
        incoming_rects,
        reason,
    };
    if full_pixels == 0 {
        return full("zero_extent");
    }
    if dirty.is_empty() {
        return full("unknown_damage");
    }
    if dirty.len() > MAX_INPUT_RECTS {
        return full("too_many_input_rects");
    }
    let clipped: Vec<DamageRect> = dirty
        .iter()
        .filter_map(|r| r.clipped(width, height))
        .collect();
    if clipped.is_empty() {
        return full("no_valid_damage");
    }
    let raw_pixels = clipped.iter().map(|r| r.area()).sum::<u64>();
    // Compute the same exact metric in all modes for comparable A/B reports;
    // only the decision algorithm changes with --gpu-damage.
    let union = disjoint_union(&clipped);
    let unique_pixels = union.iter().map(|r| r.area()).sum::<u64>();
    if options.damage == GpuDamageMode::Full {
        return DamagePlan {
            regions: vec![full_region],
            full: true,
            raw_pixels,
            unique_pixels,
            incoming_rects,
            reason: "forced_full",
        };
    }
    if options.damage == GpuDamageMode::Legacy {
        let old_full =
            clipped.len() > 64 || raw_pixels.saturating_mul(100) >= full_pixels.saturating_mul(65);
        return DamagePlan {
            regions: if old_full { vec![full_region] } else { clipped },
            full: old_full,
            raw_pixels,
            unique_pixels,
            incoming_rects,
            reason: if old_full {
                "legacy_full"
            } else {
                "legacy_partial"
            },
        };
    }
    if union.len() > MAX_DRAW_RECTS {
        return DamagePlan {
            regions: vec![full_region],
            full: true,
            raw_pixels,
            unique_pixels,
            incoming_rects,
            reason: "too_many_draw_rects",
        };
    }
    let forced_partial = options.damage == GpuDamageMode::Partial;
    let threshold = unique_pixels.saturating_mul(100)
        >= full_pixels.saturating_mul(u64::from(options.threshold_pct));
    let estimated_pixels =
        unique_pixels.saturating_add((union.len() as u64) * u64::from(options.rect_penalty_pixels));
    let full_update = !forced_partial && (threshold || estimated_pixels >= full_pixels);
    DamagePlan {
        regions: if full_update {
            vec![full_region]
        } else {
            union
        },
        full: full_update,
        raw_pixels,
        unique_pixels,
        incoming_rects,
        reason: if full_update && threshold {
            "area_threshold"
        } else if full_update {
            "estimated_draw_cost"
        } else if forced_partial {
            "forced_partial"
        } else {
            "adaptive_partial"
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn rect(x: u32, y: u32, w: u32, h: u32) -> DamageRect {
        DamageRect {
            x,
            y,
            width: w,
            height: h,
        }
    }
    fn adaptive() -> GpuOptions {
        GpuOptions {
            damage: GpuDamageMode::Adaptive,
            rect_penalty_pixels: 0,
            ..Default::default()
        }
    }
    #[test]
    fn overlap_counts_each_pixel_only_once() {
        let p = plan_damage(&[rect(0, 0, 6, 6), rect(4, 4, 6, 6)], 10, 10, adaptive());
        assert_eq!(p.raw_pixels, 72);
        assert_eq!(p.unique_pixels, 68);
        assert_eq!(p.regions.iter().map(|r| r.area()).sum::<u64>(), 100); // 68% exceeds 65% threshold.
        assert!(p.full);
    }
    #[test]
    fn small_overlapping_rectangles_do_not_trigger_false_full() {
        let p = plan_damage(
            &[rect(0, 0, 5, 5), rect(0, 0, 5, 5), rect(0, 0, 5, 5)],
            10,
            10,
            adaptive(),
        );
        assert!(!p.full);
        assert_eq!(p.raw_pixels, 75);
        assert_eq!(p.unique_pixels, 25);
        assert_eq!(p.regions, vec![rect(0, 0, 5, 5)]);
    }
    #[test]
    fn legacy_and_adaptive_report_same_union_even_when_decisions_differ() {
        let damage = [rect(0, 0, 5, 5), rect(0, 0, 5, 5), rect(0, 0, 5, 5)];
        let legacy = plan_damage(&damage, 10, 10, GpuOptions::default());
        let adaptive = plan_damage(&damage, 10, 10, adaptive());
        assert!(legacy.full); // Old raw-area test: 75%.
        assert!(!adaptive.full); // Exact union: 25%.
        assert_eq!(legacy.unique_pixels, adaptive.unique_pixels);
        assert_eq!(legacy.unique_pixels, 25);
    }
    #[test]
    fn respects_holes_and_clips_to_bounds() {
        let p = plan_damage(&[rect(0, 0, 3, 3), rect(7, 7, 20, 20)], 10, 10, adaptive());
        assert!(!p.full);
        assert_eq!(p.unique_pixels, 18);
        assert_eq!(p.regions.iter().map(|r| r.area()).sum::<u64>(), 18);
    }
    #[test]
    fn forced_modes_are_independent_of_threshold() {
        let mut cfg = adaptive();
        cfg.damage = GpuDamageMode::Full;
        assert!(plan_damage(&[rect(0, 0, 1, 1)], 100, 100, cfg).full);
        cfg.damage = GpuDamageMode::Partial;
        assert!(!plan_damage(&[rect(0, 0, 100, 100)], 100, 100, cfg).full);
        assert!(plan_damage(&[], 100, 100, cfg).full); // Unknown damage cannot be safely skipped.
    }
    #[test]
    fn merging_vertical_strips_does_not_add_draws() {
        let p = plan_damage(&[rect(0, 0, 5, 10), rect(3, 2, 2, 3)], 20, 20, adaptive());
        assert_eq!(p.regions, vec![rect(0, 0, 5, 10)]);
        assert_eq!(p.unique_pixels, 50);
    }
    #[test]
    fn excessive_input_has_bounded_work() {
        let cfg = adaptive();
        let input = vec![rect(0, 0, 1, 1); MAX_INPUT_RECTS + 1];
        assert!(plan_damage(&input, 100, 100, cfg).full);
    }
}
