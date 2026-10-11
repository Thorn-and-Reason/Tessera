//! Windows 11 caption colours; older Windows retains its native caption on failure.
use eframe::egui;
use raw_window_handle::{HasWindowHandle, RawWindowHandle};
use windows_sys::Win32::Graphics::Dwm::{
    DwmSetWindowAttribute, DWMWA_BORDER_COLOR, DWMWA_CAPTION_COLOR, DWMWA_TEXT_COLOR,
};

use windows_sys::Win32::UI::{
    Accessibility::{HCF_HIGHCONTRASTON, HIGHCONTRASTW},
    WindowsAndMessaging::{SystemParametersInfoW, SPI_GETHIGHCONTRAST},
};

#[derive(Clone, Default)]
struct CaptionState {
    attempted_theme: Option<(bool, bool)>,
    warned: bool,
}

impl CaptionState {
    fn needs_update(&self, dark: bool, high_contrast: bool) -> bool {
        self.attempted_theme != Some((dark, high_contrast))
    }
}

const fn colorref(red: u8, green: u8, blue: u8) -> u32 {
    red as u32 | (green as u32) << 8 | (blue as u32) << 16
}

fn high_contrast_or_unknown() -> bool {
    let mut settings = HIGHCONTRASTW {
        cbSize: std::mem::size_of::<HIGHCONTRASTW>() as u32,
        ..Default::default()
    };
    // SPI writes a fixed-size HIGHCONTRASTW into this live, initialized buffer.
    let success = unsafe {
        SystemParametersInfoW(
            SPI_GETHIGHCONTRAST,
            settings.cbSize,
            (&mut settings as *mut HIGHCONTRASTW).cast(),
            0,
        )
    };
    success == 0 || settings.dwFlags & HCF_HIGHCONTRASTON != 0
}

pub(super) fn sync(frame: &eframe::Frame, ctx: &egui::Context) {
    let Ok(handle) = frame.window_handle() else {
        return;
    };
    let RawWindowHandle::Win32(handle) = handle.as_raw() else {
        return;
    };
    let id = egui::Id::new(("native-caption", handle.hwnd.get()));
    let mut state = ctx.data(|data| data.get_temp::<CaptionState>(id).unwrap_or_default());
    let dark = ctx.theme() == egui::Theme::Dark;
    let high_contrast = high_contrast_or_unknown();
    if !state.needs_update(dark, high_contrast) {
        return;
    }
    let (caption, text) = if high_contrast {
        (u32::MAX, u32::MAX)
    } else if dark {
        (colorref(27, 37, 52), colorref(242, 235, 220))
    } else {
        (colorref(242, 235, 220), colorref(38, 52, 73))
    };
    let mut failure = None;
    for (attribute, color) in [
        (DWMWA_CAPTION_COLOR, caption),
        (DWMWA_TEXT_COLOR, text),
        (DWMWA_BORDER_COLOR, caption),
    ] {
        // The borrowed HWND remains valid for this call; DWM copies the COLORREF.
        let result = unsafe {
            DwmSetWindowAttribute(
                handle.hwnd.get() as _,
                attribute as u32,
                (&color as *const u32).cast(),
                std::mem::size_of::<u32>() as u32,
            )
        };
        if result < 0 {
            failure = Some(result);
        }
    }
    if let Some(result) = failure {
        if !state.warned {
            eprintln!("Native caption colours unavailable ({result:#x}); using Windows fallback.");
            state.warned = true;
        }
    }
    // Cache failures too: Windows 10 lacks these attributes; retry only after a theme or contrast change.
    state.attempted_theme = Some((dark, high_contrast));
    ctx.data_mut(|data| data.insert_temp(id, state));
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn packs_windows_colorref_in_rgb_byte_order() {
        assert_eq!(colorref(27, 37, 52), 0x0034_251b);
        assert_eq!(colorref(242, 235, 220), 0x00dc_ebf2);
        assert_eq!(colorref(38, 52, 73), 0x0049_3426);
    }

    #[test]
    fn retries_when_high_contrast_toggles_in_either_direction() {
        let normal = CaptionState {
            attempted_theme: Some((true, false)),
            warned: false,
        };
        assert!(normal.needs_update(true, true));
        let high_contrast = CaptionState {
            attempted_theme: Some((true, true)),
            warned: false,
        };
        assert!(!high_contrast.needs_update(true, true));
        assert!(high_contrast.needs_update(true, false));
    }

    #[test]
    fn skips_repeated_attempts_but_retries_theme_changes() {
        let initial = CaptionState::default();
        assert!(initial.needs_update(true, false));
        let attempted = CaptionState {
            attempted_theme: Some((true, false)),
            warned: true,
        };
        assert!(!attempted.needs_update(true, false));
        assert!(attempted.needs_update(false, false));
    }
}
