//! Opt-in Linux Fontconfig repair. Runs before CEF creates any threads.
//! Aster never edits global configuration files and never disables sandboxing.
use std::{
    env,
    path::{Path, PathBuf},
    process::Command,
    time::Instant,
};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
enum Mode {
    #[default]
    Off,
    Auto,
    Force,
}

#[derive(Debug, Default)]
struct Options {
    mode: Mode,
    file: Option<PathBuf>,
    probe: bool,
    child: bool,
}

impl Options {
    fn parse(args: impl IntoIterator<Item = String>) -> Result<Self, String> {
        let mut o = Self::default();
        o.mode = match env::var("ASTER_FONTCONFIG_MODE")
            .ok()
            .as_deref()
            .unwrap_or("off")
        {
            "off" => Mode::Off,
            "auto" => Mode::Auto,
            "force" => Mode::Force,
            other => {
                return Err(format!(
                    "invalid ASTER_FONTCONFIG_MODE={other} (off|auto|force)"
                ));
            }
        };
        o.file = env::var_os("ASTER_FONTCONFIG_FILE").map(PathBuf::from);
        o.probe = env::var("ASTER_FONTCONFIG_PROBE").ok().as_deref() == Some("1");
        let argv: Vec<String> = args.into_iter().collect();
        o.child = argv
            .iter()
            .any(|arg| arg == "--type" || arg.starts_with("--type="));
        let mut args = argv.into_iter();
        while let Some(arg) = args.next() {
            let (key, inline) = if let Some((a, b)) = arg.split_once('=') {
                (a, Some(b.to_owned()))
            } else {
                (arg.as_str(), None)
            };
            match key {
                "--fontconfig" | "--fontconfig-mode" => {
                    let v = inline
                        .or_else(|| args.next())
                        .ok_or("--fontconfig requires off, auto or force")?;
                    o.mode = match v.as_str() {
                        "off" => Mode::Off,
                        "auto" => Mode::Auto,
                        "force" => Mode::Force,
                        _ => return Err(format!("invalid fontconfig mode '{v}' (off|auto|force)")),
                    };
                }
                "--fontconfig-file" => {
                    let v = inline
                        .or_else(|| args.next())
                        .ok_or("--fontconfig-file requires a path")?;
                    o.file = Some(PathBuf::from(v));
                }
                "--fontconfig-probe" => {
                    let v = inline
                        .or_else(|| args.next())
                        .ok_or("--fontconfig-probe requires on or off")?;
                    o.probe = match v.as_str() {
                        "on" | "1" => true,
                        "off" | "0" => false,
                        _ => return Err(format!("invalid fontconfig probe '{v}' (on|off)")),
                    };
                }
                _ => {} // Other arguments belong to CEF or Aster's renderer.
            }
        }
        if o.mode == Mode::Off && o.file.is_some() {
            return Err("--fontconfig-file requires --fontconfig auto or force".into());
        }
        Ok(o)
    }
}

fn readable_file(path: &Path) -> bool {
    path.is_file() && std::fs::File::open(path).is_ok()
}

fn resolve_relative(path: &Path, config_dir: Option<&Path>) -> PathBuf {
    if path.is_absolute() {
        return path.to_owned();
    }
    config_dir.unwrap_or(Path::new("/etc/fonts")).join(path)
}

fn system_config() -> Option<PathBuf> {
    ["/etc/fonts/fonts.conf", "/usr/local/etc/fonts/fonts.conf"]
        .iter()
        .map(PathBuf::from)
        .find(|p| readable_file(p))
}

pub fn configure() -> Result<(), String> {
    let started = Instant::now();
    let options = Options::parse(env::args().skip(1))?;
    let inherited = env::var_os("FONTCONFIG_FILE").map(PathBuf::from);
    let config_dir = env::var_os("FONTCONFIG_PATH").map(PathBuf::from);
    let custom = options.file.as_ref();
    let inherited_valid = inherited
        .as_ref()
        .is_some_and(|f| readable_file(&resolve_relative(f, config_dir.as_deref())));
    let chosen = match options.mode {
        Mode::Off => None,
        Mode::Auto => {
            if let Some(custom) = custom {
                if !readable_file(custom) {
                    return Err(format!(
                        "invalid Aster Fontconfig file: {}",
                        custom.display()
                    ));
                }
                Some(
                    std::fs::canonicalize(custom)
                        .map_err(|e| format!("Fontconfig path resolution failed: {e}"))?,
                )
            } else if inherited_valid {
                None // Preserve a valid explicit user configuration.
            } else {
                system_config()
            }
        }
        Mode::Force => {
            let path = custom.cloned().or_else(system_config).ok_or(
                "Fontconfig: no readable system configuration (check /etc/fonts/fonts.conf)",
            )?;
            if !readable_file(&path) {
                return Err(format!(
                    "Fontconfig file is not readable: {}",
                    path.display()
                ));
            }
            Some(
                std::fs::canonicalize(&path)
                    .map_err(|e| format!("Fontconfig path resolution failed: {e}"))?,
            )
        }
    };
    if let Some(file) = &chosen {
        // SAFETY: configure() is called from desktop main before any CEF,
        // WGPU or Slint thread exists. No concurrent env access in Aster yet.
        unsafe {
            env::set_var("FONTCONFIG_FILE", file);
        }
        // A broken inherited FONTCONFIG_PATH can also hide fonts.dtd and includes.
        if let Some(dir) = file.parent() {
            if options.mode == Mode::Force || config_dir.as_ref().is_some_and(|p| !p.is_dir()) {
                // SAFETY: same early single-threaded startup as above.
                unsafe {
                    env::set_var("FONTCONFIG_PATH", dir);
                }
            }
        }
    }
    let status = if let Some(file) = &chosen {
        format!("selected={} changed=1", file.display())
    } else if inherited_valid {
        "inherited_user_config changed=0".to_owned()
    } else {
        "environment_unchanged changed=0".to_owned()
    };
    if !options.child && (options.mode != Mode::Off || options.probe) {
        eprintln!(
            "[aster:fontconfig] mode={:?} {} setup_ms={:.2}",
            options.mode,
            status,
            started.elapsed().as_secs_f64() * 1000.0
        );
    }
    if options.probe && !options.child {
        let began = Instant::now();
        match Command::new("fc-match")
            .args(["-f", "%{family}\\n", "sans"])
            .output()
        {
            Ok(out) => {
                let family = String::from_utf8_lossy(&out.stdout);
                eprintln!(
                    "[aster:fontconfig] probe_ok={} match={:?} probe_ms={:.2} exit={:?}",
                    out.status.success() && !family.trim().is_empty(),
                    family.lines().next().unwrap_or(""),
                    began.elapsed().as_secs_f64() * 1000.0,
                    out.status.code()
                );
                if !out.status.success() || family.trim().is_empty() {
                    let stderr = String::from_utf8_lossy(&out.stderr);
                    eprintln!("[aster:fontconfig] probe_warning={}", stderr.trim());
                }
            }
            Err(err) => eprintln!(
                "[aster:fontconfig] probe_unavailable={} (fc-match not installed)",
                err
            ),
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn relative_paths_use_fontconfig_directory() {
        assert_eq!(
            resolve_relative(Path::new("fonts.conf"), Some(Path::new("/tmp/fc"))),
            PathBuf::from("/tmp/fc/fonts.conf")
        );
        assert_eq!(
            resolve_relative(Path::new("/some/path.conf"), None),
            PathBuf::from("/some/path.conf")
        );
    }
    #[test]
    fn config_modes_have_distinct_values() {
        assert_ne!(Mode::Auto, Mode::Force);
        assert_eq!(Mode::default(), Mode::Off);
    }
}
