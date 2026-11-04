//! Hand the backlight requirements of the current GPU MUX mode to the boot-args
//! helper.
//!
//! On laptops with a MUX switch the internal panel is driven by a different GPU
//! depending on the selected mode, and with it comes a different component that
//! actually controls the backlight:
//!
//! * *Ultimate* (discrete): the panel is wired to the dGPU. The EC/WMI backlight
//!   (`nvidia_wmi_ec_backlight`) still accepts writes and even reports the new
//!   value back, but it no longer drives the panel, so brightness looks stuck.
//!   The NVIDIA driver's own handler (`nvidia_0`) is the only working path, and
//!   it is only installed when the kernel is told to prefer the native backlight
//!   - i.e. with `acpi_backlight=native`.
//! * *Hybrid* / *Integrated*: the panel is driven by the iGPU and
//!   `nvidia_wmi_ec_backlight` is the device that works, so the parameter has to
//!   be removed again (leaving it in place exposes a `nvidia_0` that controls
//!   nothing).
//!
//! `acpi_backlight=` is parsed once during boot, so the parameter can only be
//! changed for the *next* boot - which is when the queued MUX change takes effect
//! as well.
//!
//! asusd runs under `ProtectSystem=strict` and cannot regenerate the boot loader
//! configuration, so it only records the request in [`REQUEST_FILE`] and starts
//! [`HELPER_UNIT`], which runs the unsandboxed `boot-args.sh` helper. See
//! `data/boot-args.sh`.

use std::fs;
use std::process::Command;

use log::{debug, info, warn};
use rog_platform::gpu_pci::{asus_dgpu_disabled, asus_gpu_mux_discreet, asus_gpu_mux_exists};

use crate::error::RogError;

/// Request file read by the helper. Contains `native` or `default`.
pub const REQUEST_FILE: &str = "/etc/asusd/gpu_bootargs";

/// Unit that rewrites the kernel command line to match the request.
pub const HELPER_UNIT: &str = "asusd-bootargs.service";

/// Does the current GPU mode require `acpi_backlight=native`?
///
/// Integrated (`dgpu_disable=1`) and Hybrid (`gpu_mux_mode=1`) both keep the
/// panel on the iGPU, where `nvidia_wmi_ec_backlight` is the working device.
pub fn wanted_native(dgpu_disabled: bool, mux_discreet: bool) -> bool {
    !dgpu_disabled && mux_discreet
}

/// Bring the kernel command line in line with the GPU mode reported by the
/// firmware.
///
/// Returns `Ok(None)` on machines without a GPU MUX, where the parameter is
/// irrelevant.
pub fn sync_from_hardware() -> Result<Option<bool>, RogError> {
    if !asus_gpu_mux_exists() {
        return Ok(None);
    }
    let enabled = wanted_native(asus_dgpu_disabled()?, asus_gpu_mux_discreet()?);
    request(enabled).map(Some)
}

/// Record the required backlight mode and ask the helper to apply it.
///
/// Returns whether the request changed. The helper is started even when the
/// request did not change, so a hand-edited or otherwise drifted boot
/// configuration gets corrected; it exits without touching anything when the
/// kernel command line already matches.
pub fn request(native: bool) -> Result<bool, RogError> {
    let wanted = if native { "native" } else { "default" };
    let current = fs::read_to_string(REQUEST_FILE).unwrap_or_default();
    let changed = current.trim() != wanted;

    if changed {
        fs::write(REQUEST_FILE, format!("{wanted}\n"))
            .map_err(|e| RogError::Write(REQUEST_FILE.to_string(), e))?;
        info!("boot_cmdline: requested acpi_backlight=native={native}");
    }

    match Command::new("systemctl")
        .args(["start", "--no-block", HELPER_UNIT])
        .output()
    {
        Ok(out) if out.status.success() => {}
        Ok(out) => warn!(
            "boot_cmdline: could not start {HELPER_UNIT}: {}",
            String::from_utf8_lossy(&out.stderr).trim()
        ),
        Err(e) => warn!("boot_cmdline: could not run systemctl: {e}"),
    }

    debug!("boot_cmdline: request handled (changed: {changed})");
    Ok(changed)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn wanted_native_matches_gpu_modes() {
        // Ultimate: dGPU only.
        assert!(wanted_native(false, true));
        // Hybrid: dgpu enabled, MUX on Optimus.
        assert!(!wanted_native(false, false));
        // Integrated: dGPU disabled.
        assert!(!wanted_native(true, false));
        assert!(!wanted_native(true, true));
    }
}
