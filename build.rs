//! App-Icon: bevorzugt `assets/app.ico`; fehlt die Datei, wird `assets/app.svg` gerendert.
//! - Windows: das Icon wird in die EXE eingebettet (Explorer, Taskleiste, Verknüpfungen).
//! - Alle Systeme: das größte Bild wird als RGBA nach OUT_DIR/icon.rgba geschrieben; daraus setzt
//!   das Programm zur Laufzeit das Fenster-Icon. Ein fehlerhaftes Icon bricht den Build nie ab.

use std::path::{Path, PathBuf};

fn main() {
    for f in ["assets/app.ico", "assets/app.svg", "build.rs", "Cargo.toml"] {
        println!("cargo:rerun-if-changed={f}");
    }
    let out = PathBuf::from(std::env::var("OUT_DIR").unwrap());
    let ico = load_ico();
    let rgba = ico.as_deref().and_then(largest_rgba);
    // Laufzeit-Icon: [Breite u32][Höhe u32][RGBA …] – leer, wenn kein Icon verfügbar ist
    let mut blob = Vec::new();
    if let Some((w, h, px)) = &rgba {
        blob.extend_from_slice(&w.to_le_bytes());
        blob.extend_from_slice(&h.to_le_bytes());
        blob.extend_from_slice(px);
    } else {
        println!("cargo:warning=Kein gültiges App-Icon (assets/app.ico bzw. assets/app.svg) – Standard-Icon wird verwendet");
    }
    std::fs::write(out.join("icon.rgba"), blob).unwrap();

    // Android: C++-Laufzeit statisch einbinden (die Audio-Ausgabe über oboe ist C++)
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("android") {
        println!("cargo:rustc-link-lib=c++_static");
        println!("cargo:rustc-link-lib=c++abi");
    }
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("windows") {
        if let Some(ico) = ico {
            let path = out.join("app.ico");
            std::fs::write(&path, ico).unwrap();
            let rc = out.join("app.rc");
            let manifest = out.join("app.manifest");
            std::fs::write(&manifest, MANIFEST).unwrap();
            // Pfade im .rc mit Schrägstrichen, damit keine Escape-Sequenzen entstehen
            let slash = |p: &Path| p.display().to_string().replace('\\', "/");
            let mut text = format!("1 ICON \"{}\"\n1 24 \"{}\"\n", slash(&path), slash(&manifest));
            text.push_str(&version_info());
            std::fs::write(&rc, text).unwrap();
            if let Err(e) = embed_resource::compile(&rc, embed_resource::NONE).manifest_optional() {
                println!("cargo:warning=EXE-Icon nicht eingebettet: {e}");
            }
        }
    }
}

/// Anwendungsmanifest: läuft ohne Adminrechte (asInvoker), kennt Windows 10/11, lange Pfade und die
/// modernen Steuerelemente. Eine EXE mit vollständigen Angaben (Manifest + Versionsinfo) stufen
/// Virenscanner deutlich seltener als verdächtig ein als eine „nackte“ EXE.
const MANIFEST: &str = r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<assembly xmlns="urn:schemas-microsoft-com:asm.v1" manifestVersion="1.0">
  <assemblyIdentity type="win32" name="Foldex.Foldex" version="1.0.0.0" processorArchitecture="*"/>
  <description>Foldex Datei-Explorer</description>
  <trustInfo xmlns="urn:schemas-microsoft-com:asm.v3">
    <security>
      <requestedPrivileges>
        <requestedExecutionLevel level="asInvoker" uiAccess="false"/>
      </requestedPrivileges>
    </security>
  </trustInfo>
  <compatibility xmlns="urn:schemas-microsoft-com:compatibility.v1">
    <application>
      <supportedOS Id="{8e0f7a12-bfb3-4fe8-b9a5-48fd50a15a9a}"/>
    </application>
  </compatibility>
  <application xmlns="urn:schemas-microsoft-com:asm.v3">
    <windowsSettings>
      <longPathAware xmlns="http://schemas.microsoft.com/SMI/2016/WindowsSettings">true</longPathAware>
    </windowsSettings>
  </application>
  <dependency>
    <dependentAssembly>
      <assemblyIdentity type="win32" name="Microsoft.Windows.Common-Controls" version="6.0.0.0" processorArchitecture="*" publicKeyToken="6595b64144ccf1df" language="*"/>
    </dependentAssembly>
  </dependency>
</assembly>
"#;

/// VERSIONINFO-Block (Eigenschaften → Details im Explorer): Produkt, Beschreibung, Version aus Cargo.toml.
fn version_info() -> String {
    let v = std::env::var("CARGO_PKG_VERSION").unwrap_or_else(|_| "0.0.0".into());
    let mut n: Vec<u16> = v.split(['.', '-', '+']).take(3).map(|x| x.parse().unwrap_or(0)).collect();
    n.resize(4, 0);
    let num = format!("{},{},{},{}", n[0], n[1], n[2], n[3]);
    let dotted = format!("{}.{}.{}.{}", n[0], n[1], n[2], n[3]);
    format!(
        r#"
1 VERSIONINFO
FILEVERSION {num}
PRODUCTVERSION {num}
FILEOS 0x40004
FILETYPE 0x1
BEGIN
  BLOCK "StringFileInfo"
  BEGIN
    BLOCK "040704B0"
    BEGIN
      VALUE "CompanyName", "Foldex"
      VALUE "FileDescription", "Foldex Datei-Explorer"
      VALUE "FileVersion", "{dotted}"
      VALUE "InternalName", "foldex"
      VALUE "LegalCopyright", "Foldex"
      VALUE "OriginalFilename", "foldex.exe"
      VALUE "ProductName", "Foldex"
      VALUE "ProductVersion", "{v}"
    END
  END
  BLOCK "VarFileInfo"
  BEGIN
    VALUE "Translation", 0x0407, 1200
  END
END
"#
    )
}

fn load_ico() -> Option<Vec<u8>> {
    let ico = Path::new("assets/app.ico");
    if ico.is_file() {
        match std::fs::read(ico) {
            Ok(b) if b.len() > 6 && b[..4] == [0, 0, 1, 0] => return Some(b),
            _ => println!("cargo:warning=assets/app.ico ist keine gültige ICO-Datei – versuche assets/app.svg"),
        }
    }
    let svg = std::fs::read("assets/app.svg").ok()?;
    match render_ico(&svg) {
        Some(b) => Some(b),
        None => {
            println!("cargo:warning=assets/app.svg konnte nicht gerendert werden");
            None
        }
    }
}

/// ICO-Datei mit PNG-kodierten Einträgen (seit Windows Vista unterstützt).
fn render_ico(svg: &[u8]) -> Option<Vec<u8>> {
    use resvg::{tiny_skia, usvg};
    let tree = usvg::Tree::from_data(svg, &usvg::Options::default()).ok()?;
    let sizes = [16u32, 24, 32, 48, 64, 128, 256];
    let mut pngs = Vec::new();
    for &px in &sizes {
        let mut pm = tiny_skia::Pixmap::new(px, px)?;
        let sz = tree.size();
        let s = px as f32 / sz.width().max(sz.height()).max(1.0);
        let (tx, ty) = ((px as f32 - sz.width() * s) / 2.0, (px as f32 - sz.height() * s) / 2.0);
        resvg::render(&tree, tiny_skia::Transform::from_scale(s, s).post_translate(tx, ty), &mut pm.as_mut());
        pngs.push(pm.encode_png().ok()?);
    }
    let mut f = vec![0, 0, 1, 0];
    f.extend_from_slice(&(sizes.len() as u16).to_le_bytes());
    let mut offset = 6 + 16 * sizes.len() as u32;
    for (px, png) in sizes.iter().zip(&pngs) {
        let b = if *px >= 256 { 0 } else { *px as u8 };
        f.extend_from_slice(&[b, b, 0, 0, 1, 0, 32, 0]);
        f.extend_from_slice(&(png.len() as u32).to_le_bytes());
        f.extend_from_slice(&offset.to_le_bytes());
        offset += png.len() as u32;
    }
    for png in &pngs {
        f.extend_from_slice(png);
    }
    Some(f)
}

/// Größtes Bild einer ICO-Datei als RGBA (nicht vormultipliziert). Unterstützt PNG- und
/// 32-/24-Bit-BMP-Einträge – das deckt alle üblichen Icon-Editoren ab.
fn largest_rgba(ico: &[u8]) -> Option<(u32, u32, Vec<u8>)> {
    let rd16 = |o: usize| ico.get(o..o + 2).map(|b| u16::from_le_bytes([b[0], b[1]]));
    let rd32 = |o: usize| ico.get(o..o + 4).map(|b| u32::from_le_bytes([b[0], b[1], b[2], b[3]]));
    let n = rd16(4)? as usize;
    let mut best: Option<(u32, usize, usize)> = None;
    for i in 0..n {
        let e = 6 + 16 * i;
        let w = match *ico.get(e)? {
            0 => 256,
            v => v as u32,
        };
        let (size, off) = (rd32(e + 8)? as usize, rd32(e + 12)? as usize);
        if off + size <= ico.len() && best.is_none_or(|b| w > b.0) {
            best = Some((w, off, size));
        }
    }
    let (_, off, size) = best?;
    let data = &ico[off..off + size];
    if data.starts_with(b"\x89PNG") {
        let pm = resvg::tiny_skia::Pixmap::decode_png(data).ok()?;
        let px = pm.pixels().iter().flat_map(|p| {
            let c = p.demultiply();
            [c.red(), c.green(), c.blue(), c.alpha()]
        });
        return Some((pm.width(), pm.height(), px.collect()));
    }
    // BITMAPINFOHEADER: Höhe ist doppelt (Farbbild + AND-Maske), Zeilen von unten nach oben
    let rd = |o: usize| data.get(o..o + 4).map(|b| i32::from_le_bytes([b[0], b[1], b[2], b[3]]));
    let w = rd(4)?.unsigned_abs();
    let h = rd(8)?.unsigned_abs() / 2;
    let bpp = data.get(14..16).map(|b| u16::from_le_bytes([b[0], b[1]]))?;
    let header = rd(0)? as usize;
    let bytes = (bpp / 8) as usize;
    if !(bpp == 32 || bpp == 24) || w == 0 || h == 0 {
        return None;
    }
    let stride = (w as usize * bytes).div_ceil(4) * 4;
    let mask_stride = (w as usize).div_ceil(32) * 4;
    let mask_at = header + stride * h as usize;
    let mut out = vec![0u8; (w * h * 4) as usize];
    for y in 0..h as usize {
        let src = header + (h as usize - 1 - y) * stride;
        for x in 0..w as usize {
            let p = data.get(src + x * bytes..src + x * bytes + bytes)?;
            let a = if bpp == 32 {
                p[3]
            } else {
                // 24 Bit: Transparenz aus der AND-Maske (1 = durchsichtig)
                let m = data.get(mask_at + (h as usize - 1 - y) * mask_stride + x / 8).copied().unwrap_or(0);
                if m & (0x80 >> (x % 8)) != 0 { 0 } else { 255 }
            };
            let o = (y * w as usize + x) * 4;
            out[o..o + 4].copy_from_slice(&[p[2], p[1], p[0], a]);
        }
    }
    Some((w, h, out))
}
