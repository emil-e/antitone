use std::io::{self, Write};
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use windows_sys::Win32::Foundation::{HWND, RECT};
use windows_sys::Win32::UI::WindowsAndMessaging::{
    GetWindowPlacement, SW_HIDE, SW_SHOWMAXIMIZED, SetWindowPlacement, WINDOWPLACEMENT,
};
use winit::raw_window_handle::{HasWindowHandle, RawWindowHandle};
use winit::window::Window;

use crate::platform::host::Result;

#[derive(Default)]
pub(crate) struct WindowPlacement {
    last_windowed: Option<SavedPlacement>,
    maximize_on_reveal: bool,
}

impl WindowPlacement {
    pub(crate) fn restore(&mut self, window: &Window) {
        self.maximize_on_reveal = match placement_path()
            .map_or(Ok(false), |path| restore(window, &path))
        {
            Ok(maximized) => maximized,
            Err(error) => {
                log::warn!(error = error.to_string().as_str(); "window_placement_restore_failed");
                false
            }
        };
    }

    pub(crate) fn on_reveal(&mut self, window: &Window) {
        if std::mem::take(&mut self.maximize_on_reveal) {
            // Maximizing via winit shows the window, so wait for the host's first presentation.
            window.set_maximized(true);
        }
        self.capture(window);
    }

    pub(crate) fn capture(&mut self, window: &Window) {
        // Minimizing a maximized window must not replace its last visible show state.
        if window.is_minimized() == Some(true) {
            return;
        }
        if let Ok(placement) = hwnd(window).and_then(get_placement) {
            self.last_windowed = Some(SavedPlacement::from(placement));
        }
    }

    pub(crate) fn finish(&mut self, window: &Window, windowed: bool) {
        if windowed {
            self.capture(window);
        }
        if let (Some(saved), Some(path)) = (&self.last_windowed, placement_path())
            && let Err(error) = saved.write(&path)
        {
            log::warn!(error = error.to_string().as_str(); "window_placement_save_failed");
        }
    }
}

fn placement_path() -> Option<PathBuf> {
    crate::sys::local_appdata().map(|directory| directory.join("Antitone/window-placement.json"))
}

fn hwnd(window: &Window) -> Result<HWND> {
    let handle = window.window_handle()?;
    let RawWindowHandle::Win32(handle) = handle.as_raw() else {
        return Err("window placement needs a Win32 window handle".into());
    };
    Ok(handle.hwnd.get() as HWND)
}

fn get_placement(hwnd: HWND) -> Result<WINDOWPLACEMENT> {
    // Every field has a valid zero value; Win32 requires the caller to set length.
    let mut placement: WINDOWPLACEMENT = unsafe { std::mem::zeroed() };
    placement.length = std::mem::size_of::<WINDOWPLACEMENT>() as u32;
    if unsafe { GetWindowPlacement(hwnd, &mut placement) } == 0 {
        return Err(io::Error::last_os_error().into());
    }
    Ok(placement)
}

fn restore(window: &Window, path: &Path) -> Result<bool> {
    let Some(saved) = SavedPlacement::read(path)? else {
        return Ok(false);
    };
    let hwnd = hwnd(window)?;
    let mut placement = get_placement(hwnd)?;
    placement.showCmd = SW_HIDE as u32;
    placement.rcNormalPosition = saved.normal_rect()?;
    // These are Win32 workspace coordinates, not winit desktop coordinates. Windows
    // adjusts placements that would otherwise put the window entirely off-screen.
    if unsafe { SetWindowPlacement(hwnd, &placement) } == 0 {
        return Err(io::Error::last_os_error().into());
    }
    Ok(saved.maximized)
}

#[derive(Debug, PartialEq, Serialize, Deserialize)]
struct SavedPlacement {
    normal_bounds: [i32; 4],
    maximized: bool,
}

impl From<WINDOWPLACEMENT> for SavedPlacement {
    fn from(placement: WINDOWPLACEMENT) -> Self {
        let RECT {
            left,
            top,
            right,
            bottom,
        } = placement.rcNormalPosition;
        Self {
            normal_bounds: [left, top, right, bottom],
            maximized: placement.showCmd == SW_SHOWMAXIMIZED as u32,
        }
    }
}

impl SavedPlacement {
    fn normal_rect(&self) -> Result<RECT> {
        let [left, top, right, bottom] = self.normal_bounds;
        if !right.checked_sub(left).is_some_and(|width| width > 0)
            || !bottom.checked_sub(top).is_some_and(|height| height > 0)
        {
            return Err("invalid window placement bounds".into());
        }
        Ok(RECT {
            left,
            top,
            right,
            bottom,
        })
    }

    fn read(path: &Path) -> Result<Option<Self>> {
        match std::fs::read(path) {
            Ok(bytes) => Ok(Some(serde_json::from_slice(&bytes)?)),
            Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(None),
            Err(error) => Err(error.into()),
        }
    }

    fn write(&self, path: &Path) -> Result<()> {
        let parent = path
            .parent()
            .ok_or("window placement has no parent directory")?;
        std::fs::create_dir_all(parent)?;
        let mut temporary = tempfile::NamedTempFile::new_in(parent)?;
        serde_json::to_writer(&mut temporary, self)?;
        temporary.write_all(b"\n")?;
        temporary.as_file().sync_all()?;
        temporary.persist(path)?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn placement_file_round_trips_maximized_with_normal_bounds_on_a_left_monitor() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("Antitone/window-placement.json");
        assert_eq!(SavedPlacement::read(&path).unwrap(), None);

        let placement = WINDOWPLACEMENT {
            showCmd: SW_SHOWMAXIMIZED as u32,
            rcNormalPosition: RECT {
                left: -1420,
                top: 40,
                right: -520,
                bottom: 540,
            },
            // The other native fields aren't part of the persisted placement.
            ..unsafe { std::mem::zeroed() }
        };
        let saved = SavedPlacement::from(placement);
        saved.write(&path).unwrap();
        let restored = SavedPlacement::read(&path).unwrap().unwrap();
        let rect = restored.normal_rect().unwrap();
        assert_eq!(
            [rect.left, rect.top, rect.right, rect.bottom],
            [-1420, 40, -520, 540]
        );
        assert!(restored.maximized);

        let normal = SavedPlacement::from(WINDOWPLACEMENT {
            showCmd: windows_sys::Win32::UI::WindowsAndMessaging::SW_SHOWNORMAL as u32,
            ..placement
        });
        normal.write(&path).unwrap();
        assert!(!SavedPlacement::read(&path).unwrap().unwrap().maximized);
    }

    #[test]
    fn malformed_files_and_invalid_bounds_are_not_native_placement_inputs() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("window-placement.json");
        std::fs::write(&path, r#"{"normal_bounds": [1, 2, 3, 4]}"#).unwrap();
        assert!(SavedPlacement::read(&path).is_err());

        for normal_bounds in [[2, 1, 2, 5], [1, 5, 2, 1], [i32::MIN, 0, i32::MAX, 10]] {
            assert!(
                SavedPlacement {
                    normal_bounds,
                    maximized: false
                }
                .normal_rect()
                .is_err()
            );
        }
    }
}
