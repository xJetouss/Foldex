//! Ersatz für `trash` im Android-Build: Android hat keinen System-Papierkorb für Apps.
//! „In den Papierkorb“ meldet einen Fehler (Foldex fragt dann nach endgültigem Löschen), die Liste ist leer.
use std::{ffi::OsString, fmt, path::{Path, PathBuf}};

#[derive(Debug)]
pub enum Error {
    Unsupported,
    RestoreCollision { path: PathBuf, remaining_items: Vec<TrashItem> },
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("Auf Android gibt es keinen Papierkorb")
    }
}

impl std::error::Error for Error {}

#[derive(Debug, Clone)]
pub struct TrashItem {
    pub id: OsString,
    pub name: OsString,
    pub original_parent: PathBuf,
    pub time_deleted: i64,
}

impl TrashItem {
    pub fn original_path(&self) -> PathBuf {
        self.original_parent.join(&self.name)
    }
}

pub enum TrashItemSize {
    Bytes(u64),
    Entries(usize),
}

pub struct TrashItemMetadata {
    pub size: TrashItemSize,
}

pub fn delete<T: AsRef<Path>>(_path: T) -> Result<(), Error> {
    Err(Error::Unsupported)
}

pub fn delete_all<I, T>(_paths: I) -> Result<(), Error>
where
    I: IntoIterator<Item = T>,
    T: AsRef<Path>,
{
    Err(Error::Unsupported)
}

pub mod os_limited {
    use super::*;

    pub fn list() -> Result<Vec<TrashItem>, Error> {
        Ok(Vec::new())
    }
    pub fn metadata(_item: &TrashItem) -> Result<TrashItemMetadata, Error> {
        Err(Error::Unsupported)
    }
    pub fn restore_all<I: IntoIterator<Item = TrashItem>>(_items: I) -> Result<(), Error> {
        Err(Error::Unsupported)
    }
    pub fn purge_all<I: IntoIterator<Item = TrashItem>>(_items: I) -> Result<(), Error> {
        Err(Error::Unsupported)
    }
}
