//! KI-Komponenten für den Installer: Katalog (`foldex --list-ai`) und Installation mit Fortschritt
//! (`foldex --install-ai id,id,…`). Der Installer zeigt die Liste zum Auswählen und liest die Fortschritts-
//! zeilen („PROGRESS <Prozent> <Text>“, am Ende „DONE“ oder „ERROR <Text>“).

use crate::localai::{self, Asset, Need};
use crate::localllm;
use std::time::Duration;

pub struct Comp {
    pub id: String,
    pub group: &'static str,
    pub name: String,
    pub desc: String,
    pub mb: u64,
    pub installed: bool,
    pub recommended: bool,
}

/// Bildeditor-KI: (Kennung, Name, Beschreibung, benötigte Schritte)
const EDITOR: [(&str, &str, &str, Need); 5] = [
    ("bild:freistellen", "Motiv freistellen", "Hintergrund entfernen (ISNet)", Need::Subject),
    ("bild:objekte", "Objekte erkennen", "Objekte per Klick auswählen (Segment Anything)", Need::Objects),
    ("bild:entfernen", "Objekte entfernen", "Störendes aus Fotos entfernen (LaMa HD)", Need::Inpaint),
    ("bild:kolorieren", "Kolorieren", "Schwarzweiß-Fotos einfärben (DeOldify)", Need::Colorize),
    ("bild:hochskalieren", "Hochskalieren", "Bilder schärfer vergrößern (Real-ESRGAN)", Need::Upscale),
];

/// alle wählbaren KI-Komponenten
pub fn catalog() -> Vec<Comp> {
    let mut v = Vec::new();
    for m in localllm::TEXT {
        v.push(Comp {
            id: format!("text:{}", m.file.file),
            group: "KI-Chat (Textmodelle)",
            name: m.name.into(),
            desc: m.desc.into(),
            mb: (m.file.mb + m.vision_mb) as u64 + 40,
            installed: localllm::text_path(m).is_file(),
            recommended: m.name == "Qwen3.5 4B",
        });
    }
    for m in localllm::IMAGE {
        v.push(Comp { id: format!("image:{}", m.id), group: "Bilder erzeugen", name: m.name.into(), desc: m.desc.into(), mb: m.files_mb() as u64 + 30, installed: localllm::image_installed(m), recommended: m.id == "sd-turbo" });
    }
    for m in localllm::EDIT {
        v.push(Comp { id: format!("edit:{}", m.id), group: "Bilder per Anweisung bearbeiten", name: m.name.into(), desc: m.desc.into(), mb: m.total_mb() as u64 + 30, installed: m.installed(), recommended: false });
    }
    for m in localllm::STT {
        v.push(Comp { id: format!("stt:{}", m.name), group: "Spracheingabe", name: m.name.into(), desc: m.desc.into(), mb: m.mb() as u64 + 8, installed: localllm::stt_path(m).is_file(), recommended: m.name == "Whisper small" });
    }
    if localai::supported() {
        for (id, name, desc, need) in EDITOR {
            let all = localai::needed(&[need]);
            v.push(Comp {
                id: id.into(),
                group: "Bildeditor-KI",
                name: name.into(),
                desc: desc.into(),
                mb: all.iter().map(|a| a.mb as u64).sum(),
                installed: localai::missing(&[need]).is_empty(),
                recommended: id == "bild:freistellen",
            });
        }
    }
    v
}

/// Katalog als JSON (eine Zeile)
pub fn catalog_json() -> String {
    let arr: Vec<serde_json::Value> = catalog()
        .into_iter()
        .map(|c| serde_json::json!({ "id": c.id, "group": c.group, "name": c.name, "desc": c.desc, "mb": c.mb, "installed": c.installed, "recommended": c.recommended }))
        .collect();
    serde_json::Value::Array(arr).to_string()
}

/// Komponenten installieren; `report(Prozent, Text)` wird regelmäßig aufgerufen. Blockiert bis zum Ende.
pub fn install(ids: &[String], mut report: impl FnMut(f32, &str)) -> Result<(), String> {
    let mut needs: Vec<Need> = Vec::new();
    // schon Installiertes nicht noch einmal laden
    let have: Vec<String> = catalog().into_iter().filter(|c| c.installed).map(|c| c.id).collect();
    for id in ids.iter().filter(|i| !have.contains(i)) {
        let (kind, key) = id.split_once(':').ok_or_else(|| format!("unbekannt: {id}"))?;
        match kind {
            "text" => localllm::install_text(localllm::TEXT.iter().find(|m| m.file.file == key).ok_or_else(|| format!("unbekanntes Textmodell: {key}"))?),
            "image" => localllm::install_image(localllm::IMAGE.iter().find(|m| m.id == key).ok_or_else(|| format!("unbekanntes Bildmodell: {key}"))?),
            "edit" => localllm::install_edit(localllm::EDIT.iter().find(|m| m.id == key).ok_or_else(|| format!("unbekanntes Bearbeitungsmodell: {key}"))?),
            "stt" => localllm::install_stt(localllm::STT.iter().find(|m| m.name == key).ok_or_else(|| format!("unbekanntes Sprachmodell: {key}"))?),
            "bild" => needs.push(EDITOR.iter().find(|e| e.0 == id).map(|e| e.3).ok_or_else(|| format!("unbekannt: {id}"))?),
            _ => return Err(format!("unbekannt: {id}")),
        }
    }
    let editor: Vec<&'static Asset> = localai::missing(&needs);
    let editor_mb: f32 = editor.iter().map(|a| a.mb as f32).sum();
    if !editor.is_empty() {
        localai::install(editor);
    }
    // Fortschritt: Anzahl Downloads (Modelle/Programme) plus Bildeditor-KI als ein Block
    let total = localllm::downloads().len() as f32 + if editor_mb > 0.0 { 1.0 } else { 0.0 };
    let mut errors: Vec<String> = Vec::new();
    loop {
        let list = localllm::downloads();
        let ai = localai::install_status();
        for (err, msg) in localllm::take_log() {
            if err {
                errors.push(msg);
            }
        }
        if list.is_empty() && ai.is_none() {
            if let Some(Err(e)) = localai::install_result() {
                errors.push(e);
            }
            break;
        }
        let cur = list.iter().find(|d| d.1 >= 0.0 || d.1 == -1.0);
        let cur_f = cur.map_or(0.0, |d| d.1.max(0.0));
        let ai_f = ai.as_ref().map_or(0.0, |a| a.0.max(0.0));
        let remaining = list.len() as f32 + if ai.is_some() { 1.0 } else { 0.0 };
        let done = (total - remaining).max(0.0) + cur_f.min(1.0) + if ai.is_some() && list.is_empty() { ai_f } else { 0.0 };
        let pct = if total > 0.0 { (done / total * 100.0).clamp(0.0, 100.0) } else { 100.0 };
        let text = cur.map(|d| format!("{} – {}", d.0, d.2)).or_else(|| ai.as_ref().map(|a| a.1.clone())).unwrap_or_default();
        report(pct, &text);
        std::thread::sleep(Duration::from_millis(400));
    }
    report(100.0, "fertig");
    if errors.is_empty() {
        Ok(())
    } else {
        Err(errors.join("; "))
    }
}

/// Kommandozeile: `--list-ai` bzw. `--install-ai id,id,…` (Ausgabe zeilenweise auf stdout)
pub fn cli(args: &[String]) -> Option<i32> {
    use std::io::Write;
    let out = |s: String| {
        let mut o = std::io::stdout().lock();
        let _ = writeln!(o, "{s}");
        let _ = o.flush();
    };
    match args.get(1).map(String::as_str) {
        Some("--list-ai") => {
            out(catalog_json());
            Some(0)
        }
        Some("--install-ai") => {
            let ids: Vec<String> = args.get(2).map(|s| s.split(',').map(|x| x.trim().to_string()).filter(|x| !x.is_empty()).collect()).unwrap_or_default();
            let r = install(&ids, |p, t| out(format!("PROGRESS {:.1} {}", p, t.replace('\n', " "))));
            match r {
                Ok(()) => {
                    out("DONE".into());
                    Some(0)
                }
                Err(e) => {
                    out(format!("ERROR {}", e.replace('\n', " ")));
                    Some(1)
                }
            }
        }
        _ => None,
    }
}
