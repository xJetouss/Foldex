//! Kompakter, memory-mapped Dateiindex.
//!
//! Dateilayout (native endian):
//!   Header (32 B): "FCIDX002", n:u64, names_len:u64, pad
//!   offs   : u32 * (n+1)   Offset in `names` (Bit 31 = Ordner), letzter Eintrag = Sentinel
//!   parent : u32 * n       Elterneintrag
//!   names  : names_len B   ASCII-kleingeschriebene Namen, je durch '\n' beendet
//!   mask   : ceil(names_len/8) B   Bit je Byte: war ursprünglich ein ASCII-Großbuchstabe
//!
//! Der Index liegt nach dem Bau nur noch als Datei auf der Platte und wird per mmap
//! eingeblendet: das Betriebssystem hält nur die tatsächlich benutzten Seiten im RAM und kann sie
//! jederzeit verwerfen. Beim nächsten Start ist er sofort wieder da.

use memchr::memmem;
use memmap2::Mmap;
use rayon::prelude::*;
use std::{
    fs::{self, File},
    io::{self, BufWriter, Write},
    path::{Path, PathBuf},
    sync::atomic::{AtomicBool, AtomicUsize, Ordering},
};

const MAGIC: &[u8; 8] = b"FCIDX002";
const HDR: usize = 32;
const DIRBIT: u32 = 0x8000_0000;
const OFFMASK: u32 = 0x7FFF_FFFF;
const MAX_CANDIDATES: usize = 300_000;

pub struct Index {
    map: Mmap,
    n: usize,
    names_len: usize,
    par_at: usize,
    names_at: usize,
    mask_at: usize,
    pub file: PathBuf,
    pub built: std::time::SystemTime,
}

fn bad(msg: &str) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, msg.to_string())
}

impl Index {
    pub fn open(path: &Path) -> io::Result<Index> {
        let f = File::open(path)?;
        let map = unsafe { Mmap::map(&f)? };
        if map.len() < HDR || &map[..8] != MAGIC {
            return Err(bad("kein Foldex-Index"));
        }
        let n = u64::from_ne_bytes(map[8..16].try_into().unwrap());
        let nl = u64::from_ne_bytes(map[16..24].try_into().unwrap());
        if n == 0 || n >= (1 << 31) || nl >= (1 << 31) {
            return Err(bad("ungültige Größe"));
        }
        let (n, nl) = (n as usize, nl as usize);
        let par_at = HDR + (n + 1) * 4;
        let names_at = par_at + n * 4;
        let mask_at = names_at + nl;
        let total = mask_at + nl.div_ceil(8);
        if total != map.len() {
            return Err(bad("Index unvollständig"));
        }
        let built = f
            .metadata()
            .and_then(|m| m.modified())
            .unwrap_or(std::time::UNIX_EPOCH);
        Ok(Index {
            map,
            n,
            names_len: nl,
            par_at,
            names_at,
            mask_at,
            file: path.to_path_buf(),
            built,
        })
    }

    pub fn len(&self) -> usize {
        self.n
    }

    fn offs(&self) -> &[u32] {
        // Sicher: mmap ist seitenausgerichtet, HDR ist durch 4 teilbar, Länge wurde in open() geprüft.
        unsafe { std::slice::from_raw_parts(self.map.as_ptr().add(HDR) as *const u32, self.n + 1) }
    }
    fn parents(&self) -> &[u32] {
        unsafe {
            std::slice::from_raw_parts(self.map.as_ptr().add(self.par_at) as *const u32, self.n)
        }
    }
    fn names(&self) -> &[u8] {
        &self.map[self.names_at..self.names_at + self.names_len]
    }
    fn mask(&self) -> &[u8] {
        &self.map[self.mask_at..]
    }

    fn range(&self, i: usize) -> (usize, usize) {
        let o = self.offs();
        ((o[i] & OFFMASK) as usize, (o[i + 1] & OFFMASK) as usize - 1)
    }

    pub fn is_dir(&self, i: usize) -> bool {
        self.offs()[i] & DIRBIT != 0
    }

    /// Kleingeschriebener Name (Bytes) – für Vergleiche.
    pub fn lower(&self, i: usize) -> &[u8] {
        let (s, e) = self.range(i);
        &self.names()[s..e]
    }

    /// Originalname mit Groß-/Kleinschreibung.
    pub fn name(&self, i: usize) -> String {
        let (s, e) = self.range(i);
        let mut v = self.names()[s..e].to_vec();
        let mask = self.mask();
        for (k, b) in v.iter_mut().enumerate() {
            let pos = s + k;
            if (mask[pos / 8] >> (pos % 8)) & 1 == 1 {
                *b -= 32;
            }
        }
        String::from_utf8_lossy(&v).into_owned()
    }

    pub fn path(&self, i: usize) -> PathBuf {
        let mut parts = Vec::new();
        let mut c = i as u32;
        let par = self.parents();
        while c != u32::MAX && parts.len() < 512 {
            parts.push(self.name(c as usize));
            c = par[c as usize];
        }
        let mut p = PathBuf::new();
        for n in parts.iter().rev() {
            p.push(n);
        }
        p
    }

    fn in_subtree(&self, i: usize, d: u32) -> bool {
        let par = self.parents();
        let mut c = par[i];
        while c != u32::MAX {
            if c == d {
                return true;
            }
            if c < d {
                return false; // Vorfahren haben immer kleinere IDs
            }
            c = par[c as usize];
        }
        false
    }

    /// ID des Ordners `p` (muss unterhalb der Wurzel dieses Index liegen).
    /// Nutzt, dass `parent[]` ab Eintrag 1 aufsteigend sortiert ist (Breitensuche beim Bau).
    pub fn find_dir(&self, p: &Path) -> Option<usize> {
        let root = self.path(0).to_string_lossy().to_ascii_lowercase();
        let root = root.trim_end_matches(['/', '\\']);
        let full = p.to_string_lossy().to_ascii_lowercase();
        let rest = full.strip_prefix(root)?;
        if !rest.is_empty() && !rest.starts_with(['/', '\\']) {
            return None;
        }
        let par = self.parents();
        let mut cur = 0usize;
        for comp in rest.split(['/', '\\']).filter(|c| !c.is_empty()) {
            let lo = 1 + par[1..].partition_point(|&x| (x as usize) < cur);
            let hi = 1 + par[1..].partition_point(|&x| (x as usize) <= cur);
            cur = (lo..hi).find(|&j| self.is_dir(j) && self.lower(j) == comp.as_bytes())?;
        }
        Some(cur)
    }

    fn accept(&self, i: usize, q: &Query, finders: &[memmem::Finder]) -> Option<i32> {
        if let Some(d) = q.scope {
            if !self.in_subtree(i, d) {
                return None;
            }
        }
        let ln = self.lower(i);
        let mut score = q.score_name(ln, self.is_dir(i), finders)?;
        if !q.ptes.is_empty() {
            let p = self
                .path(i)
                .to_string_lossy()
                .to_ascii_lowercase()
                .replace('\\', "/");
            let pb = p.as_bytes();
            if !q.ptes.iter().all(|t| memmem::find(pb, t).is_some()) {
                return None;
            }
            score += 0;
        }
        Some(score)
    }

    /// Gerankte Suche. Rückgabe: Treffer-IDs (beste zuerst), `capped` = Kandidatenlimit erreicht.
    pub fn search(&self, q: &Query, limit: usize) -> SearchResult {
        let n = self.n;
        let offs = self.offs();
        let finders: Vec<memmem::Finder> = q.terms.iter().map(|t| memmem::Finder::new(t)).collect();
        let mut cand: Vec<(i32, u32)> = Vec::new();
        let mut capped = false;

        // „Treiber“: der längste Suchbegriff ist am selektivsten – nur er wird über den ganzen Puffer gescannt.
        let driver = q
            .terms
            .iter()
            .enumerate()
            .max_by_key(|(_, t)| t.len())
            .map(|(i, _)| i);
        match driver {
            Some(d) => {
                let mut last = usize::MAX;
                for pos in finders[d].find_iter(self.names()) {
                    let i = offs[..n].partition_point(|&o| ((o & OFFMASK) as usize) <= pos) - 1;
                    if i == last || i == 0 {
                        continue;
                    }
                    last = i;
                    if let Some(sc) = self.accept(i, q, &finders) {
                        cand.push((sc, i as u32));
                        if cand.len() >= MAX_CANDIDATES {
                            capped = true;
                            break;
                        }
                    }
                }
            }
            None => {
                for i in 1..n {
                    if let Some(sc) = self.accept(i, q, &finders) {
                        cand.push((sc, i as u32));
                        if cand.len() >= MAX_CANDIDATES {
                            capped = true;
                            break;
                        }
                    }
                }
            }
        }

        // Unscharfer Fallback (Buchstaben in Reihenfolge, z. B. "rpt" -> "report"), nur wenn nichts gefunden wurde.
        if cand.is_empty() && q.terms.len() == 1 && q.terms[0].len() >= 2 && q.is_plain() {
            let t = &q.terms[0];
            for i in 1..n {
                let ln = self.lower(i);
                if let Some(d) = q.scope {
                    if !self.in_subtree(i, d) {
                        continue;
                    }
                }
                if let Some(span) = subseq_span(ln, t) {
                    if span <= t.len() * 2 + 2 {
                        let sc = 200 - span as i32 * 10 - ln.len().min(100) as i32;
                        cand.push((sc, i as u32));
                        if cand.len() >= 20_000 {
                            capped = true;
                            break;
                        }
                    }
                }
            }
        }

        let total = cand.len();
        let cmp = |a: &(i32, u32), b: &(i32, u32)| b.0.cmp(&a.0).then(a.1.cmp(&b.1));
        if cand.len() > limit {
            cand.select_nth_unstable_by(limit, cmp);
            cand.truncate(limit);
        }
        cand.sort_unstable_by(cmp);
        SearchResult {
            ids: cand.iter().map(|&(_, i)| i).collect(),
            scores: cand.iter().map(|&(s, _)| s).collect(),
            total,
            capped,
        }
    }
}

pub struct SearchResult {
    pub ids: Vec<u32>,
    pub scores: Vec<i32>,
    pub total: usize,
    pub capped: bool,
}

fn is_sep(b: u8) -> bool {
    matches!(b, b'.' | b'_' | b'-' | b' ' | b'(' | b'[')
}

fn subseq_span(hay: &[u8], needle: &[u8]) -> Option<usize> {
    let mut it = hay.iter().enumerate();
    let mut first = None;
    let mut last = 0;
    for &c in needle {
        loop {
            let (p, &h) = it.next()?;
            if h == c {
                first.get_or_insert(p);
                last = p;
                break;
            }
        }
    }
    first.map(|f| last - f + 1)
}

// ───────────────────────────── Abfrage-Syntax ─────────────────────────────
//   wort1 wort2     alle Wörter müssen im Namen vorkommen (Reihenfolge egal)
//   !wort           Name darf das Wort nicht enthalten
//   ext:pdf,docx    oder  *.pdf   nur diese Endungen
//   is:dir | is:file
//   ordner/teil     Begriff mit / oder \ wird gegen den ganzen Pfad geprüft
//   inhalt:wort     Dateiinhalt muss das Wort enthalten (auch content:/text:, mit "…" für Leerzeichen)

#[derive(Default)]
pub struct Query {
    pub terms: Vec<Vec<u8>>,
    pub negs: Vec<Vec<u8>>,
    pub exts: Vec<Vec<u8>>,
    pub ptes: Vec<Vec<u8>>,
    pub dir: Option<bool>,
    /// Nur Einträge unterhalb dieses Ordners (ID im Index).
    pub scope: Option<u32>,
    /// Inhaltssuche: alle Begriffe (kleingeschrieben) müssen im Dateiinhalt vorkommen
    pub content: Vec<Vec<u8>>,
}

impl Query {
    pub fn finders(&self) -> Vec<memmem::Finder<'_>> {
        self.terms.iter().map(|t| memmem::Finder::new(t)).collect()
    }

    /// Bewertet einen (kleingeschriebenen) Namen. `None` = passt nicht. Wird vom Index UND von der Live-Ordnersuche benutzt.
    pub fn score_name(&self, ln: &[u8], dir: bool, finders: &[memmem::Finder]) -> Option<i32> {
        if let Some(d) = self.dir {
            if d != dir {
                return None;
            }
        }
        let mut score = 0i32;
        for (t, f) in self.terms.iter().zip(finders) {
            let p = f.find(ln)?;
            score += if ln == t.as_slice() {
                1000
            } else if p == 0 {
                800
            } else if is_sep(ln[p - 1]) {
                600
            } else {
                400
            };
        }
        if !self.terms.is_empty() {
            score /= self.terms.len() as i32;
        }
        for t in &self.negs {
            if memmem::find(ln, t).is_some() {
                return None;
            }
        }
        if !self.exts.is_empty() {
            let ext = ln.iter().rposition(|&b| b == b'.').map(|p| &ln[p + 1..]);
            match ext {
                Some(x) if self.exts.iter().any(|e| e.as_slice() == x) => {}
                _ => return None,
            }
        }
        score -= ln.len().min(200) as i32;
        if dir {
            score += 30;
        }
        Some(score)
    }

    pub fn parse(s: &str) -> Query {
        let mut q = Query::default();
        for tok in tokens(s) {
            let low = tok.to_ascii_lowercase();
            if let Some(v) = ["inhalt:", "content:", "text:"].iter().find_map(|p| low.strip_prefix(p)) {
                if !v.is_empty() {
                    q.content.push(v.as_bytes().to_vec());
                }
            } else if let Some(v) = low.strip_prefix("ext:") {
                q.exts.extend(
                    v.split(',')
                        .map(|e| e.trim_start_matches('.').as_bytes().to_vec())
                        .filter(|e| !e.is_empty()),
                );
            } else if let Some(v) = low.strip_prefix("*.") {
                if !v.is_empty() && !v.contains('*') {
                    q.exts.push(v.as_bytes().to_vec());
                }
            } else if low == "is:dir" || low == "is:folder" {
                q.dir = Some(true);
            } else if low == "is:file" {
                q.dir = Some(false);
            } else if let Some(v) = low.strip_prefix('!') {
                if !v.is_empty() {
                    q.negs.push(v.as_bytes().to_vec());
                }
            } else if low.contains('/') || low.contains('\\') {
                q.ptes.push(low.replace('\\', "/").into_bytes());
            } else {
                q.terms.push(low.into_bytes());
            }
        }
        q
    }
    pub fn has_path_terms(&self) -> bool {
        !self.ptes.is_empty()
    }
    pub fn is_empty(&self) -> bool {
        self.terms.is_empty() && self.exts.is_empty() && self.dir.is_none() && self.content.is_empty()
    }
    fn is_plain(&self) -> bool {
        self.negs.is_empty() && self.exts.is_empty() && self.ptes.is_empty() && self.dir.is_none()
    }
    /// Sucht auch im Dateiinhalt (läuft immer live, der Index kennt nur Namen).
    pub fn has_content(&self) -> bool {
        !self.content.is_empty()
    }
}

/// Zerlegt die Eingabe an Leerzeichen; Anführungszeichen halten Wörter zusammen (`inhalt:"zwei wörter"`).
fn tokens(s: &str) -> Vec<String> {
    let (mut out, mut cur, mut quoted) = (Vec::new(), String::new(), false);
    for c in s.chars() {
        match c {
            '"' => quoted = !quoted,
            c if c.is_whitespace() && !quoted => {
                if !cur.is_empty() {
                    out.push(std::mem::take(&mut cur));
                }
            }
            c => cur.push(c),
        }
    }
    if !cur.is_empty() {
        out.push(cur);
    }
    out
}

/// Prüft den Inhalt einer Datei auf alle `terms` (ASCII-unabhängig von Groß-/Kleinschreibung).
/// Binärdateien und Dateien über 32 MB werden übersprungen. Ergebnis: (Zeilennummer, Ausschnitt) des ersten Treffers.
pub fn content_match(p: &Path, terms: &[Vec<u8>]) -> Option<(usize, String)> {
    let md = fs::metadata(p).ok()?;
    if md.len() == 0 || md.len() > 32 * 1024 * 1024 {
        return None;
    }
    let f = File::open(p).ok()?;
    // große Dateien einblenden statt lesen
    let map = unsafe { Mmap::map(&f).ok()? };
    let data: &[u8] = &map;
    if data[..data.len().min(8192)].contains(&0) {
        return None;
    }
    let low = data.to_ascii_lowercase();
    let mut first = usize::MAX;
    for t in terms {
        first = first.min(memmem::find(&low, t)?);
    }
    let line = 1 + memchr::memchr_iter(b'\n', &low[..first]).count();
    let start = low[..first].iter().rposition(|&b| b == b'\n').map_or(0, |i| i + 1);
    let end = low[first..].iter().position(|&b| b == b'\n').map_or(low.len(), |i| first + i);
    let text = String::from_utf8_lossy(&data[start..end]);
    let text = text.trim();
    let snippet: String = if text.chars().count() > 160 {
        // Ausschnitt um den Treffer herum
        let at = String::from_utf8_lossy(&data[start..first]).trim_start().chars().count();
        text.chars().skip(at.saturating_sub(50)).take(160).collect()
    } else {
        text.to_string()
    };
    Some((line, snippet))
}

// ───────────────────────────── Bauen ─────────────────────────────

struct Builder {
    offs: Vec<u32>,
    parent: Vec<u32>,
    names: Vec<u8>,
    mask: Vec<u8>,
}

impl Builder {
    fn push(&mut self, name: &[u8], parent: u32, dir: bool) -> u32 {
        let id = self.offs.len() as u32;
        self.offs
            .push(self.names.len() as u32 | if dir { DIRBIT } else { 0 });
        self.parent.push(parent);
        for &b in name {
            let pos = self.names.len();
            if b.is_ascii_uppercase() {
                let byte = pos / 8;
                if self.mask.len() <= byte {
                    self.mask.resize(byte + 1, 0);
                }
                self.mask[byte] |= 1 << (pos % 8);
                self.names.push(b + 32);
            } else {
                self.names.push(b);
            }
        }
        self.names.push(b'\n');
        id
    }

    fn finish(mut self, dest: &Path) -> io::Result<Index> {
        if let Some(dir) = dest.parent() {
            fs::create_dir_all(dir)?;
        }
        let n = self.offs.len();
        let nl = self.names.len();
        self.mask.resize(nl.div_ceil(8), 0);
        self.offs.push(nl as u32); // Sentinel
        {
            let mut w = BufWriter::with_capacity(1 << 20, File::create(dest)?);
            w.write_all(MAGIC)?;
            w.write_all(&(n as u64).to_ne_bytes())?;
            w.write_all(&(nl as u64).to_ne_bytes())?;
            w.write_all(&[0u8; 8])?;
            w.write_all(as_bytes(&self.offs))?;
            w.write_all(as_bytes(&self.parent))?;
            w.write_all(&self.names)?;
            w.write_all(&self.mask)?;
            w.flush()?;
        }
        drop(self); // Vecs freigeben – ab jetzt liegt alles nur noch im mmap
        Index::open(dest)
    }
}

fn as_bytes(v: &[u32]) -> &[u8] {
    unsafe { std::slice::from_raw_parts(v.as_ptr() as *const u8, v.len() * 4) }
}

/// Ordner, deren Inhalt weder indexiert noch live durchsucht wird (System- und Cache-Ordner). Der Ordner selbst
/// erscheint trotzdem in den Treffern. `at_root`: Linux-Pseudodateisysteme nur direkt unter „/“.
pub fn excluded_dir(lower: &str, at_root: bool) -> bool {
    matches!(lower, "$recycle.bin" | "system volume information" | "$windows.~bt" | "$windows.~ws" | "windows.old" | "node_modules" | ".git" | "target")
        || (at_root && matches!(lower, "proc" | "sys" | "dev" | "run"))
}

/// Liegt `p` innerhalb eines ausgeschlossenen Ordners? (Dann hat der Index dort keine Einträge.)
pub fn inside_excluded(p: &Path) -> bool {
    p.components().any(|c| matches!(c, std::path::Component::Normal(n) if excluded_dir(&n.to_string_lossy().to_ascii_lowercase(), false)))
}

struct Listing {
    pid: u32,
    names: Vec<u8>,
    kinds: Vec<u8>, // 0 Datei, 1 Ordner (hineingehen), 2 Ordner (überspringen)
    subdirs: Vec<PathBuf>,
}

fn list_dir(pid: u32, p: &Path, at_root: bool) -> Listing {
    let mut l = Listing {
        pid,
        names: Vec::new(),
        kinds: Vec::new(),
        subdirs: Vec::new(),
    };
    if let Ok(rd) = fs::read_dir(p) {
        for e in rd.flatten() {
            let fname = e.file_name();
            let name = fname.to_string_lossy();
            if name.contains('\n') {
                continue;
            }
            let is_dir = e.file_type().map(|t| t.is_dir()).unwrap_or(false);
            // Versteckte und System-Einträge (Punkt-Namen, Attribut „versteckt“) werden immer mit indexiert –
            // auch beliebig tief verschachtelt. Nur der Inhalt von System- und Cache-Ordnern bleibt außen vor.
            let kind = if !is_dir {
                0
            } else if excluded_dir(&name.to_ascii_lowercase(), at_root) {
                2
            } else {
                1
            };
            l.names.extend_from_slice(name.as_bytes());
            l.names.push(b'\n');
            l.kinds.push(kind);
            if kind == 1 {
                l.subdirs.push(p.join(&*fname));
            }
        }
    }
    l
}

/// Baut den Index parallel (ebenenweise, in kleinen Blöcken → geringe Spitzen-RAM-Last) und legt ihn in `dest` ab.
pub fn build(
    root: &Path,
    dest: &Path,
    progress: &AtomicUsize,
    cancel: &AtomicBool,
) -> io::Result<Index> {
    let mut b = Builder {
        offs: Vec::new(),
        parent: Vec::new(),
        names: Vec::new(),
        mask: Vec::new(),
    };
    b.push(root.to_string_lossy().as_bytes(), u32::MAX, true);
    let threads = std::thread::available_parallelism()
        .map(|n| n.get())
        .unwrap_or(4)
        .clamp(2, 8);
    let pool = rayon::ThreadPoolBuilder::new()
        .num_threads(threads)
        .stack_size(512 * 1024)
        .build()
        .map_err(io::Error::other)?;
    let root_parent_is_none = root.parent().is_none();
    let mut level: Vec<(u32, PathBuf)> = vec![(0, root.to_path_buf())];
    'outer: while !level.is_empty() {
        let mut next: Vec<(u32, PathBuf)> = Vec::new();
        for chunk in level.chunks(1024) {
            if cancel.load(Ordering::Relaxed) {
                return Err(io::Error::new(io::ErrorKind::Interrupted, "abgebrochen"));
            }
            let at_root = root_parent_is_none && chunk.iter().any(|(id, _)| *id == 0);
            let found: Vec<Listing> = pool.install(|| {
                chunk
                    .par_iter()
                    .map(|(pid, p)| list_dir(*pid, p, at_root && *pid == 0))
                    .collect()
            });
            for l in found {
                let mut sub = l.subdirs.into_iter();
                for (name, kind) in l.names.split(|&c| c == b'\n').zip(l.kinds.iter()) {
                    let id = b.push(name, l.pid, *kind != 0);
                    if *kind == 1 {
                        if let Some(p) = sub.next() {
                            next.push((id, p));
                        }
                    }
                }
                if b.names.len() > 0x7000_0000 {
                    break 'outer; // 32-Bit-Offsets voll – Index wird abgeschnitten
                }
            }
            progress.store(b.offs.len(), Ordering::Relaxed);
        }
        level = next;
    }
    drop(pool);
    b.finish(dest)
}

// ───────────────────────────── Cache-Verwaltung ─────────────────────────────

pub fn cache_dir() -> PathBuf {
    dirs::cache_dir()
        .unwrap_or_else(std::env::temp_dir)
        .join("foldex")
}

fn root_key(root: &Path) -> String {
    use std::hash::{Hash, Hasher};
    let mut h = std::collections::hash_map::DefaultHasher::new();
    root.hash(&mut h);
    format!("{:016x}", h.finish())
}

pub fn new_cache_file(root: &Path) -> PathBuf {
    let ms = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis())
        .unwrap_or(0);
    cache_dir().join(format!("idx-{}-{:016}.fci", root_key(root), ms))
}

pub fn latest_cache(root: &Path) -> Option<PathBuf> {
    let prefix = format!("idx-{}-", root_key(root));
    fs::read_dir(cache_dir())
        .ok()?
        .flatten()
        .filter(|e| e.file_name().to_string_lossy().starts_with(&prefix))
        .map(|e| e.path())
        .max()
}

/// Löscht alle älteren Indexdateien dieser Wurzel.
pub fn cleanup(root: &Path, keep: &Path) {
    let prefix = format!("idx-{}-", root_key(root));
    if let Ok(rd) = fs::read_dir(cache_dir()) {
        for e in rd.flatten() {
            if e.file_name().to_string_lossy().starts_with(&prefix) && e.path() != keep {
                let _ = fs::remove_file(e.path());
            }
        }
    }
}

#[cfg(test)]
mod content_tests {
    use super::*;

    #[test]
    fn content_query_and_match() {
        let q = Query::parse(r#"bericht inhalt:"Hallo Welt" ext:txt"#);
        assert_eq!(q.content, vec![b"hallo welt".to_vec()]);
        assert_eq!(q.terms, vec![b"bericht".to_vec()]);
        assert!(q.has_content() && !q.is_empty());
        assert!(!Query::parse("inhalt:x").is_empty());
        let d = std::env::temp_dir().join(format!("fx-content-{}", std::process::id()));
        let _ = fs::remove_dir_all(&d);
        fs::create_dir_all(d.join(".versteckt/tiefer")).unwrap();
        fs::write(d.join(".versteckt/tiefer/notiz.txt"), "erste Zeile\nzweite: HALLO Welt hier\n").unwrap();
        fs::write(d.join("bild.bin"), b"hallo welt\0\0binary").unwrap();
        let m = content_match(&d.join(".versteckt/tiefer/notiz.txt"), &q.content).unwrap();
        assert_eq!(m.0, 2);
        assert!(m.1.contains("HALLO Welt"));
        assert!(content_match(&d.join("bild.bin"), &q.content).is_none(), "Binärdateien überspringen");
        // Index: versteckte, verschachtelte Ordner sind drin; Cache-Ordner nur als Eintrag
        fs::create_dir_all(d.join("node_modules/pkg")).unwrap();
        fs::write(d.join("node_modules/pkg/notiz.txt"), "x").unwrap();
        let ix = build(&d, &d.join("..").join(format!("fx-content-{}.idx", std::process::id())), &AtomicUsize::new(0), &AtomicBool::new(false)).unwrap();
        let r = ix.search(&Query::parse("notiz"), 100);
        let paths: Vec<PathBuf> = r.ids.iter().map(|&i| ix.path(i as usize)).collect();
        assert!(paths.iter().any(|p| p.ends_with(".versteckt/tiefer/notiz.txt")), "{paths:?}");
        assert!(!paths.iter().any(|p| p.to_string_lossy().contains("node_modules")), "{paths:?}");
        assert!(inside_excluded(&d.join("node_modules/pkg")));
        assert!(!inside_excluded(&d.join(".versteckt")));
        let f = ix.file.clone();
        drop(ix);
        let _ = fs::remove_file(f);
        let _ = fs::remove_dir_all(&d);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tree() -> (PathBuf, PathBuf) {
        let base = std::env::temp_dir().join(format!("fc-test-{}", std::process::id()));
        let _ = fs::remove_dir_all(&base);
        let root = base.join("Root");
        fs::create_dir_all(root.join("Photos/Holiday")).unwrap();
        fs::create_dir_all(root.join("Docs")).unwrap();
        for f in [
            "Report 2024.PDF",
            "notes.txt",
            "Photos/IMG_001.jpg",
            "Photos/Holiday/Beach.JPG",
            "Docs/Annual-Report.docx",
            "Docs/report.txt",
        ] {
            fs::write(root.join(f), b"x").unwrap();
        }
        (base.clone(), root)
    }

    fn run(ix: &Index, q: &str) -> Vec<String> {
        ix.search(&Query::parse(q), 100)
            .ids
            .iter()
            .map(|&i| ix.name(i as usize))
            .collect()
    }

    #[test]
    fn build_search_roundtrip() {
        let (base, root) = tree();
        let dest = base.join("idx.fci");
        let ix = build(&root, &dest, &AtomicUsize::new(0), &AtomicBool::new(false)).unwrap();
        assert_eq!(ix.len(), 1 + 3 + 6); // Root + Photos/Holiday/Docs + 6 Dateien

        // Groß-/Kleinschreibung wird exakt wiederhergestellt
        let r = run(&ix, "report");
        assert!(r.contains(&"Report 2024.PDF".to_string()), "{r:?}");
        assert!(r.contains(&"Annual-Report.docx".to_string()));
        assert!(r.contains(&"report.txt".to_string()));
        // Ranking: Präfix-/Wortanfang vor Teilstring; kürzere Namen zuerst
        assert_eq!(r[0], "report.txt");

        let mut j = run(&ix, "ext:jpg");
        j.sort();
        assert_eq!(j, vec!["Beach.JPG", "IMG_001.jpg"]);
        assert_eq!(run(&ix, "*.docx"), vec!["Annual-Report.docx"]);
        assert_eq!(run(&ix, "report !annual ext:txt"), vec!["report.txt"]);
        assert_eq!(run(&ix, "is:dir photos"), vec!["Photos"]);
        assert_eq!(run(&ix, "holiday/beach"), vec!["Beach.JPG"]);
        assert_eq!(run(&ix, "2024 report"), vec!["Report 2024.PDF"]);
        // unscharf
        assert!(run(&ix, "rpt").contains(&"report.txt".to_string()));
        // Pfade
        let id = ix.search(&Query::parse("beach"), 10).ids[0] as usize;
        assert_eq!(ix.path(id), root.join("Photos/Holiday/Beach.JPG"));

        // Suche nur unterhalb eines Ordners
        let scoped = |dir: &str, q: &str| -> Vec<String> {
            let d = ix.find_dir(&root.join(dir)).unwrap();
            let mut qq = Query::parse(q);
            qq.scope = Some(d as u32);
            let mut v: Vec<String> = ix
                .search(&qq, 100)
                .ids
                .iter()
                .map(|&i| ix.name(i as usize))
                .collect();
            v.sort();
            v
        };
        assert_eq!(scoped("Photos", "jpg"), vec!["Beach.JPG", "IMG_001.jpg"]);
        assert_eq!(scoped("Photos/Holiday", "jpg"), vec!["Beach.JPG"]);
        assert_eq!(
            scoped("Docs", "report"),
            vec!["Annual-Report.docx", "report.txt"]
        );
        assert_eq!(
            scoped("Docs", "rpt"),
            vec!["Annual-Report.docx", "report.txt"]
        ); // unscharf + Scope
        assert!(ix.find_dir(&root.join("Nope")).is_none());
        assert_eq!(ix.find_dir(&root), Some(0));

        // erneut laden (mmap)
        let ix2 = Index::open(&dest).unwrap();
        assert_eq!(ix2.len(), ix.len());
        let _ = fs::remove_dir_all(&base);
    }
}
