//! LocalSend ohne die LocalSend-App zu öffnen: Foldex tritt mit dem Gerätenamen und Fingerabdruck der installierten
//! App auf (aus deren Einstellungen), sucht die Geräte im Netz und sendet direkt (Protokoll v2).
//! - Suche: Ankündigung per Multicast (224.0.0.167:53317), Antworten per Multicast; zusätzlich werden alle Adressen
//!   im eigenen /24-Netz gefragt (falls Multicast gesperrt ist oder die LocalSend-App die Antworten bekommt).
//! - Senden: prepare-upload (der Empfänger bestätigt), dann upload je Datei über HTTPS. Das selbst signierte
//!   Zertifikat des Empfängers muss zu seinem angekündigten Fingerabdruck passen.

use serde_json::{json, Value};
use std::{
    io::{BufReader, Read},
    net::{IpAddr, Ipv4Addr, SocketAddr, TcpStream, UdpSocket},
    path::{Path, PathBuf},
    sync::{
        atomic::{AtomicBool, AtomicU64, Ordering::Relaxed},
        Arc, Mutex,
    },
    time::Duration,
};

const GROUP: Ipv4Addr = Ipv4Addr::new(224, 0, 0, 167);
const PORT: u16 = 53317;
const UA: &str = concat!("Foldex/", env!("CARGO_PKG_VERSION"));

/// Identität der installierten LocalSend-App. Zertifikat und Schlüssel braucht die Gegenstelle: Empfänger mit HTTPS
/// verlangen ein Client-Zertifikat (sonst „CertificateRequired“) – Foldex weist sich mit dem der App aus.
#[derive(Clone)]
pub struct Me {
    pub alias: String,
    pub fingerprint: String,
    pub port: u16,
    pub https: bool,
    /// (Zertifikat, privater Schlüssel) als PEM – bleibt nur im Speicher
    cert: Option<Arc<(String, String)>>,
}

impl std::fmt::Debug for Me {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Me").field("alias", &self.alias).field("fingerprint", &self.fingerprint).field("port", &self.port).field("https", &self.https).field("cert", &self.cert.is_some()).finish()
    }
}

impl PartialEq for Me {
    fn eq(&self, o: &Me) -> bool {
        (&self.alias, &self.fingerprint, self.port, self.https) == (&o.alias, &o.fingerprint, o.port, o.https)
    }
}

impl Me {
    pub fn json(&self, announce: Option<bool>) -> Value {
        let mut v = json!({
            "alias": self.alias, "version": "2.1", "deviceModel": "Windows", "deviceType": "desktop",
            "fingerprint": self.fingerprint, "port": self.port, "protocol": if self.https { "https" } else { "http" }, "download": false
        });
        if let Some(a) = announce {
            v["announce"] = json!(a);
        }
        v
    }
}

/// Einstellungen der LocalSend-App lesen (Name, Zertifikat-Fingerabdruck, Port, HTTPS).
pub fn identity() -> Option<Me> {
    let f = dirs::config_dir()?.join("LocalSend").join("settings.json");
    parse_identity(&std::fs::read_to_string(f).ok()?)
}

fn parse_identity(t: &str) -> Option<Me> {
    let v: Value = serde_json::from_str(t).ok()?;
    let alias = v.get("flutter.ls_alias")?.as_str()?.to_string();
    // Fingerabdruck = SHA-256 des Zertifikats der App; Zertifikat und Schlüssel für die Anmeldung beim Empfänger
    let sc = v.get("flutter.ls_security_context").and_then(Value::as_str).and_then(|s| serde_json::from_str::<Value>(s).ok())?;
    let fingerprint = sc.get("certificateHash").and_then(Value::as_str)?.to_string();
    let cert = match (sc.get("certificate").and_then(Value::as_str), sc.get("privateKey").and_then(Value::as_str)) {
        (Some(c), Some(k)) => Some(Arc::new((c.to_string(), k.to_string()))),
        _ => None,
    };
    let port = v.get("flutter.ls_port").and_then(Value::as_u64).map_or(PORT, |p| p as u16);
    let https = v.get("flutter.ls_https").and_then(Value::as_bool).unwrap_or(true);
    Some(Me { alias, fingerprint, port, https, cert })
}

#[derive(Clone, Debug, PartialEq)]
pub struct Device {
    pub alias: String,
    pub ip: IpAddr,
    pub port: u16,
    pub https: bool,
    pub fingerprint: String,
    pub kind: String,
    pub model: String,
}

impl Device {
    fn from_json(v: &Value, ip: IpAddr) -> Option<Device> {
        Some(Device {
            alias: v.get("alias")?.as_str()?.to_string(),
            ip,
            port: v.get("port").and_then(Value::as_u64).unwrap_or(PORT as u64) as u16,
            https: v.get("protocol").and_then(Value::as_str).unwrap_or("https") == "https",
            fingerprint: v.get("fingerprint").and_then(Value::as_str).unwrap_or_default().to_string(),
            kind: v.get("deviceType").and_then(Value::as_str).unwrap_or("").to_string(),
            model: v.get("deviceModel").and_then(Value::as_str).unwrap_or("").to_string(),
        })
    }
    fn base(&self) -> String {
        let host = match self.ip {
            IpAddr::V6(a) => format!("[{a}]"),
            a => a.to_string(),
        };
        format!("{}://{host}:{}", if self.https { "https" } else { "http" }, self.port)
    }
    /// Beschreibung für die Auswahl, z. B. „Handy · Pixel 8“.
    pub fn detail(&self) -> String {
        let kind = match self.kind.as_str() {
            "mobile" => "Handy",
            "desktop" => "PC",
            "web" => "Browser",
            "headless" | "server" => "Server",
            _ => "Gerät",
        };
        if self.model.is_empty() {
            format!("{kind} · {}", self.ip)
        } else {
            format!("{kind} · {} · {}", self.model, self.ip)
        }
    }
}

/// Zertifikat des Empfängers gegen seinen Fingerabdruck (SHA-256) prüfen; leer = bei der Suche.
#[derive(Debug)]
struct Pinned {
    fingerprint: String,
    provider: Arc<rustls::crypto::CryptoProvider>,
}

impl rustls::client::danger::ServerCertVerifier for Pinned {
    fn verify_server_cert(
        &self,
        end: &rustls::pki_types::CertificateDer<'_>,
        _: &[rustls::pki_types::CertificateDer<'_>],
        _: &rustls::pki_types::ServerName<'_>,
        _: &[u8],
        _: rustls::pki_types::UnixTime,
    ) -> Result<rustls::client::danger::ServerCertVerified, rustls::Error> {
        use sha2::Digest;
        let hash: String = sha2::Sha256::digest(end.as_ref()).iter().map(|b| format!("{b:02x}")).collect();
        if self.fingerprint.is_empty() || hash.eq_ignore_ascii_case(&self.fingerprint) {
            Ok(rustls::client::danger::ServerCertVerified::assertion())
        } else {
            Err(rustls::Error::General("Zertifikat passt nicht zum Gerät".into()))
        }
    }
    fn verify_tls12_signature(&self, m: &[u8], c: &rustls::pki_types::CertificateDer<'_>, d: &rustls::DigitallySignedStruct) -> Result<rustls::client::danger::HandshakeSignatureValid, rustls::Error> {
        rustls::crypto::verify_tls12_signature(m, c, d, &self.provider.signature_verification_algorithms)
    }
    fn verify_tls13_signature(&self, m: &[u8], c: &rustls::pki_types::CertificateDer<'_>, d: &rustls::DigitallySignedStruct) -> Result<rustls::client::danger::HandshakeSignatureValid, rustls::Error> {
        rustls::crypto::verify_tls13_signature(m, c, d, &self.provider.signature_verification_algorithms)
    }
    fn supported_verify_schemes(&self) -> Vec<rustls::SignatureScheme> {
        self.provider.signature_verification_algorithms.supported_schemes()
    }
}

/// HTTP(S)-Client für ein Gerät: Server-Zertifikat gegen den Fingerabdruck prüfen, selbst mit dem Zertifikat der
/// LocalSend-App ausweisen.
fn agent(fingerprint: &str, me: &Me, timeout: Duration) -> ureq::Agent {
    use rustls::pki_types::{pem::PemObject, CertificateDer, PrivateKeyDer};
    let provider = Arc::new(rustls::crypto::ring::default_provider());
    let b = ureq::AgentBuilder::new().timeout_connect(Duration::from_secs(4)).timeout(timeout).user_agent(UA);
    let Ok(c) = rustls::ClientConfig::builder_with_provider(provider.clone()).with_safe_default_protocol_versions() else { return b.build() };
    let c = c.dangerous().with_custom_certificate_verifier(Arc::new(Pinned { fingerprint: fingerprint.to_string(), provider }));
    let client = me.cert.as_ref().and_then(|pair| {
        let certs: Vec<CertificateDer<'static>> = CertificateDer::pem_slice_iter(pair.0.as_bytes()).filter_map(Result::ok).collect();
        let key = PrivateKeyDer::from_pem_slice(pair.1.as_bytes()).ok()?;
        (!certs.is_empty()).then_some((certs, key))
    });
    let cfg = match client {
        Some((certs, key)) => match c.clone().with_client_auth_cert(certs, key) {
            Ok(cfg) => cfg,
            Err(_) => c.with_no_client_auth(),
        },
        None => c.with_no_client_auth(),
    };
    b.tls_config(Arc::new(cfg)).build()
}

fn post_json(a: &ureq::Agent, url: &str, v: &Value) -> Result<ureq::Response, ureq::Error> {
    a.post(url).set("Content-Type", "application/json").send_string(&v.to_string())
}

/// IPv4-Adresse dieses PCs im lokalen Netz (ohne ein Paket zu senden).
fn local_ip() -> Option<Ipv4Addr> {
    let s = UdpSocket::bind("0.0.0.0:0").ok()?;
    s.connect("192.168.255.255:9").or_else(|_| s.connect("10.255.255.255:9")).ok()?;
    match s.local_addr().ok()?.ip() {
        IpAddr::V4(a) if !a.is_loopback() && !a.is_unspecified() => Some(a),
        _ => None,
    }
}

/// Laufende Gerätesuche (endet, wenn sie verworfen wird).
pub struct Discovery {
    pub devices: Arc<Mutex<Vec<Device>>>,
    pub me: Me,
    pub searching: Arc<AtomicBool>,
    stop: Arc<AtomicBool>,
}

impl Drop for Discovery {
    fn drop(&mut self) {
        self.stop.store(true, Relaxed);
    }
}

fn add(list: &Mutex<Vec<Device>>, d: Device, me: &Me, repaint: &(dyn Fn() + Send + Sync)) {
    if d.fingerprint.eq_ignore_ascii_case(&me.fingerprint) {
        return; // das eigene Gerät (bzw. die LocalSend-App auf diesem PC)
    }
    if let Ok(mut l) = list.lock() {
        match l.iter_mut().find(|x| x.fingerprint == d.fingerprint || (x.ip == d.ip && x.port == d.port)) {
            Some(x) => *x = d,
            None => l.push(d),
        }
        l.sort_by(|a, b| a.alias.to_lowercase().cmp(&b.alias.to_lowercase()));
    }
    repaint();
}

/// Geräte suchen. `repaint` wird bei jedem neuen Gerät aufgerufen.
pub fn discover(me: Me, repaint: Arc<dyn Fn() + Send + Sync>) -> Discovery {
    let devices = Arc::new(Mutex::new(Vec::new()));
    let stop = Arc::new(AtomicBool::new(false));
    let searching = Arc::new(AtomicBool::new(true));
    // Multicast: ankündigen, Ankündigungen und Antworten hören
    {
        let (d, m, s, r) = (devices.clone(), me.clone(), stop.clone(), repaint.clone());
        std::thread::spawn(move || {
            use socket2::{Domain, Protocol, Socket, Type};
            let sock = (|| -> std::io::Result<UdpSocket> {
                let so = Socket::new(Domain::IPV4, Type::DGRAM, Some(Protocol::UDP))?;
                so.set_reuse_address(true)?;
                // Port mit einer laufenden LocalSend-App teilen; sonst ein freier Port
                if so.bind(&SocketAddr::from((Ipv4Addr::UNSPECIFIED, PORT)).into()).is_err() {
                    so.bind(&SocketAddr::from((Ipv4Addr::UNSPECIFIED, 0)).into())?;
                }
                let iface = local_ip().unwrap_or(Ipv4Addr::UNSPECIFIED);
                let _ = so.join_multicast_v4(&GROUP, &iface);
                let _ = so.set_multicast_if_v4(&iface);
                so.set_multicast_ttl_v4(1)?;
                so.set_read_timeout(Some(Duration::from_millis(300)))?;
                Ok(so.into())
            })();
            let Ok(sock) = sock else { return };
            let hello = m.json(Some(true)).to_string();
            let target = SocketAddr::from((GROUP, PORT));
            let mut buf = [0u8; 8192];
            let start = std::time::Instant::now();
            let mut sent = 0;
            while !s.load(Relaxed) {
                let due = [0u64, 1000, 3000];
                if sent < due.len() && start.elapsed().as_millis() as u64 >= due[sent] {
                    let _ = sock.send_to(hello.as_bytes(), target);
                    sent += 1;
                }
                let Ok((n, from)) = sock.recv_from(&mut buf) else { continue };
                let Some(v) = serde_json::from_slice::<Value>(&buf[..n]).ok() else { continue };
                if let Some(dev) = Device::from_json(&v, from.ip()) {
                    add(&d, dev, &m, r.as_ref());
                }
            }
        });
    }
    // Alle Adressen im Netz direkt fragen (findet Geräte auch ohne Multicast-Antwort)
    {
        let (d, m, s, busy, r) = (devices.clone(), me.clone(), stop.clone(), searching.clone(), repaint.clone());
        std::thread::spawn(move || {
            if let Some(ip) = local_ip() {
                let o = ip.octets();
                let hosts: Vec<Ipv4Addr> = (1..=254u8).map(|x| Ipv4Addr::new(o[0], o[1], o[2], x)).filter(|a| *a != ip).collect();
                for chunk in hosts.chunks(48) {
                    if s.load(Relaxed) {
                        break;
                    }
                    let hs: Vec<_> = chunk
                        .iter()
                        .map(|a| {
                            let (a, d, m, r) = (*a, d.clone(), m.clone(), r.clone());
                            std::thread::spawn(move || {
                                if TcpStream::connect_timeout(&SocketAddr::from((a, PORT)), Duration::from_millis(300)).is_err() {
                                    return;
                                }
                                for https in [true, false] {
                                    let url = format!("{}://{a}:{PORT}/api/localsend/v2/register", if https { "https" } else { "http" });
                                    if let Ok(resp) = post_json(&agent("", &m, Duration::from_secs(3)), &url, &m.json(None)) {
                                        if let Some(mut dev) = resp.into_string().ok().and_then(|t| serde_json::from_str::<Value>(&t).ok()).and_then(|v| Device::from_json(&v, IpAddr::V4(a))) {
                                            dev.https = https;
                                            dev.port = PORT;
                                            add(&d, dev, &m, r.as_ref());
                                        }
                                        return;
                                    }
                                }
                            })
                        })
                        .collect();
                    for h in hs {
                        let _ = h.join();
                    }
                }
            }
            // Multicast-Antworten kommen meist in den ersten Sekunden
            std::thread::sleep(Duration::from_secs(2));
            busy.store(false, Relaxed);
            r();
        });
    }
    Discovery { devices, me, searching, stop }
}

/// Lesestrom mit Fortschritt und Abbruch.
struct Counted<R> {
    inner: R,
    prog: Arc<AtomicU64>,
    cancel: Arc<AtomicBool>,
}

impl<R: Read> Read for Counted<R> {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        if self.cancel.load(Relaxed) {
            return Err(std::io::Error::new(std::io::ErrorKind::Interrupted, "abgebrochen"));
        }
        let n = self.inner.read(buf)?;
        self.prog.fetch_add(n as u64, Relaxed);
        Ok(n)
    }
}

fn enc(s: &str) -> String {
    s.bytes().map(|b| if b.is_ascii_alphanumeric() || b"-_.~".contains(&b) { (b as char).to_string() } else { format!("%{b:02X}") }).collect()
}

fn mime(p: &Path) -> &'static str {
    match crate::convert::class_of(p) {
        crate::convert::Class::Image => "image/*",
        crate::convert::Class::Video => "video/*",
        crate::convert::Class::Audio => "audio/*",
        crate::convert::Class::Text => "text/plain",
        _ => "application/octet-stream",
    }
}

/// Dateien an ein Gerät senden. Der Empfänger bestätigt auf seinem Gerät (bis 3 Minuten).
pub fn send(dev: &Device, me: &Me, files: &[PathBuf], prog: Arc<AtomicU64>, cancel: Arc<AtomicBool>) -> Result<String, String> {
    let mut meta = serde_json::Map::new();
    for (i, p) in files.iter().enumerate() {
        let md = std::fs::metadata(p).map_err(|e| format!("{}: {e}", p.display()))?;
        let id = format!("f{i}");
        meta.insert(id.clone(), json!({ "id": id, "fileName": p.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default(), "size": md.len(), "fileType": mime(p), "sha256": null, "preview": null }));
    }
    let a = agent(&dev.fingerprint, me, Duration::from_secs(180));
    let resp = match post_json(&a, &format!("{}/api/localsend/v2/prepare-upload", dev.base()), &json!({ "info": me.json(None), "files": meta })) {
        Ok(r) if r.status() == 204 => return Ok(format!("{} hatte die Dateien schon", dev.alias)),
        Ok(r) => r,
        Err(ureq::Error::Status(403, _)) => return Err(format!("{} hat abgelehnt", dev.alias)),
        Err(ureq::Error::Status(401, _)) => return Err(format!("{} verlangt eine PIN – in LocalSend auf dem Gerät „PIN verlangen“ ausschalten", dev.alias)),
        Err(ureq::Error::Status(409, _)) => return Err(format!("{} ist gerade mit einer anderen Übertragung beschäftigt", dev.alias)),
        Err(ureq::Error::Status(c, _)) => return Err(format!("{}: Fehler {c}", dev.alias)),
        Err(ureq::Error::Transport(t)) => return Err(format!("{} nicht erreichbar: {t}", dev.alias)),
    };
    let v: Value = serde_json::from_str(&resp.into_string().map_err(|e| e.to_string())?).map_err(|e| e.to_string())?;
    let session = v.get("sessionId").and_then(Value::as_str).ok_or("Antwort ohne Sitzung")?.to_string();
    let tokens = v.get("files").and_then(Value::as_object).cloned().unwrap_or_default();
    let up = agent(&dev.fingerprint, me, Duration::from_secs(3600));
    let mut n = 0;
    for (i, p) in files.iter().enumerate() {
        let id = format!("f{i}");
        let Some(token) = tokens.get(&id).and_then(Value::as_str) else { continue }; // vom Empfänger abgewählt
        let f = std::fs::File::open(p).map_err(|e| e.to_string())?;
        let len = f.metadata().map_err(|e| e.to_string())?.len();
        let url = format!("{}/api/localsend/v2/upload?sessionId={}&fileId={id}&token={}", dev.base(), enc(&session), enc(token));
        let r = up.post(&url).set("Content-Type", "application/octet-stream").set("Content-Length", &len.to_string()).send(Counted { inner: BufReader::with_capacity(1 << 16, f), prog: prog.clone(), cancel: cancel.clone() });
        if cancel.load(Relaxed) {
            let _ = agent(&dev.fingerprint, me, Duration::from_secs(3)).post(&format!("{}/api/localsend/v2/cancel?sessionId={}", dev.base(), enc(&session))).call();
            return Err("abgebrochen".into());
        }
        r.map_err(|e| format!("{}: {e}", dev.alias))?;
        n += 1;
    }
    Ok(format!("{n} Datei(en) an {} gesendet", dev.alias))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Handshake mit einem echten Gerät (nur „register“, sendet nichts): FOLDEX_LS_IP=<adresse> cargo test -- --ignored
    #[test]
    #[ignore]
    fn handshake_with_device() {
        let ip = std::env::var("FOLDEX_LS_IP").expect("FOLDEX_LS_IP setzen");
        let me = identity().expect("LocalSend eingerichtet");
        let r = post_json(&agent("", &me, Duration::from_secs(5)), &format!("https://{ip}:{PORT}/api/localsend/v2/register"), &me.json(None));
        match r {
            Ok(resp) => println!("OK {}: {}", resp.status(), resp.into_string().unwrap_or_default().chars().take(200).collect::<String>()),
            Err(e) => panic!("Handshake fehlgeschlagen: {e}"),
        }
    }

    /// Mit installierter LocalSend-App: Zertifikat und Schlüssel lassen sich lesen, der Fingerabdruck passt dazu,
    /// und ein Client mit Zertifikat wird gebaut.
    #[test]
    fn real_identity_client_cert() {
        use rustls::pki_types::{pem::PemObject, CertificateDer, PrivateKeyDer};
        use sha2::Digest;
        let Some(me) = identity() else { return };
        let pair = me.cert.clone().expect("Zertifikat im Sicherheitskontext");
        let cert = CertificateDer::from_pem_slice(pair.0.as_bytes()).expect("Zertifikat lesbar");
        let hash: String = sha2::Sha256::digest(cert.as_ref()).iter().map(|b| format!("{b:02x}")).collect();
        assert!(hash.eq_ignore_ascii_case(&me.fingerprint), "Fingerabdruck passt nicht zum Zertifikat");
        let key = PrivateKeyDer::from_pem_slice(pair.1.as_bytes()).expect("Schlüssel lesbar");
        let provider = Arc::new(rustls::crypto::ring::default_provider());
        let cfg = rustls::ClientConfig::builder_with_provider(provider.clone())
            .with_safe_default_protocol_versions()
            .unwrap()
            .dangerous()
            .with_custom_certificate_verifier(Arc::new(Pinned { fingerprint: String::new(), provider }))
            .with_client_auth_cert(vec![cert], key);
        assert!(cfg.is_ok(), "Client-Zertifikat wird nicht angenommen");
    }

    #[test]
    fn identity_and_device() {
        let cfg = json!({ "flutter.ls_alias": "Tidy Tomato", "flutter.ls_security_context": json!({ "certificateHash": "abc123" }).to_string() }).to_string();
        let me = parse_identity(&cfg).unwrap();
        assert_eq!((me.alias.as_str(), me.fingerprint.as_str(), me.port, me.https), ("Tidy Tomato", "abc123", 53317, true));
        assert_eq!(me.json(Some(true))["announce"], json!(true));
        assert_eq!(me.json(None)["protocol"], json!("https"));
        let d = Device::from_json(&json!({ "alias": "Handy", "port": 53317, "protocol": "https", "fingerprint": "ff", "deviceType": "mobile", "deviceModel": "Pixel" }), "192.168.1.5".parse().unwrap()).unwrap();
        assert_eq!(d.base(), "https://192.168.1.5:53317");
        assert_eq!(d.detail(), "Handy · Pixel · 192.168.1.5");
        // das eigene Gerät wird nicht gelistet
        let list = Mutex::new(Vec::new());
        add(&list, Device { fingerprint: "ABC123".into(), ..d.clone() }, &me, &|| {});
        add(&list, d, &me, &|| {});
        assert_eq!(list.lock().unwrap().len(), 1);
    }
}
