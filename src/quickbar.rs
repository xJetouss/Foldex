//! Schnellleiste (Befehlsleiste über der Dateiliste): welche Knöpfe in welcher Reihenfolge erscheinen.
//! Die Auswahl wird in den Einstellungen per Ziehen sortiert und ein-/ausgeschaltet.

/// Alle verfügbaren Knöpfe: (ID, Beschriftung in den Einstellungen, standardmäßig sichtbar).
/// „sep…“ sind Trenner, die wie Knöpfe verschoben und ausgeblendet werden können.
pub const ITEMS: &[(&str, &str, bool)] = &[
    ("new", "Neu (Menü)", true),
    ("newfolder", "Neuer Ordner", false),
    ("sep1", "Trenner", true),
    ("cut", "Ausschneiden", true),
    ("copy", "Kopieren", true),
    ("paste", "Einfügen", true),
    ("rename", "Umbenennen", true),
    ("trash", "Löschen (Papierkorb)", true),
    ("sep2", "Trenner", true),
    ("undo", "Rückgängig", true),
    ("redo", "Wiederholen", true),
    ("sep3", "Trenner", true),
    ("sort", "Sortieren", true),
    ("view", "Ansicht", true),
    ("preview", "Vorschau", true),
    ("filter", "Filter", true),
    ("select", "Auswahl", true),
    ("sep4", "Trenner", true),
    ("tools", "Werkzeuge", true),
    ("terminal", "Terminal", true),
    ("git", "Git", true),
    ("ai", "KI-Chat", true),
    ("split", "Ansicht teilen", true),
    ("share", "Teilen", true),
    ("props", "Eigenschaften", true),
    ("copypath", "Pfad kopieren", false),
    ("refresh", "Aktualisieren", false),
    ("compress", "Komprimieren", false),
    ("openin", "Öffnen in Editor", false),
    ("hidden", "Versteckte Dateien", false),
];

pub fn label(id: &str) -> &'static str {
    ITEMS.iter().find(|i| i.0 == id).map_or("?", |i| i.1)
}

pub fn defaults() -> Vec<(String, bool)> {
    ITEMS.iter().map(|(id, _, on)| (id.to_string(), *on)).collect()
}

/// Gespeicherte Liste bereinigen: unbekannte IDs entfernen, neue Knöpfe hinten anhängen, Doppelte entfernen.
pub fn normalize(v: Vec<(String, bool)>) -> Vec<(String, bool)> {
    let mut out: Vec<(String, bool)> = Vec::new();
    for (id, on) in v {
        if ITEMS.iter().any(|i| i.0 == id) && !out.iter().any(|o| o.0 == id) {
            out.push((id, on));
        }
    }
    for (id, _, on) in ITEMS {
        if !out.iter().any(|o| o.0 == *id) {
            out.push((id.to_string(), *on));
        }
    }
    out
}

/// Speicherformat: „new:1,cut:1,…“
pub fn encode(v: &[(String, bool)]) -> String {
    v.iter().map(|(id, on)| format!("{id}:{}", if *on { 1 } else { 0 })).collect::<Vec<_>>().join(",")
}

pub fn decode(s: &str) -> Vec<(String, bool)> {
    normalize(s.split(',').filter_map(|p| p.split_once(':').map(|(id, on)| (id.trim().to_string(), on.trim() == "1"))).collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn roundtrip_and_normalize() {
        let d = defaults();
        assert_eq!(decode(&encode(&d)), d);
        // eigene Reihenfolge bleibt, Unbekanntes fällt weg, Fehlendes kommt hinten dazu
        let v = decode("paste:1,cut:0,gibtsnicht:1,paste:0");
        assert_eq!(v[0], ("paste".to_string(), true));
        assert_eq!(v[1], ("cut".to_string(), false));
        assert_eq!(v.len(), ITEMS.len());
    }
}
