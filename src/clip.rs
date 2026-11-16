//! Dateien in der System-Zwischenablage: unter Windows CF_HDROP + „Preferred DropEffect“
//! (Kopieren bzw. Ausschneiden). So lassen sich Dateien zwischen Foldex, dem Windows-Explorer,
//! VS Code, Mail-Programmen usw. austauschen. Andere Systeme: nur die interne Zwischenablage.

use std::path::PathBuf;

#[cfg(windows)]
mod imp {
    use clipboard_win::{formats::CF_HDROP, options::DoClear, raw, Clipboard};
    use std::path::PathBuf;

    const DROPEFFECT_COPY: u32 = 1;
    const DROPEFFECT_MOVE: u32 = 2;

    fn effect_format() -> Option<u32> {
        raw::register_format("Preferred DropEffect").map(|n| n.get())
    }

    /// Legt Dateien in die Zwischenablage. Gibt die Sequenznummer danach zurück (0 = fehlgeschlagen).
    pub fn set(paths: &[PathBuf], cut: bool) -> u32 {
        {
            let Ok(_open) = Clipboard::new_attempts(10) else { return 0 };
            let list: Vec<String> = paths.iter().map(|p| p.to_string_lossy().into_owned()).collect();
            if raw::set_file_list_with(&list, DoClear).is_err() {
                return 0;
            }
            if let Some(f) = effect_format() {
                let e = if cut { DROPEFFECT_MOVE } else { DROPEFFECT_COPY };
                let _ = raw::set_without_clear(f, &e.to_le_bytes());
            }
        }
        // Windows zählt die Sequenznummer erst beim Schließen der Zwischenablage hoch
        seq()
    }

    /// Strg+V gedrückt? egui meldet Strg+V nur, wenn Text in der Zwischenablage liegt –
    /// bei reinen Dateien (z. B. nach Strg+X) kommt gar kein Ereignis an.
    pub fn paste_keys_down() -> bool {
        // Zustand der Tasten laut den Nachrichten dieses Fensters (kein globales Abfragen der Tastatur)
        use windows_sys::Win32::UI::Input::KeyboardAndMouse::{GetKeyState, VK_CONTROL, VK_MENU};
        let down = |vk: u16| unsafe { GetKeyState(vk as i32) } < 0;
        down(VK_CONTROL) && !down(VK_MENU) && down(b'V' as u16)
    }

    /// Dateien aus der Zwischenablage (auch von anderen Programmen) und ob sie ausgeschnitten wurden.
    pub fn get() -> Option<(Vec<PathBuf>, bool)> {
        if !raw::is_format_avail(CF_HDROP) {
            return None;
        }
        let _open = Clipboard::new_attempts(10).ok()?;
        let mut v = Vec::new();
        raw::get_file_list_path(&mut v).ok()?;
        let cut = effect_format()
            .filter(|&f| raw::is_format_avail(f))
            .and_then(|f| {
                let mut b = [0u8; 4];
                raw::get(f, &mut b).ok().filter(|&n| n >= 4)?;
                Some(u32::from_le_bytes(b) & DROPEFFECT_MOVE != 0)
            })
            .unwrap_or(false);
        (!v.is_empty()).then_some((v, cut))
    }

    pub fn has_files() -> bool {
        raw::is_format_avail(CF_HDROP)
    }

    /// Ändert sich bei jedem Schreiben in die Zwischenablage – auch durch andere Programme.
    pub fn seq() -> u32 {
        raw::seq_num().map_or(0, |n| n.get())
    }

    /// Leert die Zwischenablage, sofern seit `owned` niemand anderes hineingeschrieben hat.
    pub fn clear_if_ours(owned: u32) {
        if owned != 0 && seq() == owned {
            if let Ok(_open) = Clipboard::new_attempts(10) {
                let _ = raw::empty();
            }
        }
    }
}

#[cfg(not(windows))]
mod imp {
    use std::path::PathBuf;
    pub fn set(_paths: &[PathBuf], _cut: bool) -> u32 {
        0
    }
    pub fn get() -> Option<(Vec<PathBuf>, bool)> {
        None
    }
    pub fn has_files() -> bool {
        false
    }
    pub fn seq() -> u32 {
        0
    }
    pub fn clear_if_ours(_owned: u32) {}
    pub fn paste_keys_down() -> bool {
        false
    }
}

pub use imp::{clear_if_ours, get, has_files, paste_keys_down, seq, set};

/// Interne Sicht auf die Zwischenablage: Pfade, ausgeschnitten?, Sequenznummer unseres Eintrags.
#[derive(Clone)]
pub struct Clip {
    pub paths: Vec<PathBuf>,
    pub cut: bool,
    pub seq: u32,
}

/// Bild in die Zwischenablage (Windows: als Bitmap; Transparenz wird auf Weiß gelegt).
#[cfg(windows)]
pub fn set_image(img: &image::DynamicImage) -> Result<(), String> {
    let rgba = img.to_rgba8();
    let rgb = image::RgbImage::from_fn(rgba.width(), rgba.height(), |x, y| {
        let p = rgba.get_pixel(x, y).0;
        let a = p[3] as u32;
        let mix = |c: u8| ((c as u32 * a + 255 * (255 - a)) / 255) as u8;
        image::Rgb([mix(p[0]), mix(p[1]), mix(p[2])])
    });
    let mut bmp = Vec::new();
    image::DynamicImage::ImageRgb8(rgb).write_to(&mut std::io::Cursor::new(&mut bmp), image::ImageFormat::Bmp).map_err(|e| e.to_string())?;
    clipboard_win::set_clipboard(clipboard_win::formats::Bitmap, &bmp).map_err(|e| e.to_string())
}

#[cfg(not(windows))]
pub fn set_image(_img: &image::DynamicImage) -> Result<(), String> {
    Err("Bilder kopieren gibt es nur unter Windows".into())
}
