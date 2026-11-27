# Foldex-Plugins (Plugin-API 1)

Mit Plugins fügst du Foldex eigene Befehle hinzu – im **Rechtsklick-Menü** und in der **Befehlspalette**
(Strg+Leertaste). Ein Befehl startet ein Programm oder Skript mit den markierten Dateien; die Ausgabe
erscheint in der Statusleiste oder in einem Fenster. Foldex muss dafür nicht neu gebaut werden.

Online-Version mit Beispielen: <https://foldex.tssnt.dev/plugins.html>

## Wo Plugins liegen

Jedes Plugin ist ein Ordner mit einer `plugin.json`:

```
%APPDATA%\foldex\plugins\          (Linux: ~/.config/foldex/plugins/)
├── bilder\
│   ├── plugin.json
│   └── verkleinern.ps1
└── git-helfer\
    └── plugin.json
```

In Foldex: **Einstellungen → Plugins** zeigt alle Plugins, schaltet sie ein und aus, öffnet den Plugin-Ordner,
lädt nach Änderungen neu („Neu laden“) und legt ein **Beispiel-Plugin** an.
Mit der Umgebungsvariable `FOLDEX_PLUGINS_DIR` lässt sich ein anderer Ordner verwenden (z. B. zum Entwickeln).

## plugin.json

```json
{
  "name": "Bilder verkleinern",
  "version": "1.0",
  "author": "Jan",
  "description": "Verkleinert Fotos mit ImageMagick",
  "homepage": "https://example.com",
  "api": 1,
  "commands": [
    {
      "id": "klein",
      "title": "Auf 1080 px verkleinern",
      "when": ["file"],
      "extensions": ["jpg", "png"],
      "each": true,
      "run": "magick",
      "args": ["{file}", "-resize", "1080x1080", "{parent}/{stem}-klein.{ext}"],
      "output": "status",
      "refresh": true,
      "icon": "image"
    }
  ]
}
```

### Plugin-Felder

| Feld | Pflicht | Bedeutung |
|---|---|---|
| `name` | ja | Anzeigename |
| `id` | nein | eindeutige ID (Standard: Ordnername) |
| `version`, `author`, `description`, `homepage` | nein | erscheinen in den Einstellungen |
| `api` | nein | Version der Plugin-API (Standard 1). Ist sie höher als die von Foldex, wird das Plugin nicht geladen |
| `commands` | nein | Liste der Befehle |

### Befehls-Felder

| Feld | Standard | Bedeutung |
|---|---|---|
| `id` | – (Pflicht) | eindeutig im Plugin |
| `title` | – (Pflicht) | Text im Menü |
| `run` | – (Pflicht) | Programm oder Skript – relativ zum Plugin-Ordner oder ein Programm im `PATH` |
| `args` | `[]` | Argumente, mit Platzhaltern (siehe unten) |
| `description` | `""` | Beschreibung |
| `when` | `["file"]` | wo der Befehl erscheint: `file`, `folder`, `background` (Rechtsklick auf eine leere Stelle) |
| `extensions` | alle | nur für diese Endungen, z. B. `["pdf", "docx"]` |
| `multiple` | `true` | auch bei mehreren markierten Einträgen anbieten |
| `each` | `false` | bei mehreren Einträgen das Programm einmal **je Eintrag** starten |
| `cwd` | aktueller Ordner | Arbeitsordner (Platzhalter erlaubt) |
| `output` | `"status"` | `none` = nur starten (eigenes Fenster, z. B. für GUI-Programme), `status` = letzte Ausgabezeile in der Statusleiste, `window` = komplette Ausgabe in einem Fenster |
| `refresh` | `false` | Ordner danach neu einlesen (wenn der Befehl Dateien anlegt oder ändert) |
| `confirm` | `""` | Rückfrage vor dem Ausführen, z. B. `"Wirklich alle Bilder verkleinern?"` |
| `icon` | Befehl | `file`, `folder`, `image`, `music`, `video`, `code`, `terminal`, `tools`, `star`, `spark`, `share`, `refresh`, `git`, `copy`, `trash` |

### Platzhalter in `args` und `cwd`

| Platzhalter | Wert |
|---|---|
| `{files}` | alle markierten Pfade – als eigenes Argument wird es zu **mehreren** Argumenten |
| `{file}` | erster markierter Pfad |
| `{name}` | Dateiname mit Endung |
| `{stem}` | Dateiname ohne Endung |
| `{ext}` | Endung ohne Punkt |
| `{parent}` | Ordner der ersten Datei |
| `{dir}` | aktueller Ordner in Foldex |
| `{plugin}` | Ordner des Plugins |
| `{count}` | Anzahl der markierten Einträge |

Bei `"when": ["background"]` gibt es keine markierten Einträge – nutze `{dir}`.

### Skripte

Endet `run` auf `.ps1`, `.bat`/`.cmd`, `.py`, `.js` oder `.sh`, startet Foldex den passenden Interpreter
(`powershell -File`, `cmd /C`, `python`, `node`, `bash`/`sh`). Die Pfade kommen als Argumente an
(PowerShell: `$args`, Python: `sys.argv[1:]`).

Befehle mit `output: status` oder `window` laufen ohne Konsolenfenster im Hintergrund; nach 10 Minuten
bricht Foldex sie ab. Ein Exit-Code ungleich 0 gilt als Fehler und zeigt die Ausgabe im Fenster.

### Je System ein eigenes Programm (Windows, Linux, Android)

Ein Befehl kann für jedes System ein eigenes `run`/`args` haben. Foldex nimmt unter Windows `windows`, unter
Linux `linux` (sonst `unix`) und auf Android `android` (sonst `unix`); fehlt beides, gilt das allgemeine
`run`/`args`. Hat ein Befehl für das laufende System kein Programm, wird er dort ausgeblendet.

```json
{ "id": "sha256", "title": "Prüfsumme (SHA-256)", "when": ["file"],
  "run": "hash.ps1", "args": ["{files}"],
  "unix": { "run": "hash.sh" },
  "output": "window" }
```

Auf Android laufen `.sh`-Skripte mit der System-Shell (`/system/bin/sh`, Toybox: `find`, `sed`, `sort`,
`sha256sum`, `du`, `wc` …).

### Logo

Ein `icon.png` (quadratisch, z. B. 128 × 128) im Plugin-Ordner erscheint in den Einstellungen links neben dem Plugin.

## Katalog

Unter **Einstellungen → Plugins → Katalog** listet Foldex die Plugins aus dem Ordner
[`plugins/`](https://github.com/xJetouss/Foldex/tree/main/plugins) des Foldex-Repositorys – mit Logo,
„Installieren“ und „Entfernen“. `plugins/index.json` nennt je Plugin `id`, `name`, `description`, `version`,
`author`, die Dateien (`files`) und optional die Systeme (`platforms`: `windows`, `linux`, `android`).

## Beispiele

**Pfade kopieren** (`kopieren.ps1` liegt im Plugin-Ordner):

```json
{ "id": "pfade", "title": "Pfade als Liste kopieren", "when": ["file", "folder"],
  "run": "kopieren.ps1", "args": ["{files}"], "output": "status" }
```

```powershell
$args | Set-Clipboard
"$($args.Count) Pfad(e) kopiert"
```

**Git-Log des Ordners** (Rechtsklick auf leere Stelle):

```json
{ "id": "log", "title": "Letzte Commits", "when": ["background"],
  "run": "git", "args": ["log", "--oneline", "-20"], "output": "window", "icon": "git" }
```

**PDFs zusammenfügen** (mit qpdf, nur bei mehreren PDFs):

```json
{ "id": "merge", "title": "PDFs zusammenfügen", "extensions": ["pdf"],
  "run": "qpdf", "args": ["--empty", "--pages", "{files}", "--", "{dir}/zusammen.pdf"],
  "refresh": true, "confirm": "Die markierten PDFs zu zusammen.pdf zusammenfügen?" }
```

**In VLC öffnen** (Programm mit eigenem Fenster):

```json
{ "id": "vlc", "title": "In VLC abspielen", "extensions": ["mp4", "mkv", "mp3"],
  "run": "C:/Program Files/VideoLAN/VLC/vlc.exe", "args": ["{files}"], "output": "none", "icon": "video" }
```

## Sicherheit

Plugins laufen mit deinen Rechten wie jedes andere Programm. Installiere nur Plugins aus Quellen, denen du
vertraust, und sieh dir `plugin.json` und Skripte vor dem Einschalten an.
