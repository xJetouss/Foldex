//! Papierkorb eines Cloud-Kontos: Google Drive direkt in Foldex (anzeigen, wiederherstellen, endgültig löschen,
//! leeren) über die Drive-Schnittstelle mit der Anmeldung, die rclone gespeichert hat. Andere Anbieter bieten
//! dafür keine Schnittstelle in rclone – dort öffnet Foldex den Papierkorb auf der Webseite des Anbieters.

use serde_json::Value;
use std::time::Duration;

#[derive(Clone, Debug)]
pub struct Item {
    pub id: String,
    pub name: String,
    pub size: u64,
    pub dir: bool,
    /// gelöscht am (Text aus der Schnittstelle, z. B. 2026-11-08T09:43:16.000Z)
    pub trashed: String,
}

/// Papierkorb-Tab eines Kontos
pub struct TrashView {
    pub remote: String,
    pub name: String,
    pub items: Option<Result<Vec<Item>, String>>,
    pub sel: std::collections::HashSet<String>,
    pub busy: bool,
    /// Rückfrage: 1 = Auswahl endgültig löschen, 2 = Papierkorb leeren
    pub confirm: u8,
    pub msg: String,
}

impl TrashView {
    pub fn new(remote: String, name: String) -> Self {
        TrashView { remote, name, items: None, sel: Default::default(), busy: false, confirm: 0, msg: String::new() }
    }
}

/// Papierkorb in Foldex selbst?
pub fn in_app(key: &str) -> bool {
    key == "gdrive"
}

/// Papierkorb auf der Webseite des Anbieters
pub fn web_url(key: &str) -> Option<&'static str> {
    Some(match key {
        "gdrive" => "https://drive.google.com/drive/trash",
        "icloud" => "https://www.icloud.com/iclouddrive/recently-deleted",
        "onedrive" => "https://onedrive.live.com/?v=recyclebin",
        "dropbox" => "https://www.dropbox.com/deleted_files",
        "box" => "https://app.box.com/trash",
        "pcloud" => "https://my.pcloud.com/#page=trash",
        "mega" => "https://mega.nz/fm/rubbish",
        "proton" => "https://drive.proton.me/trash",
        _ => return None,
    })
}

const API: &str = "https://www.googleapis.com/drive/v3";

fn agent() -> ureq::Agent {
    ureq::AgentBuilder::new().timeout(Duration::from_secs(30)).build()
}

/// Anfrage mit Wiederholung: Google bremst die gemeinsame rclone-Kennung oft kurz („Quota exceeded“, 403/429)
fn retry(f: impl Fn() -> Result<ureq::Response, ureq::Error>) -> Result<ureq::Response, String> {
    let mut wait = 2;
    for _ in 0..5 {
        match f() {
            Err(ureq::Error::Status(c @ (403 | 429 | 500 | 502 | 503), r)) => {
                let body = r.into_string().unwrap_or_default();
                let limit = c != 403 || body.contains("ate limit") || body.contains("uota") || body.contains("rateLimit");
                if !limit {
                    return Err(err(ureq::Error::Status(c, ureq::Response::new(c, "", &body).unwrap())));
                }
                std::thread::sleep(Duration::from_secs(wait));
                wait *= 2;
            }
            r => return r.map_err(err),
        }
    }
    Err("Google bremst gerade alle Anfragen (zu viele über die gemeinsame Kennung) – bitte in einer Minute „Aktualisieren“.".into())
}

fn err(e: ureq::Error) -> String {
    match e {
        ureq::Error::Status(c, r) => {
            let body = r.into_string().unwrap_or_default();
            let msg = serde_json::from_str::<Value>(&body).ok().and_then(|j| j.pointer("/error/message").and_then(|m| m.as_str()).map(str::to_string)).unwrap_or(body);
            format!("Google meldet {c}: {}", msg.chars().take(200).collect::<String>())
        }
        e => format!("Keine Verbindung zu Google: {e}"),
    }
}

/// alle Einträge im Papierkorb (nur die direkt gelöschten, nicht deren Inhalt)
pub fn list(remote: &str) -> Result<Vec<Item>, String> {
    let token = crate::cloudlogin::drive_token(remote).ok_or("Anmeldung nicht gefunden – bitte neu anmelden")?;
    let mut v = Vec::new();
    let mut page: Option<String> = None;
    loop {
        let mut r = agent()
            .get(&format!("{API}/files"))
            .set("Authorization", &format!("Bearer {token}"))
            .query("q", "trashed=true and explicitlyTrashed=true")
            .query("pageSize", "1000")
            .query("fields", "nextPageToken,files(id,name,size,mimeType,trashedTime,modifiedTime)");
        if let Some(p) = &page {
            r = r.query("pageToken", p);
        }
        let j: Value = serde_json::from_str(&retry(|| r.clone().call())?.into_string().map_err(|e| e.to_string())?).map_err(|e| e.to_string())?;
        for f in j.get("files").and_then(|f| f.as_array()).into_iter().flatten() {
            let s = |k: &str| f.get(k).and_then(|v| v.as_str()).unwrap_or("").to_string();
            v.push(Item {
                id: s("id"),
                name: s("name"),
                size: s("size").parse().unwrap_or(0),
                dir: s("mimeType") == "application/vnd.google-apps.folder",
                // nicht immer gesetzt (z. B. über andere Programme gelöscht) → zuletzt geändert
                trashed: Some(s("trashedTime")).filter(|t| !t.is_empty()).unwrap_or_else(|| s("modifiedTime")),
            });
        }
        page = j.get("nextPageToken").and_then(|p| p.as_str()).map(str::to_string);
        if page.is_none() {
            break;
        }
    }
    v.sort_by(|a, b| b.trashed.cmp(&a.trashed));
    Ok(v)
}

/// zurück an den ursprünglichen Ort
pub fn restore(remote: &str, ids: &[String]) -> Result<usize, String> {
    let token = crate::cloudlogin::drive_token(remote).ok_or("Anmeldung nicht gefunden")?;
    let mut n = 0;
    for id in ids {
        retry(|| {
            agent()
                .request("PATCH", &format!("{API}/files/{id}"))
                .set("Authorization", &format!("Bearer {token}"))
                .set("Content-Type", "application/json")
                .send_string(r#"{"trashed":false}"#)
        })?;
        n += 1;
    }
    Ok(n)
}

/// in den Papierkorb legen (für „Rückgängig“ von Wiederherstellen bzw. Löschen in Foldex)
pub fn trash(remote: &str, ids: &[String]) -> Result<usize, String> {
    let token = crate::cloudlogin::drive_token(remote).ok_or("Anmeldung nicht gefunden")?;
    let mut n = 0;
    for id in ids {
        retry(|| {
            agent()
                .request("PATCH", &format!("{API}/files/{id}"))
                .set("Authorization", &format!("Bearer {token}"))
                .set("Content-Type", "application/json")
                .send_string(r#"{"trashed":true}"#)
        })?;
        n += 1;
    }
    Ok(n)
}

/// endgültig löschen (nicht rückgängig zu machen)
pub fn delete_forever(remote: &str, ids: &[String]) -> Result<usize, String> {
    let token = crate::cloudlogin::drive_token(remote).ok_or("Anmeldung nicht gefunden")?;
    let mut n = 0;
    for id in ids {
        retry(|| agent().delete(&format!("{API}/files/{id}")).set("Authorization", &format!("Bearer {token}")).call())?;
        n += 1;
    }
    Ok(n)
}

/// ganzen Papierkorb leeren (nicht rückgängig zu machen)
pub fn empty(remote: &str) -> Result<(), String> {
    let token = crate::cloudlogin::drive_token(remote).ok_or("Anmeldung nicht gefunden")?;
    retry(|| agent().delete(&format!("{API}/files/trash")).set("Authorization", &format!("Bearer {token}")).call())?;
    Ok(())
}

/// „2026-11-08T09:43:16.000Z“ → „08.11.2026 10:43“ (Ortszeit)
pub fn fmt_time(s: &str) -> String {
    chrono::DateTime::parse_from_rfc3339(s).map(|t| t.with_timezone(&chrono::Local).format("%d.%m.%Y %H:%M").to_string()).unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// eigene Testdatei in den Papierkorb, wiederherstellen, erneut löschen, endgültig löschen
    #[test]
    #[ignore]
    fn real_trash() {
        let a = crate::cloudlogin::accounts().into_iter().find(|a| a.key == "gdrive").expect("kein Google");
        let exe = crate::cloudlogin::rclone().unwrap();
        let name = format!("foldex-papierkorb-test-{}.txt", std::process::id());
        let run = |args: &[&str]| {
            let mut c = std::process::Command::new(&exe);
            c.arg("--config").arg(dirs::config_dir().unwrap().join("foldex").join("cloud").join("rclone.conf")).args(args);
            c.output().unwrap()
        };
        let target = format!("{}:{name}", a.remote);
        let mut c = std::process::Command::new(&exe);
        c.arg("--config").arg(dirs::config_dir().unwrap().join("foldex").join("cloud").join("rclone.conf")).args(["rcat", &target]).stdin(std::process::Stdio::piped());
        let mut ch = c.spawn().unwrap();
        use std::io::Write;
        ch.stdin.take().unwrap().write_all(b"test").unwrap();
        ch.wait().unwrap();
        run(&["deletefile", &target]);
        let find = || list(&a.remote).unwrap().into_iter().find(|i| i.name == name);
        let it = find().expect("nicht im Papierkorb");
        eprintln!("im Papierkorb: {} gelöscht {}", it.name, fmt_time(&it.trashed));
        assert_eq!(restore(&a.remote, &[it.id.clone()]).unwrap(), 1);
        let back = String::from_utf8_lossy(&run(&["lsf", &format!("{}:", a.remote)]).stdout).contains(&name);
        assert!(back, "nicht wiederhergestellt");
        assert!(find().is_none());
        run(&["deletefile", &target]);
        let it = find().expect("nicht wieder im Papierkorb");
        assert_eq!(delete_forever(&a.remote, &[it.id]).unwrap(), 1);
        assert!(find().is_none(), "nicht endgültig gelöscht");
        eprintln!("wiederhergestellt und endgültig gelöscht: ok");
    }
}
