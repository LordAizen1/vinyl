//! Making it behave like a widget rather than an app.
//!
//! Everything here exists because `tauri.conf.json` turns the window into a
//! bare, undecorated, taskbar-less rectangle: with no title bar there is nothing
//! to drag, with no taskbar entry there is nothing to recover a lost window
//! from, and nothing keeps it on the desktop layer by itself.

use std::thread;
use std::time::Duration;

use serde::{Deserialize, Serialize};
use tauri::{AppHandle, LogicalPosition, Manager, Monitor, PhysicalPosition, Runtime};

use windows::Win32::Foundation::{HWND, RECT};
use windows::Win32::Graphics::Gdi::{
    GetMonitorInfoW, MonitorFromWindow, MONITORINFO, MONITOR_DEFAULTTONEAREST,
};
use windows::Win32::UI::WindowsAndMessaging::{
    GetClassNameW, GetWindow, IsWindowVisible, SetWindowPos, GW_HWNDNEXT, HWND_BOTTOM,
    SWP_NOACTIVATE, SWP_NOMOVE, SWP_NOSIZE,
};

/// How often to push the widget back to the bottom of the window stack.
const DESKTOP_POLL: Duration = Duration::from_secs(1);

/// Where the window was, in logical pixels.
///
/// Logical, not physical, so restoring onto a differently-scaled monitor puts it
/// where it looks like it was rather than a third of the way across the screen.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Placement {
    pub x: f64,
    pub y: f64,
}

/// Nudges a saved position back onto a monitor that currently exists.
///
/// Unplugging the screen the widget was parked on would otherwise restore it to
/// coordinates nothing can display, leaving it invisible with no taskbar entry
/// to recover it from. Clamped against the nearest available monitor instead.
pub fn clamp_to_visible(
    placement: Placement,
    monitors: &[Monitor],
    size: (f64, f64),
) -> Placement {
    if monitors.is_empty() {
        return placement;
    }

    let visible = monitors.iter().any(|monitor| {
        let scale = monitor.scale_factor();
        let position = monitor.position().to_logical::<f64>(scale);
        let monitor_size = monitor.size().to_logical::<f64>(scale);
        // Enough of the widget on screen to grab hold of, not merely a corner.
        placement.x + size.0 * 0.5 >= position.x
            && placement.x + size.0 * 0.5 <= position.x + monitor_size.width
            && placement.y + 20.0 >= position.y
            && placement.y + 20.0 <= position.y + monitor_size.height
    });

    if visible {
        return placement;
    }

    let primary = &monitors[0];
    let scale = primary.scale_factor();
    let position = primary.position().to_logical::<f64>(scale);
    let monitor_size = primary.size().to_logical::<f64>(scale);

    log::info!("window: saved position is off-screen, bringing it back");
    Placement {
        x: position.x + (monitor_size.width - size.0).max(0.0) / 2.0,
        y: position.y + (monitor_size.height - size.1).max(0.0) / 2.0,
    }
}

/// Puts the window back where it was, if that is still somewhere real.
pub fn restore<R: Runtime>(app: &AppHandle<R>, placement: Option<Placement>, size: (f64, f64)) {
    let Some(placement) = placement else { return };
    let Some(window) = app.get_webview_window("main") else {
        return;
    };

    let monitors = window.available_monitors().unwrap_or_default();
    let safe = clamp_to_visible(placement, &monitors, size);

    if let Err(error) = window.set_position(LogicalPosition::new(safe.x, safe.y)) {
        log::warn!("window: could not restore position ({error})");
        return;
    }

    // A position saved before the taskbar moved, or the resolution changed,
    // can be on a real monitor and still be under the taskbar.
    clamp_into_work_area(app);
}

/// Reads the window's current position, for saving.
pub fn current<R: Runtime>(app: &AppHandle<R>) -> Option<Placement> {
    let window = app.get_webview_window("main")?;
    let scale = window.scale_factor().unwrap_or(1.0);
    let position: PhysicalPosition<i32> = window.outer_position().ok()?;
    let logical = position.to_logical::<f64>(scale);
    Some(Placement {
        x: logical.x,
        y: logical.y,
    })
}

/// The transparent gutter `styles.css` keeps around the chassis, in logical
/// pixels. The window may hang this far off the work area, because those pixels
/// draw nothing: without allowing it the widget could never sit flush in a
/// corner, always floating 16px short of it.
const GUTTER: f64 = 16.0;

/// Keeps the widget inside the usable screen: on it, and above the taskbar.
///
/// Uses the monitor's *work area* rather than its full bounds, which is what
/// excludes the taskbar and any docked toolbar. Judged against the monitor the
/// window currently sits on, so this behaves on a multi-monitor desk.
///
/// Returns `true` if it had to move the window, which the caller uses to avoid
/// reacting to its own correction.
pub fn clamp_into_work_area<R: Runtime>(app: &AppHandle<R>) -> bool {
    let Some(window) = app.get_webview_window("main") else {
        return false;
    };
    let Ok(position) = window.outer_position() else {
        return false;
    };
    let Ok(size) = window.outer_size() else {
        return false;
    };
    let Some(work) = work_area_of(&window) else {
        return false;
    };

    let scale = window.scale_factor().unwrap_or(1.0);
    let slack = (GUTTER * scale).round() as i32;

    // The visible chassis, which is the thing that must stay on screen.
    let left = work.left - slack;
    let top = work.top - slack;
    let right = work.right + slack - size.width as i32;
    let bottom = work.bottom + slack - size.height as i32;

    // max/min ordering matters: on a work area smaller than the widget the
    // bounds cross over, and clamping must still land somewhere sane.
    let x = position.x.clamp(left.min(right), right.max(left));
    let y = position.y.clamp(top.min(bottom), bottom.max(top));

    if x == position.x && y == position.y {
        return false;
    }

    if let Err(error) = window.set_position(PhysicalPosition::new(x, y)) {
        log::warn!("window: could not clamp into the work area ({error})");
        return false;
    }

    true
}

fn work_area_of<R: Runtime>(window: &tauri::WebviewWindow<R>) -> Option<RECT> {
    // Tauri links its own copy of the `windows` crate, so the HWND it hands back
    // is a different type to ours despite being the same handle. Rebuilt from
    // the raw pointer, which is all either one is.
    let handle = HWND(window.hwnd().ok()?.0 as *mut _);
    unsafe {
        let monitor = MonitorFromWindow(handle, MONITOR_DEFAULTTONEAREST);
        let mut info = MONITORINFO {
            cbSize: std::mem::size_of::<MONITORINFO>() as u32,
            ..Default::default()
        };
        GetMonitorInfoW(monitor, &mut info)
            .as_bool()
            // rcWork, not rcMonitor: the difference is the taskbar.
            .then_some(info.rcWork)
    }
}

/// How long the window must sit still before its position is written.
///
/// A drag emits a `Moved` event per pixel, so writing on each one would put
/// hundreds of writes through `config.json` for one gesture.
const SAVE_QUIET: Duration = Duration::from_millis(700);

/// A nudge saying the window moved. Coalesced by the saver thread.
pub struct Saver(pub std::sync::mpsc::Sender<()>);

/// One thread that writes the position once the dragging stops.
///
/// A thread per event would be simpler to write and far worse: a single drag
/// would spawn hundreds, each sleeping out its own debounce.
pub fn spawn_saver<R: Runtime>(app: AppHandle<R>) -> Saver {
    let (tx, rx) = std::sync::mpsc::channel::<()>();

    thread::spawn(move || {
        while rx.recv().is_ok() {
            // Swallow the rest of the gesture; only write once it is over.
            while rx.recv_timeout(SAVE_QUIET).is_ok() {}

            let Some(placement) = current(&app) else { continue };
            let Some(state) = app.try_state::<crate::menu::PrefsState>() else {
                continue;
            };

            let prefs = {
                let mut current = state.0.lock();
                if current.placement == Some(placement) {
                    continue;
                }
                current.placement = Some(placement);
                *current
            };

            log::debug!("window: saving position {:?}", placement);
            crate::menu::persist(&app, prefs);
        }
    });

    Saver(tx)
}

/// Keeps the widget on the desktop, underneath every ordinary window.
///
/// This is what makes it a desktop widget rather than a floating panel: it is
/// visible when the desktop is, and anything you open covers it.
///
/// Re-asserted on a timer rather than set once, because Windows raises a window
/// when it is clicked and there is no reliable event for "someone put something
/// above me". This also replaces hiding for fullscreen apps, which a
/// bottom-most window gets for free.
///
/// The check before the call is not an optimisation. `SetWindowPos` invalidates
/// the overlapped region and forces a repaint even when the Z-order does not
/// actually change, so calling it unconditionally once a second made the
/// widget flicker whenever another window sat over it. Only move it when it is
/// genuinely not at the bottom.
pub fn keep_on_desktop<R: Runtime>(app: AppHandle<R>) {
    thread::spawn(move || loop {
        thread::sleep(DESKTOP_POLL);

        let Some(window) = app.get_webview_window("main") else {
            return;
        };
        let Ok(handle) = window.hwnd() else { continue };
        let ours = HWND(handle.0 as *mut _);

        if !something_ordinary_is_below(ours) {
            continue;
        }

        unsafe {
            let _ = SetWindowPos(
                ours,
                Some(HWND_BOTTOM),
                0,
                0,
                0,
                0,
                SWP_NOMOVE | SWP_NOSIZE | SWP_NOACTIVATE,
            );
        }
    });
}

/// Whether any ordinary window sits below ours in the Z-order.
///
/// "Ordinary" excludes the desktop and the shell, which are always below
/// everything and would otherwise make this report true forever, putting the
/// flicker straight back.
fn something_ordinary_is_below(ours: HWND) -> bool {
    unsafe {
        let mut current = ours;
        loop {
            let Ok(next) = GetWindow(current, GW_HWNDNEXT) else {
                return false;
            };
            if next.0.is_null() {
                return false;
            }
            if IsWindowVisible(next).as_bool() && !is_shell(next) {
                return true;
            }
            current = next;
        }
    }
}

/// The desktop and taskbar windows, by class name.
///
/// Progman and WorkerW are the desktop itself; Shell_TrayWnd and its relatives
/// are the taskbar. All of them legitimately live at the bottom with us.
fn is_shell(window: HWND) -> bool {
    const SHELL: [&str; 5] = [
        "Progman",
        "WorkerW",
        "Shell_TrayWnd",
        "Shell_SecondaryTrayWnd",
        "SysListView32",
    ];

    let mut buffer = [0u16; 64];
    let length = unsafe { GetClassNameW(window, &mut buffer) };
    if length <= 0 {
        return true; // Unreadable: treat as shell rather than churn the Z-order.
    }

    let name = String::from_utf16_lossy(&buffer[..length as usize]);
    SHELL.contains(&name.as_str())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn monitor_rect(left: i32, top: i32, right: i32, bottom: i32) -> RECT {
        RECT {
            left,
            top,
            right,
            bottom,
        }
    }

    /// Mirrors the arithmetic in `clamp_into_work_area`, so the edge cases can
    /// be checked without a real window.
    fn clamp(pos: (i32, i32), size: (i32, i32), work: RECT, slack: i32) -> (i32, i32) {
        let left = work.left - slack;
        let top = work.top - slack;
        let right = work.right + slack - size.0;
        let bottom = work.bottom + slack - size.1;
        (
            pos.0.clamp(left.min(right), right.max(left)),
            pos.1.clamp(top.min(bottom), bottom.max(top)),
        )
    }

    /// 1080 tall screen with a 48px taskbar: the work area stops at 1032.
    fn desk() -> RECT {
        monitor_rect(0, 0, 1920, 1032)
    }

    #[test]
    fn a_window_already_inside_is_left_alone() {
        assert_eq!(clamp((400, 300), (460, 273), desk(), 16), (400, 300));
    }

    #[test]
    fn dragging_off_the_left_is_pulled_back() {
        assert_eq!(clamp((-200, 300), (460, 273), desk(), 16).0, -16);
    }

    #[test]
    fn dragging_below_the_taskbar_is_pulled_above_it() {
        // Not 1080: the taskbar is not usable screen.
        let (_, y) = clamp((400, 1000), (460, 273), desk(), 16);
        assert_eq!(y, 1032 + 16 - 273);
    }

    #[test]
    fn the_transparent_gutter_may_hang_off_the_edge() {
        // Those 16px draw nothing, so letting them overhang is what puts the
        // visible chassis flush against the corner.
        assert_eq!(clamp((-999, -999), (460, 273), desk(), 16), (-16, -16));
    }

    /// Compact parked in the bottom-right, then switched to full size. Growing
    /// keeps the top-left corner fixed, so without a clamp the extra 180px of
    /// width hangs off the screen.
    #[test]
    fn growing_at_the_right_edge_is_pulled_back_on_screen() {
        let compact = (280, 275);
        let full = (460, 273);
        let work = desk();

        // Sitting flush in the bottom-right corner at the compact size.
        let parked = clamp((9999, 9999), compact, work, 16);
        assert_eq!(parked, (work.right + 16 - compact.0, work.bottom + 16 - compact.1));

        // Now the same position with the full size. Its right edge would be at
        // parked.0 + 460, well past the screen.
        assert!(parked.0 + full.0 > work.right + 16, "the bug needs a bug");

        let fixed = clamp(parked, full, work, 16);
        assert_eq!(fixed.0, work.right + 16 - full.0);
        assert!(fixed.0 + full.0 <= work.right + 16);
    }

    #[test]
    fn a_work_area_smaller_than_the_widget_still_lands_somewhere_sane() {
        // Bounds cross over here; the clamp must not panic or fly off.
        let tiny = monitor_rect(0, 0, 200, 200);
        let (x, y) = clamp((50, 50), (460, 273), tiny, 16);
        assert!(x <= 16 && y <= 16, "landed at {x},{y}");
    }
}
