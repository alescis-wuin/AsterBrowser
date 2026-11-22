//! P3 Linux Wayland/X11 preflight. Does not bypass CEF's GPU sandbox.
use browser_contracts::LinuxDisplayMode;
use std::{
    os::unix::fs::FileTypeExt,
    path::{Path, PathBuf},
};

fn wayland_socket(display: &str, runtime: Option<&str>) -> Option<PathBuf> {
    if display.trim().is_empty() {
        return None;
    }
    let path = Path::new(display);
    if path.is_absolute() {
        Some(path.to_path_buf())
    } else {
        runtime.map(|base| Path::new(base).join(path))
    }
}

fn is_socket(path: &Path) -> bool {
    std::fs::metadata(path).is_ok_and(|metadata| metadata.file_type().is_socket())
}

pub fn preflight(mode: LinuxDisplayMode) -> Result<(), String> {
    let display = std::env::var("DISPLAY").ok();
    let wayland = std::env::var("WAYLAND_DISPLAY").ok();
    let runtime = std::env::var("XDG_RUNTIME_DIR").ok();
    let socket = wayland
        .as_deref()
        .and_then(|d| wayland_socket(d, runtime.as_deref()));
    let has_socket = socket.as_deref().is_some_and(is_socket);
    let ozone = match mode {
        LinuxDisplayMode::Auto => {
            if display.is_some() {
                "x11"
            } else if has_socket {
                "wayland"
            } else {
                "none"
            }
        }
        LinuxDisplayMode::X11 => "x11",
        LinuxDisplayMode::Wayland => "wayland",
        LinuxDisplayMode::None => "none",
    };
    let supplied_switch = std::env::args()
        .skip(1)
        .find_map(|arg| arg.strip_prefix("--ozone-platform=").map(str::to_owned));
    if let Some(ref override_value) = supplied_switch {
        if mode != LinuxDisplayMode::Auto
            && mode != LinuxDisplayMode::None
            && override_value != ozone
        {
            return Err(format!(
                "conflicting --ozone-platform={override_value} with --linux-display={ozone}"
            ));
        }
    }
    if mode == LinuxDisplayMode::Wayland && !has_socket {
        return Err("native Wayland requested but WAYLAND_DISPLAY / XDG_RUNTIME_DIR does not identify an accessible Wayland socket".into());
    }
    if mode == LinuxDisplayMode::Wayland && display.is_some() {
        return Err("strict native Wayland requires DISPLAY unset; use: env -u DISPLAY ... --linux-display wayland".into());
    }
    if mode == LinuxDisplayMode::X11 && display.is_none() {
        return Err("X11 requested but DISPLAY is not set".into());
    }
    let slint = std::env::var("SLINT_BACKEND").unwrap_or_else(|_| "default/winit".into());
    eprintln!(
        "[aster:display] requested={:?} cef_ozone={} wayland_socket={} display={} slint_backend={} native_wayland_candidate={}",
        mode,
        ozone,
        has_socket,
        display.is_some(),
        slint,
        ozone == "wayland" && has_socket
    );
    if mode == LinuxDisplayMode::Wayland && slint != "default/winit" && !slint.starts_with("winit")
    {
        return Err(format!(
            "native Wayland requires Slint's Winit backend, but SLINT_BACKEND={slint}"
        ));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn resolves_relative_and_absolute_wayland_sockets() {
        assert_eq!(
            wayland_socket("wayland-0", Some("/run/user/42")),
            Some(PathBuf::from("/run/user/42/wayland-0"))
        );
        assert_eq!(
            wayland_socket("/tmp/wayland-debug", None),
            Some(PathBuf::from("/tmp/wayland-debug"))
        );
        assert_eq!(wayland_socket("wayland-0", None), None);
        assert_eq!(wayland_socket("", Some("/run/user/42")), None);
    }
}
