//! „Mit Foldex öffnen“ im neuen Kontextmenü von Windows 11.
//!
//! Windows 11 zeigt oben im Rechtsklick-Menü nur Befehle von Apps mit Paket-Identität, die einen
//! `IExplorerCommand` als COM-Objekt anbieten. Diese DLL ist dieses COM-Objekt; angemeldet wird sie über
//! ein kleines Paket (siehe `AppxManifest.xml`), dessen „externer Ort“ der Installationsordner von Foldex ist.
//! Die DLL liegt dort neben `foldex.exe` und startet beim Klick `foldex.exe --open "<Ordner>"`.

#![allow(non_snake_case)]

use std::{
    ffi::c_void,
    os::windows::ffi::OsStringExt,
    path::PathBuf,
    sync::atomic::{AtomicPtr, Ordering},
};
use windows::{
    core::{implement, Interface, Ref, Result, BOOL, GUID, HRESULT, PCWSTR, PWSTR},
    Win32::{
        Foundation::{CLASS_E_CLASSNOTAVAILABLE, CLASS_E_NOAGGREGATION, E_NOTIMPL, E_POINTER, HMODULE, S_FALSE},
        Globalization::GetUserDefaultUILanguage,
        System::{
            Com::{CoTaskMemFree, IBindCtx, IClassFactory, IClassFactory_Impl},
            LibraryLoader::GetModuleFileNameW,
            SystemServices::DLL_PROCESS_ATTACH,
        },
        UI::Shell::{
            IEnumExplorerCommand, IExplorerCommand, IExplorerCommand_Impl, IShellItemArray, SHStrDupW, ECF_DEFAULT,
            ECS_ENABLED, ECS_HIDDEN, SIGDN_FILESYSPATH,
        },
    },
};

/// Klassen-ID des Befehls (steht auch in `AppxManifest.xml`)
pub const CLSID_OPEN: GUID = GUID::from_u128(0x7b3e2a64_5c1d_4f8e_9a2b_f01d3e6c9a57);

static MODULE: AtomicPtr<c_void> = AtomicPtr::new(std::ptr::null_mut());

#[no_mangle]
extern "system" fn DllMain(module: HMODULE, reason: u32, _: *mut c_void) -> BOOL {
    if reason == DLL_PROCESS_ATTACH {
        MODULE.store(module.0, Ordering::Relaxed);
    }
    true.into()
}

/// Ordner dieser DLL – dort liegt auch `foldex.exe`.
fn own_dir() -> Option<PathBuf> {
    let mut buf = [0u16; 1024];
    let n = unsafe { GetModuleFileNameW(Some(HMODULE(MODULE.load(Ordering::Relaxed))), &mut buf) } as usize;
    if n == 0 || n >= buf.len() {
        return None;
    }
    PathBuf::from(std::ffi::OsString::from_wide(&buf[..n])).parent().map(PathBuf::from)
}

fn foldex_exe() -> Option<PathBuf> {
    own_dir().map(|d| d.join("foldex.exe")).filter(|p| p.is_file())
}

fn german() -> bool {
    (unsafe { GetUserDefaultUILanguage() } & 0x3ff) == 0x07
}

/// Text als von COM freizugebender String (Explorer gibt ihn mit CoTaskMemFree frei).
fn dup(s: &str) -> Result<PWSTR> {
    let w: Vec<u16> = s.encode_utf16().chain(std::iter::once(0)).collect();
    unsafe { SHStrDupW(PCWSTR(w.as_ptr())) }
}

/// Dateisystem-Pfade der ausgewählten Einträge (Ordner, Laufwerk oder – beim Hintergrund – der offene Ordner).
fn paths(items: &IShellItemArray) -> Vec<PathBuf> {
    let mut v = Vec::new();
    let n = unsafe { items.GetCount() }.unwrap_or(0);
    for i in 0..n {
        let Ok(item) = (unsafe { items.GetItemAt(i) }) else { continue };
        let Ok(p) = (unsafe { item.GetDisplayName(SIGDN_FILESYSPATH) }) else { continue };
        let s = unsafe { p.to_string() };
        unsafe { CoTaskMemFree(Some(p.0 as *const c_void)) };
        if let Ok(s) = s {
            v.push(PathBuf::from(s));
        }
    }
    v
}

#[implement(IExplorerCommand)]
struct OpenCommand;

impl IExplorerCommand_Impl for OpenCommand_Impl {
    fn GetTitle(&self, _: Ref<IShellItemArray>) -> Result<PWSTR> {
        dup(if german() { "Mit Foldex öffnen" } else { "Open with Foldex" })
    }

    fn GetIcon(&self, _: Ref<IShellItemArray>) -> Result<PWSTR> {
        match foldex_exe() {
            Some(exe) => dup(&format!("{},0", exe.display())),
            None => Err(E_NOTIMPL.into()),
        }
    }

    fn GetToolTip(&self, _: Ref<IShellItemArray>) -> Result<PWSTR> {
        Err(E_NOTIMPL.into())
    }

    fn GetCanonicalName(&self) -> Result<GUID> {
        Ok(CLSID_OPEN)
    }

    fn GetState(&self, items: Ref<IShellItemArray>, _ok_to_be_slow: BOOL) -> Result<u32> {
        // nur bei echten Ordnern/Laufwerken anbieten (nicht z. B. bei „Dieser PC“)
        let ok = foldex_exe().is_some() && items.as_ref().map_or(true, |a| paths(a).iter().any(|p| p.is_dir()));
        Ok(if ok { ECS_ENABLED.0 as u32 } else { ECS_HIDDEN.0 as u32 })
    }

    fn Invoke(&self, items: Ref<IShellItemArray>, _: Ref<IBindCtx>) -> Result<()> {
        let Some(exe) = foldex_exe() else { return Err(E_NOTIMPL.into()) };
        let dirs: Vec<PathBuf> = items.as_ref().map(paths).unwrap_or_default().into_iter().filter(|p| p.is_dir()).collect();
        if dirs.is_empty() {
            return Ok(());
        }
        let mut cmd = std::process::Command::new(&exe);
        if let Some(d) = exe.parent() {
            cmd.current_dir(d);
        }
        for d in dirs.iter().take(12) {
            cmd.arg("--open").arg(d);
        }
        cmd.spawn().map(|_| ()).map_err(|_| E_NOTIMPL.into())
    }

    fn GetFlags(&self) -> Result<u32> {
        Ok(ECF_DEFAULT.0 as u32)
    }

    fn EnumSubCommands(&self) -> Result<IEnumExplorerCommand> {
        Err(E_NOTIMPL.into())
    }
}

#[implement(IClassFactory)]
struct Factory;

impl IClassFactory_Impl for Factory_Impl {
    fn CreateInstance(&self, outer: Ref<windows::core::IUnknown>, riid: *const GUID, ppv: *mut *mut c_void) -> Result<()> {
        if ppv.is_null() {
            return Err(E_POINTER.into());
        }
        unsafe { *ppv = std::ptr::null_mut() };
        if outer.is_some() {
            return Err(CLASS_E_NOAGGREGATION.into());
        }
        let cmd: IExplorerCommand = OpenCommand.into();
        unsafe { cmd.query(riid, ppv).ok() }
    }

    fn LockServer(&self, _: BOOL) -> Result<()> {
        Ok(())
    }
}

#[no_mangle]
extern "system" fn DllGetClassObject(clsid: *const GUID, riid: *const GUID, ppv: *mut *mut c_void) -> HRESULT {
    if ppv.is_null() || clsid.is_null() {
        return E_POINTER;
    }
    unsafe { *ppv = std::ptr::null_mut() };
    if unsafe { *clsid } != CLSID_OPEN {
        return CLASS_E_CLASSNOTAVAILABLE;
    }
    let f: IClassFactory = Factory.into();
    unsafe { f.query(riid, ppv) }
}

#[no_mangle]
extern "system" fn DllCanUnloadNow() -> HRESULT {
    S_FALSE
}
