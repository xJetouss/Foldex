//! Kurzbefehle („Skills“) für die KI-Chats: „/aufräumen“, „/commit“, „/erklären“ … stehen für einen festen
//! Prompt. Eingabe „/“ zeigt die Liste; was nach dem Befehl steht, wird an der Stelle `{text}` eingesetzt
//! (oder angehängt). Eigene Kurzbefehle: `…/foldex/kurzbefehle.json` (Name, Beschreibung, Prompt).

use std::path::PathBuf;

#[derive(Clone, serde::Serialize, serde::Deserialize)]
pub struct Skill {
    pub name: String,
    #[serde(default)]
    pub desc: String,
    pub prompt: String,
    /// Gruppe für die Liste
    #[serde(default)]
    pub group: String,
}

/// eingebaute Kurzbefehle: (Name, Gruppe, Beschreibung, Prompt)
const BUILTIN: &[(&str, &str, &str, &str)] = &[
    // Dateien und Ordner
    ("aufräumen", "Dateien", "Ordner sortieren: Unterordner nach Art, Datum oder Projekt", "Räume den aktuellen Ordner auf: Schau dir alle Dateien an, schlage eine sinnvolle Ordnerstruktur vor (nach Art, Datum oder Projekt) und verschiebe die Dateien nach meiner Bestätigung. Lösche nichts. {text}"),
    ("umbenennen", "Dateien", "Einheitliche, sprechende Dateinamen", "Benenne die markierten (oder alle) Dateien im aktuellen Ordner einheitlich und sprechend um – z. B. Datum vorne (JJJJ-MM-TT), dann Inhalt. Zeig mir die Liste alt → neu, bevor du umbenennst. {text}"),
    ("duplikate", "Dateien", "Doppelte Dateien finden", "Finde doppelte Dateien im aktuellen Ordner und seinen Unterordnern (gleicher Inhalt, nicht nur gleicher Name). Zeig sie gruppiert mit Größe und schlage vor, welche behalten werden. Lösche nichts ohne Rückfrage. {text}"),
    ("groß", "Dateien", "Größte Dateien und Platzfresser", "Welche Dateien und Ordner belegen hier am meisten Platz? Liste die 20 größten mit Größe auf und sag, was davon vermutlich weg kann (Caches, Installer, Duplikate). {text}"),
    ("zusammenfassen", "Dateien", "Inhalt der markierten Dateien kurz zusammenfassen", "Fasse den Inhalt der markierten Dateien kurz zusammen: worum geht es, wichtigste Punkte, offene Fragen. Stichpunkte, höchstens eine halbe Seite. {text}"),
    ("sortieren", "Dateien", "Fotos und Downloads nach Datum in Ordner", "Sortiere die Dateien im aktuellen Ordner nach Datum in Unterordner (JJJJ/MM). Nutze das Aufnahmedatum bei Fotos, sonst das Änderungsdatum. Zeig vorher, was passiert. {text}"),
    ("archivieren", "Dateien", "Alte Dateien in ein ZIP packen", "Packe Dateien im aktuellen Ordner, die älter als ein Jahr sind, in ein ZIP-Archiv „Archiv-JJJJ.zip“ und frag, ob die Originale danach in den Papierkorb sollen. {text}"),
    ("finde", "Dateien", "Datei nach Inhalt finden (PDFs, Bilder, Dokumente)", "Finde im aktuellen Ordner die Datei, auf die diese Beschreibung passt, und erkläre, warum sie passt: {text}"),
    ("bericht", "Dateien", "Überblick über den Ordner", "Gib mir einen Überblick über den aktuellen Ordner: Anzahl und Arten der Dateien, Gesamtgröße, neueste Änderungen, Auffälligkeiten. {text}"),
    // Code
    ("erklären", "Code", "Code oder Datei verständlich erklären", "Erkläre verständlich, was der folgende Code bzw. die markierte Datei macht – erst in zwei Sätzen, dann Schritt für Schritt. Weise auf Stolperstellen hin. {text}"),
    ("commit", "Code", "Commit-Nachricht aus den Änderungen", "Schau dir die aktuellen Git-Änderungen (git diff, auch ungestagt) an und schreibe eine prägnante Commit-Nachricht: erste Zeile höchstens 72 Zeichen im Imperativ, danach kurz das Warum. Committe erst nach meiner Bestätigung. {text}"),
    ("review", "Code", "Code-Review: Fehler, Risiken, Verbesserungen", "Mach ein gründliches Code-Review der aktuellen Änderungen bzw. der markierten Dateien: echte Fehler zuerst (mit Zeile und Begründung), dann Risiken, dann kleinere Verbesserungen. Keine Stilfragen ohne Grund. {text}"),
    ("fix", "Code", "Fehler finden und beheben", "Finde die Ursache dieses Fehlers und behebe sie mit der kleinstmöglichen Änderung. Erkläre kurz, warum es vorher kaputt war: {text}"),
    ("tests", "Code", "Tests schreiben", "Schreibe sinnvolle Tests für den folgenden Code bzw. die markierte Datei – normale Fälle, Grenzfälle und Fehlerfälle – im Test-Framework, das das Projekt schon nutzt. {text}"),
    ("refactor", "Code", "Code vereinfachen, ohne das Verhalten zu ändern", "Vereinfache den folgenden Code bzw. die markierte Datei, ohne das Verhalten zu ändern: doppelten Code zusammenfassen, Namen verbessern, unnötige Verschachtelung entfernen. Zeig die Änderungen. {text}"),
    ("doku", "Code", "Dokumentation und Kommentare schreiben", "Schreibe eine klare Dokumentation für den folgenden Code bzw. das Projekt: Zweck, Benutzung mit Beispiel, wichtige Parameter. Kommentare nur, wo sie etwas erklären, das der Code nicht zeigt. {text}"),
    ("readme", "Code", "README für das Projekt", "Schreibe eine README.md für das Projekt im aktuellen Ordner: was es ist, Installation, Benutzung, Aufbau, Lizenzhinweis. Schau dir dazu die Dateien an. {text}"),
    ("übersetzen-code", "Code", "Code in eine andere Sprache übertragen", "Übertrage den folgenden Code in die genannte Programmiersprache – idiomatisch, nicht Zeile für Zeile: {text}"),
    ("regex", "Code", "Regulären Ausdruck bauen und erklären", "Baue einen regulären Ausdruck für folgende Aufgabe, erkläre ihn Teil für Teil und gib drei passende und zwei nicht passende Beispiele: {text}"),
    ("sql", "Code", "SQL-Abfrage schreiben", "Schreibe eine SQL-Abfrage für folgende Aufgabe und erkläre sie kurz: {text}"),
    ("shell", "Code", "Befehl für Terminal/PowerShell", "Gib mir den passenden Terminal-Befehl (PowerShell unter Windows, sonst Bash) für folgende Aufgabe, mit kurzer Erklärung. Nichts ausführen, was Daten löscht, ohne zu fragen: {text}"),
    ("performance", "Code", "Langsame Stellen finden", "Finde die Stellen im folgenden Code bzw. Projekt, die vermutlich am meisten Zeit oder Speicher kosten, und schlage konkrete, messbare Verbesserungen vor. {text}"),
    ("sicherheit", "Code", "Sicherheitslücken prüfen", "Prüfe den folgenden Code bzw. die Änderungen auf Sicherheitsprobleme (Eingaben, Pfade, Geheimnisse im Code, Injection, Rechte) und nenne konkrete Abhilfen. {text}"),
    ("changelog", "Code", "Änderungsprotokoll aus Git", "Erstelle aus dem Git-Verlauf seit dem letzten Tag (bzw. den letzten 30 Commits) ein Änderungsprotokoll für Nutzer: Neu, Verbessert, Behoben. {text}"),
    ("gitignore", "Code", ".gitignore passend zum Projekt", "Erstelle eine passende .gitignore für das Projekt im aktuellen Ordner (Sprache und Werkzeuge anhand der Dateien erkennen). {text}"),
    // Schreiben
    ("korrigieren", "Schreiben", "Rechtschreibung und Grammatik verbessern", "Korrigiere Rechtschreibung, Grammatik und Zeichensetzung im folgenden Text. Ändere Stil und Inhalt nicht. Gib nur den korrigierten Text zurück: {text}"),
    ("kürzer", "Schreiben", "Text kürzen, Kern behalten", "Kürze den folgenden Text auf etwa die Hälfte, ohne wichtige Aussagen zu verlieren: {text}"),
    ("förmlich", "Schreiben", "In höflich-förmlichen Ton bringen", "Formuliere den folgenden Text höflich und förmlich (Sie-Form), klar und ohne Floskeln: {text}"),
    ("locker", "Schreiben", "In lockeren Ton bringen", "Formuliere den folgenden Text locker und freundlich (Du-Form), ohne den Inhalt zu ändern: {text}"),
    ("übersetzen", "Schreiben", "Ins Englische (oder genannte Sprache) übersetzen", "Übersetze den folgenden Text ins Englische – oder in die Sprache, die ich nenne. Natürlich klingend, nicht wörtlich: {text}"),
    ("email", "Schreiben", "E-Mail formulieren", "Schreibe eine klare, freundliche E-Mail zu folgendem Anliegen. Mit Betreff, kurz und auf den Punkt: {text}"),
    ("antwort", "Schreiben", "Auf eine Nachricht antworten", "Schreibe eine passende Antwort auf folgende Nachricht – freundlich, kurz, alle Fragen beantwortet: {text}"),
    ("brief", "Schreiben", "Formellen Brief aufsetzen", "Setze einen formellen Brief (z. B. Kündigung, Widerspruch, Anfrage) zu folgendem Anliegen auf, mit Platzhaltern für Adressen und Datum: {text}"),
    ("stichpunkte", "Schreiben", "Text in Stichpunkte fassen", "Fasse den folgenden Text in übersichtlichen Stichpunkten zusammen: {text}"),
    ("protokoll", "Schreiben", "Notizen zu einem Protokoll machen", "Mach aus diesen Notizen ein sauberes Protokoll: Teilnehmer, Themen, Entscheidungen, Aufgaben mit Verantwortlichen und Terminen: {text}"),
    ("bewerbung", "Schreiben", "Anschreiben für eine Bewerbung", "Schreibe ein überzeugendes, ehrliches Bewerbungsanschreiben (höchstens eine Seite) für folgende Stelle und mein Profil: {text}"),
    ("titel", "Schreiben", "Überschriften vorschlagen", "Schlage zehn treffende, unterschiedliche Überschriften für folgenden Inhalt vor: {text}"),
    ("tweet", "Schreiben", "Kurzer Beitrag für soziale Netzwerke", "Schreibe drei kurze Beiträge (höchstens 280 Zeichen) für soziale Netzwerke zu folgendem Thema, mit passenden Hashtags: {text}"),
    // Lernen und Denken
    ("einfach", "Lernen", "Wie für ein Kind erklärt", "Erkläre Folgendes so einfach, dass es ein zwölfjähriges Kind versteht, mit einem Alltagsbeispiel: {text}"),
    ("quiz", "Lernen", "Quizfragen zum Thema", "Stell mir zehn Quizfragen (gemischt leicht bis schwer) zu folgendem Thema, die Lösungen erst ganz am Ende: {text}"),
    ("lernplan", "Lernen", "Lernplan erstellen", "Erstelle einen realistischen Lernplan für folgendes Ziel – Wochen, Themen, Übungen, Zwischenziele: {text}"),
    ("pro-contra", "Lernen", "Vor- und Nachteile abwägen", "Wäge Vor- und Nachteile zu folgender Entscheidung ab, als Tabelle, mit einer klaren Empfehlung am Ende: {text}"),
    ("ideen", "Lernen", "Ideen sammeln", "Sammle zwanzig unterschiedliche Ideen zu folgendem Thema – von naheliegend bis ungewöhnlich: {text}"),
    ("plan", "Lernen", "Schritt-für-Schritt-Plan", "Mach einen konkreten Schritt-für-Schritt-Plan für folgendes Vorhaben, mit Reihenfolge, Aufwand und möglichen Stolpersteinen: {text}"),
    ("vergleich", "Lernen", "Dinge vergleichen", "Vergleiche Folgendes übersichtlich in einer Tabelle (wichtigste Merkmale, Preis, Stärken, Schwächen) und gib eine Empfehlung: {text}"),
    ("faktencheck", "Lernen", "Aussage prüfen", "Prüfe folgende Aussage kritisch: Was stimmt, was ist umstritten, was ist falsch? Sag ehrlich, wo du unsicher bist: {text}"),
    // Daten
    ("tabelle", "Daten", "Text als Tabelle", "Bring folgende Angaben in eine übersichtliche Markdown-Tabelle: {text}"),
    ("csv", "Daten", "CSV auswerten", "Werte die markierte CSV-/Excel-Datei aus: Spalten, auffällige Werte, Summen und Durchschnitte, drei interessante Erkenntnisse. {text}"),
    ("json", "Daten", "JSON prüfen und erklären", "Prüfe das folgende JSON auf Fehler, formatiere es lesbar und erkläre die Struktur: {text}"),
    ("rechnen", "Daten", "Rechnung Schritt für Schritt", "Rechne Folgendes Schritt für Schritt nach und gib das Ergebnis klar an: {text}"),
    ("budget", "Daten", "Kosten aufstellen", "Stelle eine übersichtliche Kostenaufstellung (Tabelle mit Posten, Betrag, Summe) für Folgendes auf: {text}"),
    // Bilder
    ("bild", "Bilder", "Bildbeschreibung für die Bild-KI", "Schreibe einen ausführlichen englischen Prompt für eine Bild-KI (Motiv, Stil, Licht, Perspektive, Details) zu folgender Idee: {text}"),
    ("alt-text", "Bilder", "Bildbeschreibung (Barrierefreiheit)", "Beschreibe das angehängte bzw. markierte Bild sachlich in ein bis zwei Sätzen als Alternativtext. {text}"),
];

pub fn builtin() -> Vec<Skill> {
    BUILTIN.iter().map(|(n, g, d, p)| Skill { name: n.to_string(), desc: d.to_string(), prompt: p.to_string(), group: g.to_string() }).collect()
}

pub fn custom_file() -> PathBuf {
    dirs::config_dir().unwrap_or_else(std::env::temp_dir).join("foldex").join("kurzbefehle.json")
}

/// eigene Kurzbefehle (überschreiben eingebaute mit gleichem Namen)
pub fn custom() -> Vec<Skill> {
    std::fs::read(custom_file()).ok().and_then(|b| serde_json::from_slice::<Vec<Skill>>(&b).ok()).unwrap_or_default()
}

/// Datei mit Beispiel anlegen (zum Bearbeiten)
pub fn ensure_custom_file() -> PathBuf {
    let f = custom_file();
    if !f.is_file() {
        let _ = std::fs::create_dir_all(f.parent().unwrap_or(std::path::Path::new(".")));
        let ex = vec![Skill {
            name: "wochenbericht".into(),
            desc: "Beispiel: eigener Kurzbefehl".into(),
            prompt: "Schreibe aus diesen Stichpunkten einen kurzen Wochenbericht für mein Team: {text}".into(),
            group: "Eigene".into(),
        }];
        let _ = std::fs::write(&f, serde_json::to_string_pretty(&ex).unwrap_or_default());
    }
    f
}

pub fn all() -> Vec<Skill> {
    let own = custom();
    let mut v: Vec<Skill> = own.iter().cloned().map(|mut s| {
        if s.group.is_empty() {
            s.group = "Eigene".into();
        }
        s
    }).collect();
    v.extend(builtin().into_iter().filter(|b| !own.iter().any(|o| o.name.eq_ignore_ascii_case(&b.name))));
    v
}

fn fold(s: &str) -> String {
    s.to_lowercase().replace('ä', "a").replace('ö', "o").replace('ü', "u").replace('ß', "ss")
}

/// passende Kurzbefehle zur Eingabe „/auf…“ (Name beginnt damit, sonst enthält Name/Beschreibung)
pub fn matching(input: &str) -> Vec<Skill> {
    let Some(q) = input.strip_prefix('/') else { return Vec::new() };
    if q.contains(char::is_whitespace) {
        return Vec::new();
    }
    let q = fold(q);
    let all = all();
    let mut v: Vec<Skill> = all.iter().filter(|s| fold(&s.name).starts_with(&q)).cloned().collect();
    v.extend(all.into_iter().filter(|s| !fold(&s.name).starts_with(&q) && (fold(&s.name).contains(&q) || fold(&s.desc).contains(&q))));
    v
}

/// „/name rest“ → fertiger Prompt (None = kein Kurzbefehl)
pub fn expand(input: &str) -> Option<String> {
    let s = input.trim_start().strip_prefix('/')?;
    let (name, rest) = s.split_once(char::is_whitespace).unwrap_or((s, ""));
    let skill = all().into_iter().find(|k| fold(&k.name) == fold(name))?;
    let rest = rest.trim();
    Some(if skill.prompt.contains("{text}") {
        skill.prompt.replace("{text}", rest).trim().to_string()
    } else if rest.is_empty() {
        skill.prompt
    } else {
        format!("{}\n\n{rest}", skill.prompt)
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn expands() {
        let p = expand("/erklären fn main() {}").unwrap();
        assert!(p.starts_with("Erkläre") && p.ends_with("fn main() {}"));
        assert!(expand("/gibtsnicht x").is_none());
        assert!(expand("hallo").is_none());
        assert_eq!(matching("/auf")[0].name, "aufräumen");
        assert!(matching("/erkl").iter().any(|s| s.name == "erklären"));
        assert!(BUILTIN.len() >= 50);
    }
}
