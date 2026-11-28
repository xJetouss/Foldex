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

/// Höhe von Statusleiste (oben) und Gesten-/Navigationsleiste (unten) in Pixeln. Die App zeichnet bis an den
/// Bildschirmrand (Statusleiste in App-Farbe statt schwarzem Balken); diese Bereiche bleiben frei.
pub fn bar_insets() -> (f32, f32) {
    static INSETS: OnceLock<(f32, f32)> = OnceLock::new();
    *INSETS.get_or_init(|| {
        let dimen = |name: &str| -> f32 {
            with_env(|env, activity| {
                let res = env.call_method(activity, "getResources", "()Landroid/content/res/Resources;", &[])?.l()?;
                let n = env.new_string(name)?;
                let t = env.new_string("dimen")?;
                let pkg = env.new_string("android")?;
                let id = env.call_method(&res, "getIdentifier", "(Ljava/lang/String;Ljava/lang/String;Ljava/lang/String;)I", &[JValue::Object(&n), JValue::Object(&t), JValue::Object(&pkg)])?.i()?;
                if id == 0 {
                    return Ok(0.0);
                }
                Ok(env.call_method(&res, "getDimensionPixelSize", "(I)I", &[JValue::Int(id)])?.i()? as f32)
            })
            .unwrap_or(0.0)
        };
        (dimen("status_bar_height"), dimen("navigation_bar_height"))
    })
}

/// Ist eine App installiert (und startbar)? Die Pakete stehen im Manifest unter `<queries>`.
pub fn installed(pkg: &str) -> bool {
    with_env(|env, activity| {
        let pm = env.call_method(activity, "getPackageManager", "()Landroid/content/pm/PackageManager;", &[])?.l()?;
        let p = env.new_string(pkg)?;
        let i = env.call_method(&pm, "getLaunchIntentForPackage", "(Ljava/lang/String;)Landroid/content/Intent;", &[JValue::Object(&p)])?.l()?;
        Ok(!i.is_null())
    })
    .unwrap_or(false)
}

/// Symbol einer installierten App als RGBA (Kantenlänge `px`).
pub fn app_icon(pkg: &str, px: i32) -> Option<(usize, Vec<u8>)> {
    with_env(|env, activity| {
        let pm = env.call_method(activity, "getPackageManager", "()Landroid/content/pm/PackageManager;", &[])?.l()?;
        let p = env.new_string(pkg)?;
        let d = env.call_method(&pm, "getApplicationIcon", "(Ljava/lang/String;)Landroid/graphics/drawable/Drawable;", &[JValue::Object(&p)])?.l()?;
        let cfg = env.get_static_field("android/graphics/Bitmap$Config", "ARGB_8888", "Landroid/graphics/Bitmap$Config;")?.l()?;
        let bmp = env.call_static_method("android/graphics/Bitmap", "createBitmap", "(IILandroid/graphics/Bitmap$Config;)Landroid/graphics/Bitmap;", &[JValue::Int(px), JValue::Int(px), JValue::Object(&cfg)])?.l()?;
        let canvas = env.new_object("android/graphics/Canvas", "(Landroid/graphics/Bitmap;)V", &[JValue::Object(&bmp)])?;
        env.call_method(&d, "setBounds", "(IIII)V", &[JValue::Int(0), JValue::Int(0), JValue::Int(px), JValue::Int(px)])?;
        env.call_method(&d, "draw", "(Landroid/graphics/Canvas;)V", &[JValue::Object(&canvas)])?;
        let n = (px * px) as usize;
        let arr = env.new_int_array(px * px)?;
        env.call_method(&bmp, "getPixels", "([IIIIIII)V", &[JValue::Object(&arr), JValue::Int(0), JValue::Int(px), JValue::Int(0), JValue::Int(0), JValue::Int(px), JValue::Int(px)])?;
        let mut buf = vec![0i32; n];
        env.get_int_array_region(&arr, 0, &mut buf)?;
        // ARGB (nicht vormultipliziert) → RGBA
        let rgba = buf.iter().flat_map(|&c| {
            let c = c as u32;
            [(c >> 16) as u8, (c >> 8) as u8, c as u8, (c >> 24) as u8]
        });
        Ok((px as usize, rgba.collect()))
    })
    .ok()
}

/// Adresse einer Datei im MediaStore (Bilder/Videos/Audio typisiert, sonst „files“); `None`, wenn er sie nicht kennt.
fn media_uri<'a>(env: &mut jni::JNIEnv<'a>, activity: &JObject, path: &str, mime: &str) -> jni::errors::Result<Option<JObject<'a>>> {
    let vol = env.new_string("external")?;
    let typed = match mime.split('/').next().unwrap_or("") {
        "image" => "android/provider/MediaStore$Images$Media",
        "video" => "android/provider/MediaStore$Video$Media",
        "audio" => "android/provider/MediaStore$Audio$Media",
        _ => "android/provider/MediaStore$Files",
    };
    let base = env.call_static_method(typed, "getContentUri", "(Ljava/lang/String;)Landroid/net/Uri;", &[JValue::Object(&vol)])?.l()?;
    let cr = env.call_method(activity, "getContentResolver", "()Landroid/content/ContentResolver;", &[])?.l()?;
    let id_col = env.new_string("_id")?;
    let proj = env.new_object_array(1, "java/lang/String", &id_col)?;
    let sel = env.new_string("_data=?")?;
    let p = env.new_string(path)?;
    let args = env.new_object_array(1, "java/lang/String", &p)?;
    let null = JObject::null();
    let cur = env
        .call_method(&cr, "query", "(Landroid/net/Uri;[Ljava/lang/String;Ljava/lang/String;[Ljava/lang/String;Ljava/lang/String;)Landroid/database/Cursor;", &[JValue::Object(&base), JValue::Object(&proj), JValue::Object(&sel), JValue::Object(&args), JValue::Object(&null)])
        .and_then(|v| v.l());
    if env.exception_check().unwrap_or(false) {
        let _ = env.exception_clear();
    }
    if let Ok(cur) = cur {
        if !cur.is_null() {
            let found = env.call_method(&cur, "moveToFirst", "()Z", &[])?.z()?;
            let id = if found { Some(env.call_method(&cur, "getLong", "(I)J", &[JValue::Int(0)])?.j()?) } else { None };
            env.call_method(&cur, "close", "()V", &[])?;
            if let Some(id) = id {
                return Ok(Some(env.call_static_method("android/content/ContentUris", "withAppendedId", "(Landroid/net/Uri;J)Landroid/net/Uri;", &[JValue::Object(&base), JValue::Long(id)])?.l()?));
            }
        }
    }
    Ok(None)
}

/// Adresse einer Datei für andere Apps: aus dem MediaStore; kennt er sie nicht, wird sie erst eingelesen
/// (MediaScanner, bis 2,5 s), sonst `file://` (dafür ist die StrictMode-Sperre aufgehoben).
fn content_uris<'a>(env: &mut jni::JNIEnv<'a>, activity: &JObject, paths: &[String], mime: &str) -> jni::errors::Result<Vec<JObject<'a>>> {
    let mut out: Vec<Option<JObject<'a>>> = Vec::new();
    for p in paths {
        out.push(media_uri(env, activity, p, mime)?);
    }
    let missing: Vec<&String> = paths.iter().zip(&out).filter(|(_, u)| u.is_none()).map(|(p, _)| p).collect();
    if !missing.is_empty() {
        let first = env.new_string(missing[0])?;
        let arr = env.new_object_array(missing.len() as i32, "java/lang/String", &first)?;
        for (i, m) in missing.iter().enumerate().skip(1) {
            let js = env.new_string(m)?;
            env.set_object_array_element(&arr, i as i32, &js)?;
        }
        let null = JObject::null();
        env.call_static_method("android/media/MediaScannerConnection", "scanFile", "(Landroid/content/Context;[Ljava/lang/String;[Ljava/lang/String;Landroid/media/MediaScannerConnection$OnScanCompletedListener;)V", &[JValue::Object(activity), JValue::Object(&arr), JValue::Object(&null), JValue::Object(&null)])?;
        for _ in 0..25 {
            if out.iter().all(|u| u.is_some()) {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(100));
            for (k, p) in paths.iter().enumerate() {
                if out[k].is_none() {
                    out[k] = media_uri(env, activity, p, mime)?;
                }
            }
        }
    }
    let mut v = Vec::new();
    for (k, u) in out.into_iter().enumerate() {
        v.push(match u {
            Some(u) => u,
            None => {
                let p = env.new_string(&paths[k])?;
                let f = env.new_object("java/io/File", "(Ljava/lang/String;)V", &[JValue::Object(&p)])?;
                env.call_static_method("android/net/Uri", "fromFile", "(Ljava/io/File;)Landroid/net/Uri;", &[JValue::Object(&f)])?.l()?
            }
        });
    }
    Ok(v)
}

/// Dateien (und optional Text) teilen: mit `package` direkt an diese App, sonst über das Teilen-Menü von Android.
pub fn share(paths: &[std::path::PathBuf], mime: &str, text: Option<&str>, package: Option<&str>) -> Result<(), String> {
    with_env(|env, activity| {
        // file://-Adressen erlauben (Dateien, die der MediaStore nicht kennt)
        let b = env.new_object("android/os/StrictMode$VmPolicy$Builder", "()V", &[])?;
        let pol = env.call_method(&b, "build", "()Landroid/os/StrictMode$VmPolicy;", &[])?.l()?;
        env.call_static_method("android/os/StrictMode", "setVmPolicy", "(Landroid/os/StrictMode$VmPolicy;)V", &[JValue::Object(&pol)])?;
        let list = env.new_object("java/util/ArrayList", "()V", &[])?;
        let mut clip: Option<JObject> = None;
        let strs: Vec<String> = paths.iter().map(|p| p.to_string_lossy().into_owned()).collect();
        for uri in content_uris(env, activity, &strs, mime)? {
            env.call_method(&list, "add", "(Ljava/lang/Object;)Z", &[JValue::Object(&uri)])?;
            match &clip {
                None => {
                    let l = env.new_string("")?;
                    clip = Some(env.call_static_method("android/content/ClipData", "newRawUri", "(Ljava/lang/CharSequence;Landroid/net/Uri;)Landroid/content/ClipData;", &[JValue::Object(&l), JValue::Object(&uri)])?.l()?);
                }
                Some(c) => {
                    let item = env.new_object("android/content/ClipData$Item", "(Landroid/net/Uri;)V", &[JValue::Object(&uri)])?;
                    env.call_method(c, "addItem", "(Landroid/content/ClipData$Item;)V", &[JValue::Object(&item)])?;
                }
            }
        }
        let action = env.new_string(if paths.len() > 1 { "android.intent.action.SEND_MULTIPLE" } else { "android.intent.action.SEND" })?;
        let intent = env.new_object("android/content/Intent", "(Ljava/lang/String;)V", &[JValue::Object(&action)])?;
        let jm = env.new_string(if paths.is_empty() { "text/plain" } else { mime })?;
        env.call_method(&intent, "setType", "(Ljava/lang/String;)Landroid/content/Intent;", &[JValue::Object(&jm)])?;
        let key = env.new_string("android.intent.extra.STREAM")?;
        if paths.len() == 1 {
            let uri = env.call_method(&list, "get", "(I)Ljava/lang/Object;", &[JValue::Int(0)])?.l()?;
            env.call_method(&intent, "putExtra", "(Ljava/lang/String;Landroid/os/Parcelable;)Landroid/content/Intent;", &[JValue::Object(&key), JValue::Object(&uri)])?;
        } else if paths.len() > 1 {
            env.call_method(&intent, "putParcelableArrayListExtra", "(Ljava/lang/String;Ljava/util/ArrayList;)Landroid/content/Intent;", &[JValue::Object(&key), JValue::Object(&list)])?;
        }
        if let Some(t) = text {
            let k = env.new_string("android.intent.extra.TEXT")?;
            let v = env.new_string(t)?;
            env.call_method(&intent, "putExtra", "(Ljava/lang/String;Ljava/lang/String;)Landroid/content/Intent;", &[JValue::Object(&k), JValue::Object(&v)])?;
        }
        if let Some(c) = clip {
            env.call_method(&intent, "setClipData", "(Landroid/content/ClipData;)V", &[JValue::Object(&c)])?;
        }
        env.call_method(&intent, "addFlags", "(I)Landroid/content/Intent;", &[JValue::Int(1)])?; // FLAG_GRANT_READ_URI_PERMISSION
        match package {
            Some(pkg) => {
                let jp = env.new_string(pkg)?;
                env.call_method(&intent, "setPackage", "(Ljava/lang/String;)Landroid/content/Intent;", &[JValue::Object(&jp)])?;
                env.call_method(activity, "startActivity", "(Landroid/content/Intent;)V", &[JValue::Object(&intent)])?;
            }
            None => {
                let title = env.new_string("Teilen")?;
                let chooser = env.call_static_method("android/content/Intent", "createChooser", "(Landroid/content/Intent;Ljava/lang/CharSequence;)Landroid/content/Intent;", &[JValue::Object(&intent), JValue::Object(&title)])?.l()?;
                env.call_method(activity, "startActivity", "(Landroid/content/Intent;)V", &[JValue::Object(&chooser)])?;
            }
        }
        Ok(())
    })
}

/// Dunkelmodus des Systems (Configuration.uiMode)
pub fn night_mode() -> Option<bool> {
    with_env(|env, activity| {
        let res = env.call_method(activity, "getResources", "()Landroid/content/res/Resources;", &[])?.l()?;
        let cfg = env.call_method(&res, "getConfiguration", "()Landroid/content/res/Configuration;", &[])?.l()?;
        let ui = env.get_field(&cfg, "uiMode", "I")?.i()?;
        Ok(ui & 0x30 == 0x20)
    })
    .ok()
}

/// Text in die Android-Zwischenablage (egui kennt auf Android nur eine interne).
pub fn set_clip_text(text: &str) -> Result<(), String> {
    with_env(|env, activity| {
        let svc = env.new_string("clipboard")?;
        let cm = env.call_method(activity, "getSystemService", "(Ljava/lang/String;)Ljava/lang/Object;", &[JValue::Object(&svc)])?.l()?;
        let label = env.new_string("Foldex")?;
        let t = env.new_string(text)?;
        let clip = env.call_static_method("android/content/ClipData", "newPlainText", "(Ljava/lang/CharSequence;Ljava/lang/CharSequence;)Landroid/content/ClipData;", &[JValue::Object(&label), JValue::Object(&t)])?.l()?;
        env.call_method(&cm, "setPrimaryClip", "(Landroid/content/ClipData;)V", &[JValue::Object(&clip)])?;
        Ok(())
    })
}

/// Datei (z. B. ein Bild) in die Android-Zwischenablage – als MediaStore-Adresse, zum Einfügen in andere Apps.
pub fn set_clip_file(path: &std::path::Path, mime: &str) -> Result<(), String> {
    with_env(|env, activity| {
        let uris = content_uris(env, activity, &[path.to_string_lossy().into_owned()], mime)?;
        let Some(uri) = uris.into_iter().next() else { return Ok(()) };
        let svc = env.new_string("clipboard")?;
        let cm = env.call_method(activity, "getSystemService", "(Ljava/lang/String;)Ljava/lang/Object;", &[JValue::Object(&svc)])?.l()?;
        let cr = env.call_method(activity, "getContentResolver", "()Landroid/content/ContentResolver;", &[])?.l()?;
        let label = env.new_string("Foldex")?;
        let clip = env.call_static_method("android/content/ClipData", "newUri", "(Landroid/content/ContentResolver;Ljava/lang/CharSequence;Landroid/net/Uri;)Landroid/content/ClipData;", &[JValue::Object(&cr), JValue::Object(&label), JValue::Object(&uri)])?.l()?;
        env.call_method(&cm, "setPrimaryClip", "(Landroid/content/ClipData;)V", &[JValue::Object(&clip)])?;
        Ok(())
    })
}

/// MIME-Typ zur Endung (Android-Liste), sonst `*/*`.
pub fn mime_of(path: &std::path::Path) -> String {
    let ext = path.extension().map(|e| e.to_string_lossy().to_lowercase()).unwrap_or_default();
    if ext == "apk" {
        return "application/vnd.android.package-archive".into();
    }
    with_env(|env, _| {
        let map = env.call_static_method("android/webkit/MimeTypeMap", "getSingleton", "()Landroid/webkit/MimeTypeMap;", &[])?.l()?;
        let e = env.new_string(&ext)?;
        let m = env.call_method(&map, "getMimeTypeFromExtension", "(Ljava/lang/String;)Ljava/lang/String;", &[JValue::Object(&e)])?.l()?;
        if m.is_null() {
            return Ok("*/*".to_string());
        }
        let js = jni::objects::JString::from(m);
        let s: String = env.get_string(&js)?.into();
        Ok(s)
    })
    .unwrap_or_else(|_| "*/*".into())
}

/// Datei in einer anderen App öffnen (APK: Installation). `chooser` = immer die App-Auswahl zeigen.
pub fn view(path: &std::path::Path, chooser: bool) -> Result<(), String> {
    let mime = mime_of(path);
    with_env(|env, activity| {
        let b = env.new_object("android/os/StrictMode$VmPolicy$Builder", "()V", &[])?;
        let pol = env.call_method(&b, "build", "()Landroid/os/StrictMode$VmPolicy;", &[])?.l()?;
        env.call_static_method("android/os/StrictMode", "setVmPolicy", "(Landroid/os/StrictMode$VmPolicy;)V", &[JValue::Object(&pol)])?;
        let uri = content_uris(env, activity, &[path.to_string_lossy().into_owned()], &mime)?.remove(0);
        let action = env.new_string("android.intent.action.VIEW")?;
        let intent = env.new_object("android/content/Intent", "(Ljava/lang/String;)V", &[JValue::Object(&action)])?;
        let jm = env.new_string(&mime)?;
        env.call_method(&intent, "setDataAndType", "(Landroid/net/Uri;Ljava/lang/String;)Landroid/content/Intent;", &[JValue::Object(&uri), JValue::Object(&jm)])?;
        env.call_method(&intent, "addFlags", "(I)Landroid/content/Intent;", &[JValue::Int(1)])?; // Lesen erlauben
        let start = if chooser {
            let title = env.new_string("Öffnen mit")?;
            env.call_static_method("android/content/Intent", "createChooser", "(Landroid/content/Intent;Ljava/lang/CharSequence;)Landroid/content/Intent;", &[JValue::Object(&intent), JValue::Object(&title)])?.l()?
        } else {
            intent
        };
        env.call_method(activity, "startActivity", "(Landroid/content/Intent;)V", &[JValue::Object(&start)])?;
        Ok(())
    })
}

/// Kurz vibrieren (Rückmeldung bei langem Drücken).
pub fn vibrate(ms: i64) {
    let _ = with_env(|env, activity| {
        let svc = env.new_string("vibrator")?;
        let v = env.call_method(activity, "getSystemService", "(Ljava/lang/String;)Ljava/lang/Object;", &[JValue::Object(&svc)])?.l()?;
        if v.is_null() {
            return Ok(());
        }
        let effect = env.call_static_method("android/os/VibrationEffect", "createOneShot", "(JI)Landroid/os/VibrationEffect;", &[JValue::Long(ms), JValue::Int(-1)])?.l()?;
        env.call_method(&v, "vibrate", "(Landroid/os/VibrationEffect;)V", &[JValue::Object(&effect)])?;
        Ok(())
    });
}
