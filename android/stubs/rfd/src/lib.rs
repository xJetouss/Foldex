//! Ersatz für `rfd` im Android-Build: Android hat keine Datei-Dialoge im Sinne von Windows/Linux.
//! Gleiche Aufrufe wie `rfd::FileDialog`, aber jeder Dialog liefert „abgebrochen“ (None).
use std::path::{Path, PathBuf};

#[derive(Default, Clone)]
pub struct FileDialog;

impl FileDialog {
    pub fn new() -> Self {
        FileDialog
    }
    pub fn add_filter(self, _name: impl Into<String>, _ext: &[impl ToString]) -> Self {
        self
    }
    pub fn set_directory<P: AsRef<Path>>(self, _p: P) -> Self {
        self
    }
    pub fn set_file_name(self, _n: impl Into<String>) -> Self {
        self
    }
    pub fn set_title(self, _t: impl Into<String>) -> Self {
        self
    }
    pub fn pick_file(self) -> Option<PathBuf> {
        None
    }
    pub fn pick_files(self) -> Option<Vec<PathBuf>> {
        None
    }
    pub fn pick_folder(self) -> Option<PathBuf> {
        None
    }
    pub fn save_file(self) -> Option<PathBuf> {
        None
    }
}
