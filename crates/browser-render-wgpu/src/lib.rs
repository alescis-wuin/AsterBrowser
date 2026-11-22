//! Engine- and UI-toolkit-neutral GPU frame transport and incremental WGPU composition.
//! CEF imports a foreign surface into an *owned* GPU texture first; that step is
//! a full GPU-to-GPU copy, never a CPU readback. A GPU shader converts BGRA or
//! RGBA source frames into Slint-compatible RGBA8 using damage-rect scissors.
use browser_contracts::{BufferCount, DamageRect, GpuImportFilter, GpuOptions, GpuSyncMode, TabId};
mod damage_plan;
use damage_plan::plan_damage;
use std::{
    cell::RefCell,
    collections::HashMap,
    rc::Rc,
    sync::Arc,
    time::{Duration, Instant},
};
pub use wgpu;

#[derive(Clone)]
pub struct GpuHub(Rc<RefCell<State>>);

#[derive(Clone)]
pub struct GpuDevice {
    pub device: wgpu::Device,
    pub queue: wgpu::Queue,
}

#[derive(Clone)]
pub struct PresentedTexture {
    pub texture: wgpu::Texture,
    pub revision: u64,
    /// CPU time of GPU command submission (NOT display scan-out).
    pub submitted_at: Instant,
}

#[derive(Default)]
struct GpuStats {
    started: Option<Instant>,
    frames: u64,
    full_copies: u64,
    partial_copies: u64,
    bytes_owned_imports: u64,
    bytes_presentation_copies: u64,
    copy_time: Duration,
    max_copy_time: Duration,
    import_wall: Duration,
    import_wall_max: Duration,
    sync_wait: Duration,
    sync_wait_max: Duration,
    plan_cpu: Duration,
    raw_damage_pixels: u64,
    unique_damage_pixels: u64,
    planned_frame_pixels: u64,
    scissor_draws: u64,
    imported_frames: u64,
    skipped_imports: u64,
    estimated_import_savings: u64,
    lazy_full_initializations: u64,
    input_rects: u64,
    plan_reasons: HashMap<&'static str, u64>,
}

struct State {
    device: Option<GpuDevice>,
    blit: Option<Rc<Blit>>,
    count: BufferCount,
    options: GpuOptions,
    lifecycle: bool,
    surfaces: HashMap<TabId, GpuSurface>,
    failure: Option<String>,
    /// Set when Slint tears down the renderer (including normal window close).
    /// This is *not* evidence of a GPU device failure by itself.
    teardown_at: Option<Instant>,
    stats: GpuStats,
    perf: bool,
    on_present: Option<Arc<dyn Fn() + Send + Sync>>,
}

struct Blit {
    pipeline: wgpu::RenderPipeline,
    bind_group_layout: wgpu::BindGroupLayout,
}

impl Blit {
    fn new(device: &wgpu::Device) -> Self {
        let bind_group_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("Aster CEF GPU source texture layout"),
            entries: &[wgpu::BindGroupLayoutEntry {
                binding: 0,
                visibility: wgpu::ShaderStages::FRAGMENT,
                ty: wgpu::BindingType::Texture {
                    sample_type: wgpu::TextureSampleType::Float { filterable: true },
                    view_dimension: wgpu::TextureViewDimension::D2,
                    multisampled: false,
                },
                count: None,
            }],
        });
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("Aster CEF texture normalization"),
            source: wgpu::ShaderSource::Wgsl(include_str!("swizzle.wgsl").into()),
        });
        let layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("Aster CEF texture normalization pipeline"),
            bind_group_layouts: &[Some(&bind_group_layout)],
            immediate_size: 0,
        });
        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("Aster CEF BGRA/RGBA to RGBA8"),
            layout: Some(&layout),
            vertex: wgpu::VertexState {
                module: &shader,
                entry_point: Some("vertex_main"),
                buffers: &[],
                compilation_options: Default::default(),
            },
            fragment: Some(wgpu::FragmentState {
                module: &shader,
                entry_point: Some("fragment_main"),
                targets: &[Some(wgpu::ColorTargetState {
                    format: wgpu::TextureFormat::Rgba8Unorm,
                    blend: None,
                    write_mask: wgpu::ColorWrites::ALL,
                })],
                compilation_options: Default::default(),
            }),
            primitive: Default::default(),
            depth_stencil: None,
            multisample: Default::default(),
            multiview_mask: None,
            cache: None,
        });
        Self {
            pipeline,
            bind_group_layout,
        }
    }

    /// Render only the dirty rectangles. The source is already an owned GPU
    /// texture so it remains valid through queue submission. No CPU readback.
    fn compose(
        &self,
        device: &wgpu::Device,
        encoder: &mut wgpu::CommandEncoder,
        source: &wgpu::Texture,
        target: &wgpu::Texture,
        dirty: &[DamageRect],
        clear: bool,
    ) {
        let source_view = source.create_view(&wgpu::TextureViewDescriptor::default());
        let target_view = target.create_view(&wgpu::TextureViewDescriptor::default());
        let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("Aster CEF GPU frame source"),
            layout: &self.bind_group_layout,
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: wgpu::BindingResource::TextureView(&source_view),
            }],
        });
        let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("Aster CEF partial RGBA composition"),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view: &target_view,
                resolve_target: None,
                ops: wgpu::Operations {
                    load: if clear {
                        wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT)
                    } else {
                        wgpu::LoadOp::Load
                    },
                    store: wgpu::StoreOp::Store,
                },
                depth_slice: None,
            })],
            ..Default::default()
        });
        pass.set_pipeline(&self.pipeline);
        pass.set_bind_group(0, &bind_group, &[]);
        for rect in dirty {
            if rect.width == 0 || rect.height == 0 {
                continue;
            }
            pass.set_scissor_rect(rect.x, rect.y, rect.width, rect.height);
            pass.draw(0..3, 0..1);
        }
    }
}

struct Slot {
    texture: wgpu::Texture,
    /// Damage missed by this slot while another slot was displayed.
    pending: Vec<DamageRect>,
    initialized: bool,
}

struct GpuSurface {
    slots: Vec<Slot>,
    front: usize,
    revision: u64,
    width: u32,
    height: u32,
    format: wgpu::TextureFormat,
    submitted_at: Instant,
}

impl GpuHub {
    pub fn new(count: BufferCount) -> Self {
        Self::with_options(count, GpuOptions::default())
    }

    pub fn with_options(count: BufferCount, options: GpuOptions) -> Self {
        Self(Rc::new(RefCell::new(State {
            device: None,
            blit: None,
            count,
            options,
            surfaces: HashMap::new(),
            failure: None,
            teardown_at: None,
            stats: GpuStats::default(),
            perf: std::env::var("ASTER_PERF").ok().as_deref() == Some("1"),
            lifecycle: std::env::var("ASTER_LIFECYCLE").ok().as_deref() == Some("1"),
            on_present: None,
        })))
    }

    pub fn import_filter(&self) -> GpuImportFilter {
        self.0.borrow().options.import_filter
    }

    pub fn set_frame_notifier(&self, notifier: Option<Arc<dyn Fn() + Send + Sync>>) {
        self.0.borrow_mut().on_present = notifier;
    }

    /// Observes the upstream cef-rs GPU-to-GPU ownership copy. This is a
    /// *wall-clock* import time, not a GPU timestamp query.
    pub fn note_import(&self, pixels: u64, elapsed: Duration) {
        let mut state = self.0.borrow_mut();
        if !state.perf {
            return;
        }
        let s = &mut state.stats;
        s.started.get_or_insert_with(Instant::now);
        s.imported_frames += 1;
        s.bytes_owned_imports += pixels.saturating_mul(4);
        s.import_wall += elapsed;
        s.import_wall_max = s.import_wall_max.max(elapsed);
    }

    /// Can only be used when CEF explicitly reports no dirty rectangles, and
    /// a prior valid GPU image is already available for this tab.
    pub fn note_empty_damage_skip(&self, pixels: u64) {
        let mut state = self.0.borrow_mut();
        if !state.perf {
            return;
        }
        let s = &mut state.stats;
        s.started.get_or_insert_with(Instant::now);
        s.skipped_imports += 1;
        s.estimated_import_savings += pixels.saturating_mul(4);
    }

    pub fn set_device(&self, device: wgpu::Device, queue: wgpu::Queue) {
        let mut state = self.0.borrow_mut();
        let blit = Rc::new(Blit::new(&device));
        state.device = Some(GpuDevice { device, queue });
        state.blit = Some(blit);
        state.failure = None;
        state.teardown_at = None;
        state.surfaces.clear();
        if state.lifecycle {
            eprintln!(
                "[aster:lifecycle] gpu_renderer_setup device_ready=1 buffers={}",
                state.count.get()
            );
        }
    }

    pub fn device(&self) -> Option<GpuDevice> {
        self.0.borrow().device.clone()
    }

    pub fn has_device(&self) -> bool {
        self.0.borrow().device.is_some()
    }

    /// RenderingTeardown can be a normal window close or a short renderer
    /// restart. Suppress transient missing-device errors while Slint is
    /// handing off or destroying its graphics resources.
    pub fn renderer_teardown_pending(&self) -> bool {
        let state = self.0.borrow();
        state.device.is_none() && state.teardown_at.is_some()
    }

    /// A renderer that stays unavailable after a grace period needs a CPU
    /// fallback, unlike one being destroyed as the application exits.
    pub fn renderer_teardown_timed_out(&self, grace: Duration) -> bool {
        let state = self.0.borrow();
        state.device.is_none() && state.teardown_at.is_some_and(|at| at.elapsed() >= grace)
    }

    pub fn take_failure(&self) -> Option<String> {
        self.0.borrow_mut().failure.take()
    }

    pub fn fail(&self, reason: impl Into<String>) {
        let mut state = self.0.borrow_mut();
        if state.failure.is_none() {
            state.failure = Some(reason.into());
        }
        state.surfaces.clear();
    }

    pub fn clear(&self) {
        let mut state = self.0.borrow_mut();
        if state.lifecycle {
            eprintln!(
                "[aster:lifecycle] gpu_renderer_teardown device_ready={} surfaces_released={} buffers={}",
                state.device.is_some(),
                state.surfaces.len(),
                state.count.get()
            );
        }
        state.teardown_at = Some(Instant::now());
        state.device = None;
        state.blit = None;
        state.surfaces.clear();
    }

    pub fn remove(&self, id: TabId) {
        self.0.borrow_mut().surfaces.remove(&id);
    }

    pub fn revision(&self, id: TabId) -> Option<u64> {
        let state = self.0.borrow();
        if state.failure.is_some() {
            return None;
        }
        state.surfaces.get(&id).map(|s| s.revision)
    }

    pub fn current(&self, id: TabId) -> Option<PresentedTexture> {
        let state = self.0.borrow();
        if state.failure.is_some() {
            return None;
        }
        let surface = state.surfaces.get(&id)?;
        Some(PresentedTexture {
            texture: surface.slots.get(surface.front)?.texture.clone(),
            revision: surface.revision,
            submitted_at: surface.submitted_at,
        })
    }

    /// Composite CEF's owned GPU source onto an independent presentation slot.
    /// The source belongs to cef-rs until its full GPU-to-GPU import is done.
    /// We only submit GPU work on Slint's own queue, so queue ordering avoids
    /// another forced wait unless explicitly requested for A/B comparison.
    pub fn present(
        &self,
        id: TabId,
        source: &wgpu::Texture,
        dirty: &[DamageRect],
    ) -> Result<(), String> {
        let mut state = self.0.borrow_mut();
        let ctx = state.device.clone().ok_or("WGPU device not initialized")?;
        let blit = state
            .blit
            .clone()
            .ok_or("WGPU compositing pipeline not initialized")?;
        let options = state.options;
        let extent = source.size();
        let (width, height) = (extent.width, extent.height);
        if width == 0 || height == 0 || extent.depth_or_array_layers != 1 {
            return Err("invalid CEF GPU frame dimensions".into());
        }
        let started = Instant::now();
        let format = source.format();
        if !matches!(
            format,
            wgpu::TextureFormat::Bgra8Unorm | wgpu::TextureFormat::Rgba8Unorm
        ) {
            return Err(format!("CEF GPU frame format {format:?} not supported"));
        }
        let count = state.count.get();
        let resized = state.surfaces.get(&id).is_none_or(|surface| {
            surface.width != width || surface.height != height || surface.format != format
        });
        let mut encoder = ctx
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("Aster incremental CEF GPU composition"),
            });
        let mut full_copies = 0_u64;
        let mut partial_copies = 0_u64;
        let mut written_bytes = 0_u64;
        let mut raw_pixels = 0_u64;
        let mut unique_pixels = 0_u64;
        let mut frame_pixels = 0_u64;
        let mut lazy_full = 0_u64;
        let mut input_rects = 0_u64;
        let mut plan_reason = "initial_surface";
        let full_pixels = u64::from(width) * u64::from(height);
        let full_rect = [DamageRect {
            x: 0,
            y: 0,
            width,
            height,
        }];
        let planning_start = Instant::now();
        if resized {
            let next_revision = state
                .surfaces
                .get(&id)
                .map_or(1, |old| old.revision.saturating_add(1));
            let mut slots = Vec::with_capacity(count);
            for index in 0..count {
                let target = ctx.device.create_texture(&wgpu::TextureDescriptor {
                    label: Some("Aster GPU presentation buffer"),
                    size: extent,
                    mip_level_count: 1,
                    sample_count: 1,
                    dimension: wgpu::TextureDimension::D2,
                    format: wgpu::TextureFormat::Rgba8Unorm,
                    usage: wgpu::TextureUsages::TEXTURE_BINDING
                        | wgpu::TextureUsages::RENDER_ATTACHMENT,
                    view_formats: &[],
                });
                let initialize = !options.lazy_init || index == 0;
                if initialize {
                    blit.compose(&ctx.device, &mut encoder, source, &target, &full_rect, true);
                    written_bytes += full_pixels * 4;
                    full_copies += 1;
                    raw_pixels += full_pixels;
                    unique_pixels += full_pixels;
                    frame_pixels += full_pixels;
                }
                slots.push(Slot {
                    texture: target,
                    pending: Vec::new(),
                    initialized: initialize,
                });
            }
            state.surfaces.insert(
                id,
                GpuSurface {
                    slots,
                    front: 0,
                    revision: next_revision,
                    width,
                    height,
                    format,
                    submitted_at: Instant::now(),
                },
            );
        } else {
            let surface = state.surfaces.get_mut(&id).expect("checked above");
            // Unspecified damage remains a full-frame update, never a skip.
            let rects = if dirty.is_empty() {
                &full_rect[..]
            } else {
                dirty
            };
            for slot in &mut surface.slots {
                slot.pending
                    .extend(rects.iter().filter_map(|r| r.clipped(width, height)));
            }
            let target_index = (surface.front + 1) % surface.slots.len();
            let target = &mut surface.slots[target_index];
            let plan = plan_damage(&target.pending, width, height, options);
            raw_pixels = plan.raw_pixels;
            unique_pixels = plan.unique_pixels;
            frame_pixels = full_pixels;
            input_rects = plan.incoming_rects as u64;
            plan_reason = plan.reason;
            let full = plan.full || !target.initialized;
            let selected = if full {
                &full_rect[..]
            } else {
                &plan.regions[..]
            };
            blit.compose(
                &ctx.device,
                &mut encoder,
                source,
                &target.texture,
                selected,
                !target.initialized,
            );
            if full {
                written_bytes += full_pixels * 4;
                full_copies += 1;
                if !target.initialized {
                    lazy_full += 1;
                }
            } else {
                for rect in selected {
                    written_bytes += rect.area() * 4;
                    partial_copies += 1;
                }
            }
            target.initialized = true;
            target.pending.clear();
            surface.front = target_index;
            surface.revision += 1;
        }
        let plan_cpu = planning_start.elapsed();
        let submission = ctx.queue.submit([encoder.finish()]);
        if let Some(surface) = state.surfaces.get_mut(&id) {
            surface.submitted_at = Instant::now();
        }
        let mut sync_wait = Duration::ZERO;
        if options.sync == GpuSyncMode::Wait {
            let wait_start = Instant::now();
            ctx.device
                .poll(wgpu::PollType::Wait {
                    submission_index: Some(submission),
                    timeout: Some(Duration::from_secs(2)),
                })
                .map_err(|error| format!("GPU diagnostic sync wait failed: {error:?}"))?;
            sync_wait = wait_start.elapsed();
        }
        if state.perf {
            let stats = &mut state.stats;
            stats.started.get_or_insert_with(Instant::now);
            stats.frames += 1;
            stats.full_copies += full_copies;
            stats.partial_copies += partial_copies;
            stats.bytes_presentation_copies += written_bytes;
            stats.raw_damage_pixels += raw_pixels;
            stats.unique_damage_pixels += unique_pixels;
            stats.planned_frame_pixels += frame_pixels;
            stats.scissor_draws += full_copies + partial_copies;
            stats.lazy_full_initializations += lazy_full;
            stats.input_rects += input_rects;
            *stats.plan_reasons.entry(plan_reason).or_insert(0) += 1;
            stats.plan_cpu += plan_cpu;
            stats.sync_wait += sync_wait;
            stats.sync_wait_max = stats.sync_wait_max.max(sync_wait);
            let elapsed = started.elapsed();
            stats.copy_time += elapsed;
            stats.max_copy_time = stats.max_copy_time.max(elapsed);
        }
        let notifier = state.on_present.clone();
        drop(state); // Never notify Slint while the hub is mutably borrowed.
        if let Some(notify) = notifier {
            notify();
        }
        Ok(())
    }

    pub fn report_if_due(&self) {
        let mut state = self.0.borrow_mut();
        if !state.perf {
            return;
        }
        let Some(since) = state.stats.started else {
            return;
        };
        if since.elapsed() < Duration::from_secs(2) {
            return;
        }
        let s = &state.stats;
        let seconds = since.elapsed().as_secs_f64();
        let full_pixels = s.planned_frame_pixels.max(1) as f64;
        eprintln!(
            "[aster:gpu] imported={:.1}/s importer_full_copy={:.1}MiB/s presentation_writes={:.1}MiB/s full_regions={} partial_regions={} submission_cpu_avg={:.2}ms max={:.2}ms buffers={} damage={:?} raw_damage_pct={:.1} unique_damage_pct={:.1} overlap_saved_pct={:.1} draws={} lazy_full={} import_filter_skips={} import_saved_estimate={:.1}MiB/s import_wall_avg={:.2}ms plan_avg={:.3}ms sync={:?} sync_wait_avg={:.2}ms sync_wait_max={:.2}ms input_rects={} reasons={:?}",
            s.frames as f64 / seconds,
            s.bytes_owned_imports as f64 / 1048576.0 / seconds,
            s.bytes_presentation_copies as f64 / 1048576.0 / seconds,
            s.full_copies,
            s.partial_copies,
            s.copy_time.as_secs_f64() * 1000.0 / s.frames.max(1) as f64,
            s.max_copy_time.as_secs_f64() * 1000.0,
            state.count.get(),
            state.options.damage,
            100.0 * s.raw_damage_pixels as f64 / full_pixels,
            100.0 * s.unique_damage_pixels as f64 / full_pixels,
            100.0 * s.raw_damage_pixels.saturating_sub(s.unique_damage_pixels) as f64
                / s.raw_damage_pixels.max(1) as f64,
            s.scissor_draws,
            s.lazy_full_initializations,
            s.skipped_imports,
            s.estimated_import_savings as f64 / 1048576.0 / seconds,
            s.import_wall.as_secs_f64() * 1000.0 / s.imported_frames.max(1) as f64,
            s.plan_cpu.as_secs_f64() * 1000.0 / s.frames.max(1) as f64,
            state.options.sync,
            s.sync_wait.as_secs_f64() * 1000.0 / s.frames.max(1) as f64,
            s.sync_wait_max.as_secs_f64() * 1000.0,
            s.input_rects,
            s.plan_reasons
        );
        state.stats = GpuStats::default();
    }
}

/// How much of a frame needs copying to make an inactive buffer coherent.
/// Kept pure for deterministic testing without creating a native GPU device.
pub fn accumulated_damage(
    previous: &[DamageRect],
    current: &[DamageRect],
    width: u32,
    height: u32,
) -> Vec<DamageRect> {
    previous
        .iter()
        .chain(current)
        .filter_map(|r| r.clipped(width, height))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn rendering_teardown_is_not_an_immediate_device_failure() {
        let hub = GpuHub::new(BufferCount::default());
        assert!(!hub.renderer_teardown_pending());
        assert!(!hub.renderer_teardown_timed_out(Duration::from_secs(3)));
        hub.clear(); // Slint can call this during normal window destruction.
        assert!(hub.renderer_teardown_pending());
        assert!(!hub.renderer_teardown_timed_out(Duration::from_secs(3)));
        // A renderer that did not recover still needs a fallback.
        hub.0.borrow_mut().teardown_at = Some(Instant::now() - Duration::from_secs(5));
        assert!(hub.renderer_teardown_timed_out(Duration::from_secs(3)));
    }

    #[test]
    fn older_buffer_replays_both_generations() {
        let a = DamageRect {
            x: 1,
            y: 2,
            width: 1,
            height: 1,
        };
        let b = DamageRect {
            x: 4,
            y: 5,
            width: 2,
            height: 2,
        };
        assert_eq!(accumulated_damage(&[a], &[b], 20, 20), vec![a, b]);
    }
    #[test]
    fn three_generations_of_damage_are_preserved_before_slot_reuse() {
        let a = DamageRect {
            x: 1,
            y: 0,
            width: 1,
            height: 1,
        };
        let b = DamageRect {
            x: 2,
            y: 0,
            width: 1,
            height: 1,
        };
        let c = DamageRect {
            x: 3,
            y: 0,
            width: 1,
            height: 1,
        };
        // A third presentation texture may have missed all three changes.
        let missed = accumulated_damage(&[a, b], &[c], 10, 10);
        let plan = plan_damage(
            &missed,
            10,
            10,
            GpuOptions {
                damage: browser_contracts::GpuDamageMode::Adaptive,
                rect_penalty_pixels: 0,
                ..GpuOptions::default()
            },
        );
        assert!(!plan.full);
        assert_eq!(plan.unique_pixels, 3);
        assert_eq!(plan.regions.iter().map(|r| r.area()).sum::<u64>(), 3);
    }
    #[test]
    fn damage_clips_to_surface() {
        let r = DamageRect {
            x: 5,
            y: 5,
            width: 20,
            height: 20,
        };
        assert_eq!(
            accumulated_damage(&[], &[r], 8, 7),
            vec![DamageRect {
                x: 5,
                y: 5,
                width: 3,
                height: 2
            }]
        );
    }
}
