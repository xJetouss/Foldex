//! Android-Anbindung (nur im APK-Build): Bildschirmtastatur, Links öffnen, Zugriff auf alle Dateien.
//! Spricht über JNI mit Android – die App läuft als NativeActivity ohne eigenen Java-Code.

use jni::objects::{JObject, JValue};
use std::sync::{
    atomic::{AtomicBool, Ordering::Relaxed},
    OnceLock,
};
use winit::platform::android::activity::AndroidApp;

static APP: OnceLock<AndroidApp> = OnceLock::new();
static KEYBOARD: AtomicBool = AtomicBool::new(false);
static ASKED: AtomicBool = AtomicBool::new(false);

pub fn init(app: AndroidApp) {
    let _ = APP.set(app);
}

/// Jedes Bild: Tastatur zeigen, solange ein Textfeld den Fokus hat; beim ersten Bild einmal nach dem
/// Zugriff auf alle Dateien fragen (ohne ihn sieht Foldex nur die eigenen App-Ordner).
static GRANTED: AtomicBool = AtomicBool::new(false);

/// Rückgabe `true`: der Zugriff auf alle Dateien wurde gerade erteilt (Ordner neu einlesen)
pub fn frame(ctx: &eframe::egui::Context) -> bool {
    let want = ctx.wants_keyboard_input();
    if want != KEYBOARD.swap(want, Relaxed) {
        if let Some(app) = APP.get() {
            if want {
                app.show_soft_input(true);
            } else {
                app.hide_soft_input(true);
            }
        }
    }
    if !ASKED.swap(true, Relaxed) {
        if all_files_access() {
            GRANTED.store(true, Relaxed);
        } else {
            let _ = request_all_files_access();
        }
    }
    // bis der Zugriff da ist, gelegentlich nachsehen (der Nutzer kommt aus den Einstellungen zurück)
    if !GRANTED.load(Relaxed) {
        ctx.request_repaint_after(std::time::Duration::from_millis(1500));
        if ctx.cumulative_pass_nr() % 30 == 0 && all_files_access() {
            GRANTED.store(true, Relaxed);
            return true;
        }
    }
    false
}

fn with_env<R>(f: impl FnOnce(&mut jni::JNIEnv, &JObject) -> jni::errors::Result<R>) -> Result<R, String> {
    let ctx = ndk_context::android_context();
    let vm = unsafe { jni::JavaVM::from_raw(ctx.vm().cast()) }.map_err(|e| e.to_string())?;
    let mut env = vm.attach_current_thread().map_err(|e| e.to_string())?;
    // die Activity (moveTaskToBack, startActivity …); ndk_context liefert nur die Application
    let raw = APP.get().map(|a| a.activity_as_ptr()).unwrap_or(ctx.context());
    let activity = unsafe { JObject::from_raw(raw.cast()) };
    let r = f(&mut env, &activity);
    // Java-Ausnahmen nicht stehen lassen (sonst stürzt der nächste JNI-Aufruf ab)
    if env.exception_check().unwrap_or(false) {
        let _ = env.exception_describe();
        let _ = env.exception_clear();
        return Err("Android hat die Aktion abgelehnt".into());
    }
    r.map_err(|e| e.to_string())
}

/// Intent mit Aktion und Adresse starten (neue Aufgabe)
fn start(action: &str, uri: &str) -> Result<(), String> {
    with_env(|env, activity| {
        let juri = env.new_string(uri)?;
        let uri = env.call_static_method("android/net/Uri", "parse", "(Ljava/lang/String;)Landroid/net/Uri;", &[JValue::Object(&juri)])?.l()?;
        let jaction = env.new_string(action)?;
        let intent = env.new_object("android/content/Intent", "(Ljava/lang/String;Landroid/net/Uri;)V", &[JValue::Object(&jaction), JValue::Object(&uri)])?;
        env.call_method(&intent, "addFlags", "(I)Landroid/content/Intent;", &[JValue::Int(0x1000_0000)])?; // FLAG_ACTIVITY_NEW_TASK
        env.call_method(activity, "startActivity", "(Landroid/content/Intent;)V", &[JValue::Object(&intent)])?;
        Ok(())
    })
}

/// Adresse im Browser öffnen (z. B. die neueste APK zum Aktualisieren)
pub fn open_url(url: &str) -> Result<(), String> {
    start("android.intent.action.VIEW", url)
}

/// Hat Foldex Zugriff auf alle Dateien (Android 11+: „Alle Dateien verwalten“)?
pub fn all_files_access() -> bool {
    with_env(|env, _| env.call_static_method("android/os/Environment", "isExternalStorageManager", "()Z", &[])?.z()).unwrap_or(true)
}

/// Systemseite „Zugriff auf alle Dateien“ für Foldex öffnen
pub fn request_all_files_access() -> Result<(), String> {
    start("android.settings.MANAGE_APP_ALL_FILES_ACCESS_PERMISSION", "package:dev.tssnt.foldex")
}

/// App in den Hintergrund (wie „Zurück“ auf der obersten Ebene anderer Apps)
pub fn to_background() {
    let _ = with_env(|env, activity| env.call_method(activity, "moveTaskToBack", "(Z)Z", &[JValue::Bool(1)])?.z());
}

/// Meldung ins Android-Log (adb logcat -s Foldex)
pub fn log(msg: &str) {
    let _ = with_env(|env, _| {
        let tag = env.new_string("Foldex")?;
        let m = env.new_string(msg)?;
        env.call_static_method("android/util/Log", "i", "(Ljava/lang/String;Ljava/lang/String;)I", &[JValue::Object(&tag), JValue::Object(&m)])?;
        Ok(())
    });
}
