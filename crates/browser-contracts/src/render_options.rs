//! Engine/UI-independent rendering experiment options. CLI overrides environment.
use std::str::FromStr;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum RenderMode {
    #[default]
    Cpu,
    Gpu,
    Auto,
}

impl FromStr for RenderMode {
    type Err = String;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.trim().to_ascii_lowercase().as_str() {
            "cpu" => Ok(Self::Cpu),
            "gpu" => Ok(Self::Gpu),
            "auto" => Ok(Self::Auto),
            _ => Err(format!("invalid renderer '{s}'; expected cpu, gpu or auto")),
        }
    }
}

/// Linux Ozone backend for CEF. `Auto` preserves X11/XWayland preference.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum LinuxDisplayMode {
    #[default]
    Auto,
    X11,
    Wayland,
    None,
}
impl LinuxDisplayMode {
    pub fn ozone_override(self) -> &'static str {
        match self {
            Self::Auto => "auto",
            Self::X11 => "x11",
            Self::Wayland => "wayland",
            Self::None => "none",
        }
    }
}
impl FromStr for LinuxDisplayMode {
    type Err = String;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.trim().to_ascii_lowercase().as_str() {
            "auto" => Ok(Self::Auto),
            "x11" => Ok(Self::X11),
            "wayland" => Ok(Self::Wayland),
            "none" => Ok(Self::None),
            _ => Err(format!(
                "invalid Linux display '{s}'; expected auto, x11, wayland or none"
            )),
        }
    }
}

/// CEF/Linux diagnostic choice. Default preserves the proven EGL path.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum CefAngleMode {
    #[default]
    Auto,
    GlEgl,
    Vulkan,
    Default,
}
impl FromStr for CefAngleMode {
    type Err = String;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.trim().to_ascii_lowercase().as_str() {
            "auto" => Ok(Self::Auto),
            "gl-egl" => Ok(Self::GlEgl),
            "vulkan" => Ok(Self::Vulkan),
            "default" => Ok(Self::Default),
            _ => Err(format!(
                "invalid CEF ANGLE mode '{s}'; expected auto, gl-egl, vulkan or default"
            )),
        }
    }
}

/// CPU-first Wayland mode is useful while CEF's accelerated capture is broken.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum WaylandGpuMode {
    #[default]
    Probe,
    Cpu,
}
impl FromStr for WaylandGpuMode {
    type Err = String;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.trim().to_ascii_lowercase().as_str() {
            "probe" => Ok(Self::Probe),
            "cpu" => Ok(Self::Cpu),
            _ => Err(format!(
                "invalid Wayland GPU policy '{s}'; expected probe or cpu"
            )),
        }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum GpuPresentationMode {
    #[default]
    Poll,
    Notify,
}
impl FromStr for GpuPresentationMode {
    type Err = String;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.trim().to_ascii_lowercase().as_str() {
            "poll" => Ok(Self::Poll),
            "notify" => Ok(Self::Notify),
            _ => Err(format!(
                "invalid GPU presentation mode '{s}'; expected poll or notify"
            )),
        }
    }
}

/// CEF/Slint scheduling strategy. The experimental event mode is opt-in.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum PumpMode {
    #[default]
    Legacy,
    Event,
}
impl FromStr for PumpMode {
    type Err = String;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.trim().to_ascii_lowercase().as_str() {
            "legacy" => Ok(Self::Legacy),
            "event" => Ok(Self::Event),
            _ => Err(format!("invalid pump mode '{s}'; expected legacy or event")),
        }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum PumpWakeMode {
    Eager,
    #[default]
    Deadline,
}
impl FromStr for PumpWakeMode {
    type Err = String;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.trim().to_ascii_lowercase().as_str() {
            "eager" => Ok(Self::Eager),
            "deadline" => Ok(Self::Deadline),
            _ => Err(format!(
                "invalid pump wake mode '{s}'; expected eager or deadline"
            )),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PumpOptions {
    pub mode: PumpMode,
    /// Fallback CEF heartbeat in event mode, even if a notification is lost.
    pub idle_ms: u64,
    pub wake: PumpWakeMode,
}
impl Default for PumpOptions {
    fn default() -> Self {
        Self {
            mode: PumpMode::Legacy,
            idle_ms: 33,
            wake: PumpWakeMode::Deadline,
        }
    }
}
fn parse_pump_idle(s: &str) -> Result<u64, String> {
    let ms = s
        .parse::<u64>()
        .map_err(|_| format!("invalid pump idle interval '{s}' (16..=1000 ms)"))?;
    if (16..=1000).contains(&ms) {
        Ok(ms)
    } else {
        Err(format!("invalid pump idle interval '{s}' (16..=1000 ms)"))
    }
}
fn parse_gpu_first_timeout(s: &str) -> Result<u64, String> {
    let ms = s
        .parse::<u64>()
        .map_err(|_| format!("invalid GPU first frame timeout '{s}' (1000..=30000 ms)"))?;
    if (1000..=30000).contains(&ms) {
        Ok(ms)
    } else {
        Err(format!(
            "invalid GPU first frame timeout '{s}' (1000..=30000 ms)"
        ))
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct BufferCount(u8);
impl Default for BufferCount {
    fn default() -> Self {
        Self(1)
    }
}
impl BufferCount {
    pub fn get(self) -> usize {
        usize::from(self.0)
    }
    /// CPU presentation is limited to two buffers; GPU may use three.
    pub fn for_cpu(self) -> Self {
        Self(self.0.min(2))
    }
}
impl FromStr for BufferCount {
    type Err = String;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.trim() {
            "1" => Ok(Self(1)),
            "2" => Ok(Self(2)),
            "3" => Ok(Self(3)),
            _ => Err(format!(
                "invalid buffer count '{s}'; expected 1, 2 or 3 (GPU only)"
            )),
        }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum GpuDamageMode {
    #[default]
    Legacy,
    Adaptive,
    Full,
    Partial,
}
impl FromStr for GpuDamageMode {
    type Err = String;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.trim().to_ascii_lowercase().as_str() {
            "legacy" => Ok(Self::Legacy),
            "adaptive" => Ok(Self::Adaptive),
            "full" => Ok(Self::Full),
            "partial" => Ok(Self::Partial),
            _ => Err(format!(
                "invalid GPU damage mode '{s}'; expected legacy, adaptive, full or partial"
            )),
        }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum GpuSyncMode {
    #[default]
    Queue,
    Wait,
}
impl FromStr for GpuSyncMode {
    type Err = String;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.trim().to_ascii_lowercase().as_str() {
            "queue" => Ok(Self::Queue),
            "wait" => Ok(Self::Wait),
            _ => Err(format!(
                "invalid GPU sync mode '{s}'; expected queue or wait"
            )),
        }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum GpuImportFilter {
    #[default]
    Off,
    EmptyDamage,
}
impl FromStr for GpuImportFilter {
    type Err = String;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.trim().to_ascii_lowercase().as_str() {
            "off" => Ok(Self::Off),
            "empty" => Ok(Self::EmptyDamage),
            _ => Err(format!(
                "invalid GPU import filter '{s}'; expected off or empty"
            )),
        }
    }
}

fn parse_switch(s: &str) -> Result<bool, String> {
    match s.trim().to_ascii_lowercase().as_str() {
        "1" | "on" | "true" => Ok(true),
        "0" | "off" | "false" => Ok(false),
        _ => Err(format!("invalid on/off value '{s}'")),
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct GpuOptions {
    pub damage: GpuDamageMode,
    /// Percentage of unique changed pixels beyond which a full update wins.
    pub threshold_pct: u8,
    /// Estimated cost of one scissor/draw relative to this many copied pixels.
    pub rect_penalty_pixels: u32,
    pub lazy_init: bool,
    pub sync: GpuSyncMode,
    pub import_filter: GpuImportFilter,
}
impl Default for GpuOptions {
    fn default() -> Self {
        Self {
            damage: GpuDamageMode::Legacy,
            threshold_pct: 65,
            rect_penalty_pixels: 4096,
            lazy_init: false,
            sync: GpuSyncMode::Queue,
            import_filter: GpuImportFilter::Off,
        }
    }
}

fn parse_threshold(s: &str) -> Result<u8, String> {
    let n = s
        .parse::<u8>()
        .map_err(|_| format!("invalid GPU threshold '{s}' (1..=100)"))?;
    if (1..=100).contains(&n) {
        Ok(n)
    } else {
        Err(format!("invalid GPU threshold '{s}' (1..=100)"))
    }
}
fn parse_penalty(s: &str) -> Result<u32, String> {
    let n = s
        .parse::<u32>()
        .map_err(|_| format!("invalid GPU rect penalty '{s}' (0..=262144)"))?;
    if n <= 262144 {
        Ok(n)
    } else {
        Err(format!("invalid GPU rect penalty '{s}' (0..=262144)"))
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct RenderOptions {
    pub mode: RenderMode,
    pub buffers: BufferCount,
    pub gpu: GpuOptions,
    pub linux_display: LinuxDisplayMode,
    pub cef_angle: CefAngleMode,
    pub wayland_gpu: WaylandGpuMode,
    pub gpu_first_frame_timeout_ms: u64,
    pub gpu_presentation: GpuPresentationMode,
    pub pump: PumpOptions,
}

impl Default for RenderOptions {
    fn default() -> Self {
        Self {
            mode: RenderMode::default(),
            buffers: BufferCount::default(),
            gpu: GpuOptions::default(),
            linux_display: LinuxDisplayMode::default(),
            cef_angle: CefAngleMode::default(),
            wayland_gpu: WaylandGpuMode::default(),
            gpu_first_frame_timeout_ms: 8000,
            gpu_presentation: GpuPresentationMode::default(),
            pump: PumpOptions::default(),
        }
    }
}
impl RenderOptions {
    pub fn from_env_and_args(args: impl IntoIterator<Item = String>) -> Result<Self, String> {
        let mut options = Self::default();
        if let Ok(v) = std::env::var("ASTER_RENDERER") {
            options.mode = v.parse()?;
        }
        // Existing diagnostic variable remains supported until users migrate.
        if let Ok(v) = std::env::var("ASTER_CEF_OZONE") {
            options.linux_display = v.parse()?;
        }
        if let Ok(v) = std::env::var("ASTER_LINUX_DISPLAY") {
            options.linux_display = v.parse()?;
        }
        if let Ok(v) = std::env::var("ASTER_CEF_ANGLE") {
            options.cef_angle = v.parse()?;
        }
        if let Ok(v) = std::env::var("ASTER_WAYLAND_GPU") {
            options.wayland_gpu = v.parse()?;
        }
        if let Ok(v) = std::env::var("ASTER_GPU_FIRST_FRAME_TIMEOUT_MS") {
            options.gpu_first_frame_timeout_ms = parse_gpu_first_timeout(&v)?;
        }
        if let Ok(v) = std::env::var("ASTER_GPU_PRESENTATION") {
            options.gpu_presentation = v.parse()?;
        }
        if let Ok(v) = std::env::var("ASTER_PUMP_WAKE") {
            options.pump.wake = v.parse()?;
        }
        if let Ok(v) = std::env::var("ASTER_PUMP_MODE") {
            options.pump.mode = v.parse()?;
        }
        if let Ok(v) = std::env::var("ASTER_PUMP_IDLE_MS") {
            options.pump.idle_ms = parse_pump_idle(&v)?;
        }
        if let Ok(v) = std::env::var("ASTER_BUFFERS") {
            options.buffers = v.parse()?;
        }
        if let Ok(v) = std::env::var("ASTER_GPU_DAMAGE") {
            options.gpu.damage = v.parse()?;
        }
        if let Ok(v) = std::env::var("ASTER_GPU_THRESHOLD_PCT") {
            options.gpu.threshold_pct = parse_threshold(&v)?;
        }
        if let Ok(v) = std::env::var("ASTER_GPU_RECT_PENALTY_PIXELS") {
            options.gpu.rect_penalty_pixels = parse_penalty(&v)?;
        }
        if let Ok(v) = std::env::var("ASTER_GPU_LAZY_INIT") {
            options.gpu.lazy_init = parse_switch(&v)?;
        }
        if let Ok(v) = std::env::var("ASTER_GPU_SYNC") {
            options.gpu.sync = v.parse()?;
        }
        if let Ok(v) = std::env::var("ASTER_GPU_IMPORT_FILTER") {
            options.gpu.import_filter = v.parse()?;
        }
        options.with_args(args)
    }

    pub fn with_args(mut self, args: impl IntoIterator<Item = String>) -> Result<Self, String> {
        let mut args = args.into_iter();
        while let Some(argument) = args.next() {
            let (key, value) = if let Some((key, value)) = argument.split_once('=') {
                (key, Some(value.to_owned()))
            } else {
                (argument.as_str(), None)
            };
            let mut take = || {
                value
                    .clone()
                    .map(Ok)
                    .unwrap_or_else(|| args.next().ok_or_else(|| format!("{key} requires a value")))
            };
            match key {
                "--renderer" => self.mode = take()?.parse()?,
                "--linux-display" => self.linux_display = take()?.parse()?,
                "--cef-angle" => self.cef_angle = take()?.parse()?,
                "--wayland-gpu" => self.wayland_gpu = take()?.parse()?,
                "--gpu-first-frame-timeout-ms" => {
                    self.gpu_first_frame_timeout_ms = parse_gpu_first_timeout(&take()?)?
                }
                "--gpu-presentation" => self.gpu_presentation = take()?.parse()?,
                "--pump-wake" => self.pump.wake = take()?.parse()?,
                "--pump-mode" => self.pump.mode = take()?.parse()?,
                "--pump-idle-ms" => self.pump.idle_ms = parse_pump_idle(&take()?)?,
                "--buffers" => self.buffers = take()?.parse()?,
                "--gpu-damage" => self.gpu.damage = take()?.parse()?,
                "--gpu-threshold" => self.gpu.threshold_pct = parse_threshold(&take()?)?,
                "--gpu-rect-penalty" => self.gpu.rect_penalty_pixels = parse_penalty(&take()?)?,
                "--gpu-lazy-init" => self.gpu.lazy_init = parse_switch(&take()?)?,
                "--gpu-sync" => self.gpu.sync = take()?.parse()?,
                "--gpu-import-filter" => self.gpu.import_filter = take()?.parse()?,
                _ => {} // CEF/Chromium owns other switches.
            }
        }
        self.validate()?;
        Ok(self)
    }

    fn validate(&self) -> Result<(), String> {
        if self.mode == RenderMode::Gpu
            && self.linux_display == LinuxDisplayMode::Wayland
            && self.wayland_gpu == WaylandGpuMode::Cpu
        {
            return Err("--renderer gpu conflicts with --wayland-gpu cpu".into());
        }
        if self.mode == RenderMode::Cpu && self.buffers.get() == 3 {
            return Err("--buffers 3 requires --renderer gpu or auto (CPU supports 1 or 2)".into());
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn accepts_gpu_triple_but_not_cpu_triple() {
        assert_eq!("3".parse::<BufferCount>().unwrap().get(), 3);
        assert!("4".parse::<BufferCount>().is_err());
        assert!(
            RenderOptions::default()
                .with_args(["--buffers=3".into()])
                .is_err()
        );
        let v = RenderOptions::default()
            .with_args(["--renderer=gpu".into(), "--buffers=3".into()])
            .unwrap();
        assert_eq!(v.buffers.get(), 3);
        assert_eq!(v.buffers.for_cpu().get(), 2);
    }
    #[test]
    fn parses_independent_experiment_flags() {
        let v = RenderOptions::default()
            .with_args([
                "--renderer=gpu".into(),
                "--buffers=3".into(),
                "--gpu-damage=adaptive".into(),
                "--gpu-threshold=42".into(),
                "--gpu-rect-penalty=128".into(),
                "--gpu-lazy-init=on".into(),
                "--gpu-sync=wait".into(),
                "--gpu-import-filter=empty".into(),
            ])
            .unwrap();
        assert_eq!(v.gpu.damage, GpuDamageMode::Adaptive);
        assert_eq!(v.gpu.threshold_pct, 42);
        assert_eq!(v.gpu.rect_penalty_pixels, 128);
        assert!(v.gpu.lazy_init);
        assert_eq!(v.gpu.sync, GpuSyncMode::Wait);
        assert_eq!(v.gpu.import_filter, GpuImportFilter::EmptyDamage);
        assert!(
            RenderOptions::default()
                .with_args(["--gpu-threshold=0".into()])
                .is_err()
        );
        assert!(
            RenderOptions::default()
                .with_args(["--gpu-lazy-init=perhaps".into()])
                .is_err()
        );
        assert!(
            RenderOptions::default()
                .with_args(["--gpu-sync=invalid".into()])
                .is_err()
        );
    }
    #[test]
    fn wayland_and_event_modes_are_opt_in_and_independent() {
        let default = RenderOptions::default();
        assert_eq!(default.linux_display, LinuxDisplayMode::Auto);
        assert_eq!(default.pump.mode, PumpMode::Legacy);
        let options = default
            .with_args([
                "--renderer=gpu".into(),
                "--linux-display=wayland".into(),
                "--pump-mode=event".into(),
                "--pump-idle-ms=160".into(),
            ])
            .unwrap();
        assert_eq!(options.linux_display, LinuxDisplayMode::Wayland);
        assert_eq!(options.pump.mode, PumpMode::Event);
        assert_eq!(options.pump.idle_ms, 160);
        assert_eq!(options.buffers.get(), 1);
        assert!(
            RenderOptions::default()
                .with_args(["--pump-idle-ms=0".into()])
                .is_err()
        );
        assert!(
            RenderOptions::default()
                .with_args(["--linux-display=invalid".into()])
                .is_err()
        );
        assert!(
            RenderOptions::default()
                .with_args(["--pump-mode=invalid".into()])
                .is_err()
        );
    }
}

#[cfg(test)]
mod p3_p4_hardening_tests {
    use super::*;
    #[test]
    fn independent_modes_and_heartbeat_parse() {
        let opts = RenderOptions::default()
            .with_args([
                "--renderer=auto".into(),
                "--linux-display=wayland".into(),
                "--wayland-gpu=cpu".into(),
                "--cef-angle=default".into(),
                "--pump-mode=event".into(),
                "--pump-wake=deadline".into(),
                "--pump-idle-ms=16".into(),
                "--gpu-presentation=notify".into(),
                "--gpu-first-frame-timeout-ms=3000".into(),
            ])
            .unwrap();
        assert_eq!(opts.pump.idle_ms, 16);
        assert_eq!(opts.pump.wake, PumpWakeMode::Deadline);
        assert_eq!(opts.wayland_gpu, WaylandGpuMode::Cpu);
        assert_eq!(opts.cef_angle, CefAngleMode::Default);
        assert_eq!(opts.gpu_presentation, GpuPresentationMode::Notify);
        assert_eq!(opts.gpu_first_frame_timeout_ms, 3000);
        assert!(
            RenderOptions::default()
                .with_args([
                    "--renderer=gpu".into(),
                    "--linux-display=wayland".into(),
                    "--wayland-gpu=cpu".into()
                ])
                .is_err()
        );
        assert!(
            RenderOptions::default()
                .with_args(["--pump-idle-ms=15".into()])
                .is_err()
        );
        assert!(
            RenderOptions::default()
                .with_args(["--gpu-first-frame-timeout-ms=100".into()])
                .is_err()
        );
    }
}
