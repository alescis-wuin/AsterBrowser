//! Frontend- and engine-neutral rendering options for browser experiments.
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
}
impl FromStr for BufferCount {
    type Err = String;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.trim() {
            "1" => Ok(Self(1)),
            "2" => Ok(Self(2)),
            _ => Err(format!("invalid buffer count '{s}'; expected 1 or 2")),
        }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct RenderOptions {
    pub mode: RenderMode,
    pub buffers: BufferCount,
}

impl RenderOptions {
    /// Read environment options first, then apply explicit CLI overrides.
    /// Unknown options are left for CEF/Chromium; only Aster's own flags are parsed.
    pub fn from_env_and_args(args: impl IntoIterator<Item = String>) -> Result<Self, String> {
        let mut options = Self::default();
        if let Ok(mode) = std::env::var("ASTER_RENDERER") {
            options.mode = mode.parse()?;
        }
        if let Ok(buffers) = std::env::var("ASTER_BUFFERS") {
            options.buffers = buffers.parse()?;
        }
        options.with_args(args)
    }

    pub fn with_args(mut self, args: impl IntoIterator<Item = String>) -> Result<Self, String> {
        let mut args = args.into_iter();
        while let Some(argument) = args.next() {
            if let Some(value) = argument.strip_prefix("--renderer=") {
                self.mode = value.parse()?;
            } else if argument == "--renderer" {
                self.mode = args.next().ok_or("--renderer requires a value")?.parse()?;
            } else if let Some(value) = argument.strip_prefix("--buffers=") {
                self.buffers = value.parse()?;
            } else if argument == "--buffers" {
                self.buffers = args.next().ok_or("--buffers requires a value")?.parse()?;
            }
        }
        Ok(self)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn buffer_count_is_strict() {
        assert_eq!("1".parse::<BufferCount>().unwrap().get(), 1);
        assert_eq!("2".parse::<BufferCount>().unwrap().get(), 2);
        assert!("0".parse::<BufferCount>().is_err());
        assert!("3".parse::<BufferCount>().is_err());
    }
    #[test]
    fn cli_overrides_and_rejects_invalid_values() {
        let options = RenderOptions::default()
            .with_args(["--renderer=gpu".into(), "--buffers".into(), "2".into()])
            .unwrap();
        assert_eq!(options.mode, RenderMode::Gpu);
        assert_eq!(options.buffers.get(), 2);
        assert!(
            RenderOptions::default()
                .with_args(["--buffers=3".into()])
                .is_err()
        );
        assert_eq!(RenderOptions::default().mode, RenderMode::Cpu);
    }
}
