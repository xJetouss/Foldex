# Foldex-Plugin-Katalog

Diese Plugins erscheinen in Foldex unter **Einstellungen → Plugins → Katalog** und lassen sich dort mit einem Klick installieren und wieder entfernen – unter Windows, Linux und Android.

- `index.json` – Liste der Plugins (id, name, description, version, author, files, platforms)
- `<id>/plugin.json` – das Plugin (siehe [Plugin-API](https://foldex.tssnt.dev/plugins.html))
- `<id>/icon.png` – Logo (quadratisch, 128 × 128)

Ein Befehl kann je System ein eigenes Programm haben: `"windows"`, `"unix"` (Linux und Android), `"linux"`, `"android"` – jeweils mit `run` und `args`.
