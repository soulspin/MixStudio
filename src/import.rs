#![allow(dead_code)]
//! Read-only import of other DJ libraries: Rekordbox XML, Traktor collection.nml, Serato
//! (`database V2` + crates), Engine DJ `m.db` (opened from a temporary copy) and iTunes / Music
//! Library XML. Nothing here ever writes to the other applications' files.
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::{Path, PathBuf};

#[derive(Clone, Copy, PartialEq, Eq, Debug, Serialize, Deserialize)]
pub enum Source { Rekordbox, Traktor, Serato, Engine, ITunes }
impl Source { pub fn name(self) -> &'static str { match self { Source::Rekordbox => "Rekordbox", Source::Traktor => "Traktor", Source::Serato => "Serato", Source::Engine => "Engine DJ", Source::ITunes => "iTunes" } } }

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Cue { pub name: String, pub pos: f64, pub hot: i32 }

#[derive(Clone, Debug, Default)]
pub struct ExtTrack { pub raw: String, pub path: String, pub title: String, pub artist: String, pub album: String, pub genre: String,
    pub bpm: Option<f64>, pub key: Option<String>, pub duration: f64, pub cues: Vec<Cue>, pub exists: bool,
    /// beat grid from the other app: (start s, BPM, beat number in the bar at start)
    pub grid: Vec<(f64, f64, i32)> }

#[derive(Clone, Debug, Default)]
pub struct Node { pub name: String, pub folder: bool, pub children: Vec<Node>, pub tracks: Vec<usize> }
impl Node {
    /// own tracks, or for a folder without own tracks every track below it (no duplicates)
    pub fn all_tracks(&self) -> Vec<usize> {
        if !self.tracks.is_empty() || self.children.is_empty() { return self.tracks.clone(); }
        let mut v = vec![]; let mut seen = std::collections::HashSet::new();
        fn rec(n: &Node, v: &mut Vec<usize>, s: &mut std::collections::HashSet<usize>) { for &t in &n.tracks { if s.insert(t) { v.push(t); } } for c in &n.children { rec(c, v, s); } }
        rec(self, &mut v, &mut seen); v
    }
    pub fn at(&self, path: &[usize]) -> Option<&Node> { let mut n = self; for &i in path { n = n.children.get(i)?; } Some(n) }
    pub fn count(&self) -> usize { (if self.folder { 0 } else { 1 }) + self.children.iter().map(|c| c.count()).sum::<usize>() }
}

#[derive(Clone, Debug)]
pub struct ExtLib { pub source: Source, pub file: String, pub tracks: Vec<ExtTrack>, pub root: Node }

#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
pub struct Remap { pub from: String, pub to: String }

/// Apply the longest matching prefix rule (case-insensitive, `\` and `/` equal) to a path.
pub fn remap(p: &str, maps: &[Remap]) -> String {
    let n = p.replace('\\', "/");
    let mut best: Option<(&Remap, usize)> = None;
    for m in maps {
        let f = m.from.replace('\\', "/"); let f = f.trim_end_matches('/'); if f.is_empty() { continue; }
        let Some(head) = n.get(..f.len()) else { continue };
        if head.eq_ignore_ascii_case(f) && (n.len() == f.len() || n.as_bytes()[f.len()] == b'/') && best.map_or(true, |b| f.len() > b.1) { best = Some((m, f.len())); }
    }
    match best { Some((m, l)) => format!("{}{}", m.to.replace('\\', "/").trim_end_matches('/'), &n[l..]), None => n }
}

// ---------- minimal XML reader ----------
#[derive(Debug, Default, Clone)]
pub struct El { pub name: String, pub attrs: Vec<(String, String)>, pub kids: Vec<El>, pub text: String }
impl El {
    pub fn attr(&self, k: &str) -> Option<&str> { self.attrs.iter().find(|a| a.0 == k).map(|a| a.1.as_str()) }
    pub fn child(&self, n: &str) -> Option<&El> { self.kids.iter().find(|k| k.name == n) }
    pub fn all<'a>(&'a self, n: &'a str) -> impl Iterator<Item = &'a El> + 'a { self.kids.iter().filter(move |k| k.name == n) }
}
fn unescape(s: &str) -> String {
    if !s.contains('&') { return s.to_string(); }
    let mut o = String::with_capacity(s.len()); let mut it = s.char_indices().peekable();
    while let Some((i, c)) = it.next() {
        if c != '&' { o.push(c); continue; }
        if let Some(j) = s[i..].find(';').filter(|&j| j < 12) {
            let e = &s[i + 1..i + j];
            let r = match e { "amp" => Some('&'), "lt" => Some('<'), "gt" => Some('>'), "quot" => Some('"'), "apos" => Some('\''),
                _ if e.starts_with("#x") || e.starts_with("#X") => u32::from_str_radix(&e[2..], 16).ok().and_then(char::from_u32),
                _ if e.starts_with('#') => e[1..].parse().ok().and_then(char::from_u32), _ => None };
            if let Some(r) = r { o.push(r); for _ in 0..j { it.next(); } continue; }
        }
        o.push('&');
    }
    o
}
/// Deepest element nesting accepted. DJ library exports use about ten levels; a corrupt or hostile file
/// with unbounded nesting would otherwise overflow the stack when the tree is walked or dropped.
const MAX_XML_DEPTH: usize = 256;
pub fn parse_xml(s: &str) -> Result<El, String> {
    let b = s.as_bytes(); let mut i = 0; let mut stack = vec![El::default()];
    let find = |from: usize, pat: &str| s[from..].find(pat).map(|k| from + k);
    while i < b.len() {
        if b[i] != b'<' {
            let e = find(i, "<").unwrap_or(b.len()); let t = &s[i..e];
            if !t.trim().is_empty() { stack.last_mut().unwrap().text.push_str(&unescape(t)); }
            i = e; continue;
        }
        if s[i..].starts_with("<!--") { i = find(i, "-->").map(|k| k + 3).unwrap_or(b.len()); continue; }
        if s[i..].starts_with("<![CDATA[") { let e = find(i, "]]>").unwrap_or(b.len()); stack.last_mut().unwrap().text.push_str(&s[i + 9..e]); i = (e + 3).min(b.len()); continue; }
        if s[i..].starts_with("<?") { i = find(i, "?>").map(|k| k + 2).unwrap_or(b.len()); continue; }
        if s[i..].starts_with("<!") { i = find(i, ">").map(|k| k + 1).unwrap_or(b.len()); continue; }
        if s[i..].starts_with("</") {
            i = find(i, ">").map(|k| k + 1).unwrap_or(b.len());
            if stack.len() > 1 { let e = stack.pop().unwrap(); stack.last_mut().unwrap().kids.push(e); }
            continue;
        }
        // start tag
        let mut j = i + 1; while j < b.len() && !b[j].is_ascii_whitespace() && b[j] != b'>' && b[j] != b'/' { j += 1; }
        let mut el = El { name: s[i + 1..j].to_string(), ..Default::default() }; let mut close = false;
        loop {
            while j < b.len() && b[j].is_ascii_whitespace() { j += 1; }
            if j >= b.len() { break; }
            if b[j] == b'>' { j += 1; break; }
            if b[j] == b'/' { close = true; j += 1; continue; }
            let k0 = j; while j < b.len() && b[j] != b'=' && !b[j].is_ascii_whitespace() && b[j] != b'>' && b[j] != b'/' { j += 1; }
            let key = s[k0..j].to_string();
            while j < b.len() && b[j].is_ascii_whitespace() { j += 1; }
            if j < b.len() && b[j] == b'=' { j += 1; while j < b.len() && b[j].is_ascii_whitespace() { j += 1; }
                if j < b.len() && (b[j] == b'"' || b[j] == b'\'') { let q = b[j] as char; let v0 = j + 1; let v1 = find(v0, if q == '"' { "\"" } else { "'" }).unwrap_or(b.len()); el.attrs.push((key, unescape(&s[v0..v1]))); j = (v1 + 1).min(b.len()); } }
            else { el.attrs.push((key, String::new())); }
        }
        i = j;
        if close { stack.last_mut().unwrap().kids.push(el); } else { stack.push(el); }
        if stack.len() > MAX_XML_DEPTH { return Err(format!("XML is nested more than {MAX_XML_DEPTH} levels deep; the file is damaged")); }
    }
    while stack.len() > 1 { let e = stack.pop().unwrap(); stack.last_mut().unwrap().kids.push(e); }
    Ok(stack.pop().unwrap())
}

/// file:// URL (Rekordbox, iTunes) -> path; Windows drive paths keep their drive letter.
pub fn url_path(s: &str) -> String {
    let mut p = s.strip_prefix("file://").unwrap_or(s); p = p.strip_prefix("localhost").unwrap_or(p);
    let bytes = p.as_bytes(); let mut v = Vec::with_capacity(bytes.len()); let mut i = 0;
    while i < bytes.len() { if bytes[i] == b'%' && i + 2 < bytes.len() { if let Some(x) = std::str::from_utf8(&bytes[i + 1..i + 3]).ok().and_then(|h| u8::from_str_radix(h, 16).ok()) { v.push(x); i += 3; continue; } } v.push(bytes[i]); i += 1; }
    let d = String::from_utf8_lossy(&v).to_string();
    let db = d.as_bytes(); if db.len() > 2 && db[0] == b'/' && db[1].is_ascii_alphabetic() && db[2] == b':' { d[1..].to_string() } else { d }
}
fn num(s: Option<&str>) -> Option<f64> { s.and_then(|x| x.trim().parse::<f64>().ok()).filter(|x| x.is_finite() && *x > 0.0) }
fn nonempty(s: Option<&str>) -> Option<String> { s.map(|x| x.trim().to_string()).filter(|x| !x.is_empty()) }
fn s(e: &El, k: &str) -> String { e.attr(k).unwrap_or("").to_string() }

// ---------- Rekordbox ----------
pub fn rekordbox(root: &El) -> Result<(Vec<ExtTrack>, Node), String> {
    let dj = root.child("DJ_PLAYLISTS").ok_or("not a Rekordbox XML (no DJ_PLAYLISTS)")?;
    let mut tracks = vec![]; let (mut by_id, mut by_loc) = (HashMap::new(), HashMap::new());
    if let Some(c) = dj.child("COLLECTION") { for t in c.all("TRACK") {
        let raw = url_path(t.attr("Location").unwrap_or(""));
        let cues = t.all("POSITION_MARK").filter_map(|m| Some(Cue { name: s(m, "Name"), pos: m.attr("Start")?.parse().ok()?, hot: m.attr("Num").and_then(|n| n.parse().ok()).unwrap_or(-1) })).collect();
        by_id.insert(s(t, "TrackID"), tracks.len()); by_loc.insert(t.attr("Location").map(url_path).unwrap_or_default(), tracks.len());
        tracks.push(ExtTrack { raw, title: s(t, "Name"), artist: s(t, "Artist"), album: s(t, "Album"), genre: s(t, "Genre"), bpm: num(t.attr("AverageBpm")), key: nonempty(t.attr("Tonality")),
            duration: num(t.attr("TotalTime")).unwrap_or(0.0), cues,
            grid: t.all("TEMPO").filter_map(|g| Some((g.attr("Inizio")?.parse().ok()?, g.attr("Bpm")?.parse().ok()?, g.attr("Battito").and_then(|b| b.parse().ok()).unwrap_or(1)))).collect(), ..Default::default() });
    } }
    fn node(e: &El, id: &HashMap<String, usize>, loc: &HashMap<String, usize>) -> Node {
        let folder = e.attr("Type") == Some("0"); let by_loc = e.attr("KeyType") == Some("1");
        Node { name: s(e, "Name"), folder, children: e.all("NODE").map(|c| node(c, id, loc)).collect(),
            tracks: e.all("TRACK").filter_map(|t| { let k = t.attr("Key")?; if by_loc { loc.get(&url_path(k)).copied() } else { id.get(k).copied() } }).collect() }
    }
    let mut r = dj.child("PLAYLISTS").and_then(|p| p.child("NODE")).map(|n| node(n, &by_id, &by_loc)).unwrap_or_default();
    r.name = "Playlists".into(); r.folder = true; Ok((tracks, r))
}

// ---------- Traktor ----------
fn nml_key(l: &El) -> String { format!("{}{}{}", s(l, "VOLUME"), s(l, "DIR"), s(l, "FILE")) }
fn nml_path(l: &El) -> String {
    let v = s(l, "VOLUME"); let d = s(l, "DIR").replace("/:", "/");
    let win = v.len() == 2 && v.ends_with(':'); format!("{}{}{}", if win { v.as_str() } else { "" }, d, s(l, "FILE"))
}
pub fn traktor(root: &El) -> Result<(Vec<ExtTrack>, Node), String> {
    let nml = root.child("NML").ok_or("not a Traktor collection (no NML)")?;
    let mut tracks = vec![]; let mut by_key = HashMap::new();
    if let Some(c) = nml.child("COLLECTION") { for e in c.all("ENTRY") {
        let Some(l) = e.child("LOCATION") else { continue };
        let info = e.child("INFO");
        let key = info.and_then(|i| nonempty(i.attr("KEY"))).or_else(|| e.child("MUSICAL_KEY").and_then(|m| m.attr("VALUE")?.parse::<usize>().ok()).filter(|&v| v < 24).map(|v| crate::analysis::camelot(v % 12, v >= 12)));
        let cues = e.all("CUE_V2").filter(|c| c.attr("TYPE") != Some("4")).filter_map(|c| Some(Cue { name: s(c, "NAME"), pos: c.attr("START")?.parse::<f64>().ok()? / 1000.0, hot: c.attr("HOTCUE").and_then(|h| h.parse().ok()).unwrap_or(-1) })).collect();
        by_key.insert(nml_key(l), tracks.len());
        tracks.push(ExtTrack { raw: nml_path(l), title: s(e, "TITLE"), artist: s(e, "ARTIST"), album: e.child("ALBUM").map(|a| s(a, "TITLE")).unwrap_or_default(),
            genre: info.map(|i| s(i, "GENRE")).unwrap_or_default(), bpm: e.child("TEMPO").and_then(|t| num(t.attr("BPM"))), key,
            duration: info.and_then(|i| num(i.attr("PLAYTIME_FLOAT")).or(num(i.attr("PLAYTIME")))).unwrap_or(0.0), cues,
            grid: match (e.all("CUE_V2").filter(|c| c.attr("TYPE") == Some("4")).find_map(|c| c.attr("START")?.parse::<f64>().ok()), e.child("TEMPO").and_then(|t| num(t.attr("BPM")))) { (Some(s), Some(b)) => vec![(s / 1000.0, b, 1)], _ => vec![] }, ..Default::default() });
    } }
    fn node(e: &El, k: &HashMap<String, usize>) -> Node {
        let folder = e.attr("TYPE") == Some("FOLDER");
        Node { name: s(e, "NAME"), folder, children: e.child("SUBNODES").map(|sn| sn.all("NODE").map(|c| node(c, k)).collect()).unwrap_or_default(),
            tracks: e.child("PLAYLIST").map(|p| p.all("ENTRY").filter_map(|en| k.get(en.child("PRIMARYKEY")?.attr("KEY")?).copied()).collect()).unwrap_or_default() }
    }
    let mut r = nml.child("PLAYLISTS").and_then(|p| p.child("NODE")).map(|n| node(n, &by_key)).unwrap_or_default();
    r.name = "Playlists".into(); r.folder = true; Ok((tracks, r))
}

// ---------- iTunes / Music ----------
enum P { Dict(Vec<(String, P)>), Arr(Vec<P>), S(String), I(i64), B(bool), O }
impl P {
    fn get(&self, k: &str) -> Option<&P> { if let P::Dict(v) = self { v.iter().find(|x| x.0 == k).map(|x| &x.1) } else { None } }
    fn str(&self, k: &str) -> String { match self.get(k) { Some(P::S(s)) => s.clone(), Some(P::I(i)) => i.to_string(), _ => String::new() } }
    fn int(&self, k: &str) -> Option<i64> { match self.get(k) { Some(P::I(i)) => Some(*i), _ => None } }
    fn yes(&self, k: &str) -> bool { matches!(self.get(k), Some(P::B(true))) }
}
fn plist(e: &El) -> P {
    match e.name.as_str() {
        "dict" => { let mut v = vec![]; let mut k = None; for c in &e.kids { if c.name == "key" { k = Some(c.text.clone()); } else if let Some(kk) = k.take() { v.push((kk, plist(c))); } } P::Dict(v) }
        "array" => P::Arr(e.kids.iter().map(plist).collect()),
        "string" | "date" => P::S(e.text.clone()), "integer" => P::I(e.text.trim().parse().unwrap_or(0)), "real" => P::S(e.text.clone()),
        "true" => P::B(true), "false" => P::B(false), _ => P::O }
}
pub fn itunes(root: &El) -> Result<(Vec<ExtTrack>, Node), String> {
    let d = root.child("plist").and_then(|p| p.child("dict")).ok_or("not an iTunes / Music library XML")?; let d = plist(d);
    let mut tracks = vec![]; let mut by_id = HashMap::new();
    if let Some(P::Dict(ts)) = d.get("Tracks") { for (_, t) in ts {
        let loc = t.str("Location"); if loc.is_empty() { continue; }
        by_id.insert(t.int("Track ID").unwrap_or(-1), tracks.len());
        tracks.push(ExtTrack { raw: url_path(&loc), title: t.str("Name"), artist: t.str("Artist"), album: t.str("Album"), genre: t.str("Genre"),
            bpm: t.int("BPM").filter(|&b| b > 0).map(|b| b as f64), duration: t.int("Total Time").unwrap_or(0) as f64 / 1000.0, ..Default::default() });
    } }
    let mut root = Node { name: "Playlists".into(), folder: true, ..Default::default() };
    if let Some(P::Arr(pl)) = d.get("Playlists") {
        // (persistent id, parent id, node)
        let mut items: Vec<(String, String, Node)> = vec![];
        for p in pl { if p.yes("Master") || p.get("Distinguished Kind").is_some() { continue; }
            let tr = match p.get("Playlist Items") { Some(P::Arr(v)) => v.iter().filter_map(|i| by_id.get(&i.int("Track ID")?).copied()).collect(), _ => vec![] };
            items.push((p.str("Playlist Persistent ID"), p.str("Parent Persistent ID"), Node { name: p.str("Name"), folder: p.yes("Folder"), children: vec![], tracks: tr })); }
        fn build(parent: &str, items: &[(String, String, Node)], depth: usize) -> Vec<Node> {
            if depth > 30 { return vec![]; }
            items.iter().filter(|x| x.1 == parent).map(|x| { let mut n = x.2.clone(); if !x.0.is_empty() { n.children = build(&x.0, items, depth + 1); } n }).collect()
        }
        let ids: std::collections::HashSet<&str> = items.iter().map(|x| x.0.as_str()).collect();
        let tops: Vec<Node> = items.iter().filter(|x| x.1.is_empty() || !ids.contains(x.1.as_str())).map(|x| { let mut n = x.2.clone(); if !x.0.is_empty() { n.children = build(&x.0, &items, 1); } n }).collect();
        root.children = tops;
    }
    Ok((tracks, root))
}

// ---------- Serato ----------
/// tag-length-value records (4-byte tag, big-endian u32 length)
pub fn tlv(b: &[u8]) -> Vec<(String, &[u8])> {
    let mut v = vec![]; let mut i = 0;
    while i + 8 <= b.len() { let tag = String::from_utf8_lossy(&b[i..i + 4]).to_string(); let n = u32::from_be_bytes([b[i + 4], b[i + 5], b[i + 6], b[i + 7]]) as usize;
        let e = (i + 8 + n).min(b.len()); v.push((tag, &b[i + 8..e])); i = e; }
    v
}
pub fn utf16be(b: &[u8]) -> String { let u: Vec<u16> = b.chunks_exact(2).map(|c| u16::from_be_bytes([c[0], c[1]])).collect(); String::from_utf16_lossy(&u).trim_end_matches('\0').to_string() }
/// Volume root for Serato's relative paths: the nearest ancestor of the `_Serato_` folder where the
/// sample tracks exist, otherwise the drive the folder is on (its parent).
fn serato_root(dir: &Path, sample: &[String]) -> PathBuf {
    let base = dir.parent().unwrap_or(Path::new("/"));
    for a in base.ancestors() { if !sample.is_empty() && sample.iter().take(8).any(|p| a.join(p).is_file()) { return a.to_path_buf(); } }
    if let Some(p) = sample.first() { if let Some(i) = p.find('/') { let first = &p[..i];
        for a in base.ancestors() { if a.file_name().is_some_and(|n| n == first) { return a.parent().unwrap_or(Path::new("/")).to_path_buf(); } } } }
    base.to_path_buf()
}
pub fn serato(dir: &Path) -> Result<(Vec<ExtTrack>, Node), String> {
    let db = std::fs::read(dir.join("database V2")).map_err(|e| format!("database V2: {e}"))?;
    let mut rel = vec![]; let mut tracks = vec![];
    for (tag, body) in tlv(&db) { if tag != "otrk" { continue; }
        let mut t = ExtTrack::default(); let mut p = String::new();
        for (k, v) in tlv(body) { let st = || utf16be(v); match k.as_str() {
            "pfil" => p = st(), "tsng" => t.title = st(), "tart" => t.artist = st(), "talb" => t.album = st(), "tgen" => t.genre = st(),
            "tbpm" => t.bpm = num(Some(&st())), "tkey" => t.key = nonempty(Some(&st())),
            "tlen" => { let x = st(); let mut secs = 0.0; for part in x.split(':') { secs = secs * 60.0 + part.trim().parse::<f64>().unwrap_or(0.0); } t.duration = secs; }
            _ => {} } }
        if p.is_empty() { continue; }
        p = p.trim_start_matches('/').to_string(); rel.push(p); tracks.push(t);
    }
    let mut crates: Vec<(Vec<String>, Vec<String>)> = vec![];
    if let Ok(rd) = std::fs::read_dir(dir.join("Subcrates")) { for e in rd.flatten() { let p = e.path();
        if p.extension().and_then(|x| x.to_str()) != Some("crate") { continue; }
        let name = p.file_stem().map(|s| s.to_string_lossy().to_string()).unwrap_or_default();
        let Ok(b) = std::fs::read(&p) else { continue };
        let paths = tlv(&b).into_iter().filter(|x| x.0 == "otrk").flat_map(|(_, o)| tlv(o).into_iter().filter(|x| x.0 == "ptrk").map(|x| utf16be(x.1).trim_start_matches('/').to_string()).collect::<Vec<_>>()).collect();
        crates.push((name.split("%%").map(|s| s.to_string()).collect(), paths)); } }
    crates.sort();
    let sample: Vec<String> = rel.iter().chain(crates.iter().flat_map(|c| c.1.iter())).take(8).cloned().collect();
    let root_dir = serato_root(dir, &sample);
    let mut by = HashMap::new(); for (i, p) in rel.iter().enumerate() { by.insert(p.clone(), i); }
    for (i, t) in tracks.iter_mut().enumerate() { t.raw = root_dir.join(&rel[i]).display().to_string(); }
    let mut root = Node { name: "Crates".into(), folder: true, ..Default::default() };
    for (names, paths) in crates {
        let mut ids = vec![];
        for p in paths { let i = *by.entry(p.clone()).or_insert_with(|| { tracks.push(ExtTrack { raw: root_dir.join(&p).display().to_string(),
            title: Path::new(&p).file_stem().map(|s| s.to_string_lossy().to_string()).unwrap_or_default(), ..Default::default() }); tracks.len() - 1 }); ids.push(i); }
        let mut n = &mut root;
        for nm in &names { let k = match n.children.iter().position(|c| &c.name == nm) { Some(k) => k, None => { n.children.push(Node { name: nm.clone(), ..Default::default() }); n.children.len() - 1 } }; n = &mut n.children[k]; }
        n.tracks = ids;
    }
    fn mark(n: &mut Node) { for c in n.children.iter_mut() { mark(c); } if !n.children.is_empty() && n.tracks.is_empty() { n.folder = true; } }
    mark(&mut root);
    Ok((tracks, root))
}

// ---------- Engine DJ ----------
/// Engine key index (0 = C, 1 = Am, 2 = G, ... around the circle of fifths) -> Camelot.
pub fn engine_key(k: i64) -> Option<String> { if !(0..24).contains(&k) { return None; } Some(format!("{}{}", (7 + k / 2) % 12 + 1, if k % 2 == 0 { 'B' } else { 'A' })) }
fn lexical(p: &Path) -> PathBuf { let mut o = PathBuf::new(); for c in p.components() { match c { std::path::Component::ParentDir => { o.pop(); } std::path::Component::CurDir => {} c => o.push(c.as_os_str()) } } o }
pub fn engine(db: &Path) -> Result<(Vec<ExtTrack>, Node), String> {
    // work on a private copy so Engine's database (and its -wal/-shm) is never touched
    let tmp = std::env::temp_dir().join(format!("mixstudio-engine-{}-{}", std::process::id(), std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_nanos()).unwrap_or(0)));
    std::fs::create_dir_all(&tmp).map_err(|e| e.to_string())?;
    let copy = tmp.join("m.db"); std::fs::copy(db, &copy).map_err(|e| format!("m.db: {e}"))?;
    let wal = PathBuf::from(format!("{}-wal", db.display())); if wal.is_file() { let _ = std::fs::copy(&wal, tmp.join("m.db-wal")); }
    let r = (|| -> Result<(Vec<ExtTrack>, Node), String> {
        let c = rusqlite::Connection::open(&copy).map_err(|e| e.to_string())?;
        let cols: Vec<String> = c.prepare("PRAGMA table_info(Track)").and_then(|mut s| s.query_map([], |r| r.get::<_, String>(1))?.collect()).map_err(|e| e.to_string())?;
        if cols.is_empty() { return Err("no Track table (not an Engine DJ database?)".into()); }
        let col = |n: &str| if cols.iter().any(|c| c == n) { n.to_string() } else { "NULL".into() };
        let q = format!("SELECT id, {}, {}, {}, {}, {}, {}, {}, {}, {}, {} FROM Track", col("path"), col("filename"), col("title"), col("artist"), col("album"), col("genre"), col("bpm"), col("bpmAnalyzed"), col("key"), col("length"));
        let base = db.parent().unwrap_or(Path::new("."));
        let mut tracks = vec![]; let mut by = HashMap::new();
        let mut st = c.prepare(&q).map_err(|e| e.to_string())?;
        let rows = st.query_map([], |r| Ok((r.get::<_, i64>(0)?, r.get::<_, Option<String>>(1)?, r.get::<_, Option<String>>(2)?, r.get::<_, Option<String>>(3)?, r.get::<_, Option<String>>(4)?,
            r.get::<_, Option<String>>(5)?, r.get::<_, Option<String>>(6)?, r.get::<_, Option<f64>>(7)?, r.get::<_, Option<f64>>(8)?, r.get::<_, Option<i64>>(9)?, r.get::<_, Option<f64>>(10)?))).map_err(|e| e.to_string())?;
        for row in rows { let (id, p, f, ti, ar, al, ge, bpm, bpma, key, len) = row.map_err(|e| e.to_string())?;
            let Some(p) = p.filter(|p| !p.is_empty()) else { continue };
            let raw = if Path::new(&p).is_absolute() || p.get(1..2) == Some(":") { p.clone() } else { lexical(&base.join(&p)).display().to_string() };
            by.insert(id, tracks.len());
            tracks.push(ExtTrack { raw, title: ti.filter(|s| !s.is_empty()).or(f).unwrap_or_default(), artist: ar.unwrap_or_default(), album: al.unwrap_or_default(), genre: ge.unwrap_or_default(),
                bpm: bpma.or(bpm).filter(|b| *b > 0.0), key: key.and_then(engine_key), duration: len.unwrap_or(0.0), ..Default::default() });
        }
        // playlists: sibling order via nextListId, entries via nextEntityId
        let pls: Vec<(i64, String, i64, i64)> = c.prepare("SELECT id, title, parentListId, nextListId FROM Playlist").and_then(|mut s| s.query_map([], |r| Ok((r.get(0)?, r.get::<_, Option<String>>(1)?.unwrap_or_default(), r.get::<_, Option<i64>>(2)?.unwrap_or(0), r.get::<_, Option<i64>>(3)?.unwrap_or(0))))?.collect()).unwrap_or_default();
        let ents: Vec<(i64, i64, i64, i64)> = c.prepare("SELECT id, listId, trackId, nextEntityId FROM PlaylistEntity").and_then(|mut s| s.query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get::<_, Option<i64>>(3)?.unwrap_or(0))))?.collect()).unwrap_or_default();
        fn chain<T: Clone>(items: Vec<(i64, i64, T)>) -> Vec<T> { // (id, next, value)
            let nexts: std::collections::HashSet<i64> = items.iter().map(|x| x.1).collect();
            let map: HashMap<i64, (i64, T)> = items.iter().map(|x| (x.0, (x.1, x.2.clone()))).collect();
            let mut out = vec![]; let mut seen = std::collections::HashSet::new();
            let mut heads: Vec<i64> = items.iter().filter(|x| !nexts.contains(&x.0)).map(|x| x.0).collect(); heads.sort();
            for h in heads.into_iter().chain(items.iter().map(|x| x.0)) { let mut cur = h; while let Some((n, v)) = map.get(&cur) { if !seen.insert(cur) { break; } out.push(v.clone()); cur = *n; } }
            out
        }
        fn build(parent: i64, pls: &[(i64, String, i64, i64)], ents: &[(i64, i64, i64, i64)], by: &HashMap<i64, usize>, d: usize) -> Vec<Node> {
            if d > 30 { return vec![]; }
            let kids: Vec<(i64, i64, (i64, String))> = pls.iter().filter(|p| p.2 == parent && p.0 != parent).map(|p| (p.0, p.3, (p.0, p.1.clone()))).collect();
            chain(kids).into_iter().map(|(id, name)| {
                let tr: Vec<usize> = chain(ents.iter().filter(|e| e.1 == id).map(|e| (e.0, e.3, e.2)).collect()).into_iter().filter_map(|t| by.get(&t).copied()).collect();
                let ch = build(id, pls, ents, by, d + 1); Node { name, folder: tr.is_empty() && !ch.is_empty(), children: ch, tracks: tr } }).collect()
        }
        Ok((tracks, Node { name: "Playlists".into(), folder: true, children: build(0, &pls, &ents, &by, 0), tracks: vec![] }))
    })();
    let _ = std::fs::remove_dir_all(&tmp);
    r
}

// ---------- detection / loading ----------
pub fn sniff(p: &Path) -> Option<Source> {
    let name = p.file_name()?.to_string_lossy().to_lowercase(); let ext = p.extension().map(|e| e.to_string_lossy().to_lowercase()).unwrap_or_default();
    if ext == "nml" { return Some(Source::Traktor); }
    if name == "database v2" || ext == "crate" || name == "_serato_" { return Some(Source::Serato); }
    if ext == "db" { return Some(Source::Engine); }
    if ext == "xml" { let mut b = vec![0u8; 4096]; let n = std::io::Read::read(&mut std::fs::File::open(p).ok()?, &mut b).ok()?; let h = String::from_utf8_lossy(&b[..n]);
        if h.contains("DJ_PLAYLISTS") { return Some(Source::Rekordbox); } if h.contains("<plist") { return Some(Source::ITunes); } if h.contains("<NML") { return Some(Source::Traktor); } }
    None
}
/// Read a library file (read-only), remap its paths and check which files exist.
pub fn load(p: &Path, maps: &[Remap]) -> Result<ExtLib, String> {
    let src = sniff(p).ok_or_else(|| format!("{}: unknown library format", p.display()))?;
    let xml = || -> Result<El, String> { let b = std::fs::read(p).map_err(|e| e.to_string())?; parse_xml(&String::from_utf8_lossy(&b)) };
    let (mut tracks, root) = match src {
        Source::Rekordbox => rekordbox(&xml()?)?, Source::Traktor => traktor(&xml()?)?, Source::ITunes => itunes(&xml()?)?, Source::Engine => engine(p)?,
        Source::Serato => { let d = if p.file_name().is_some_and(|n| n.to_string_lossy().eq_ignore_ascii_case("_serato_")) { p.to_path_buf() } else if p.extension().is_some_and(|e| e == "crate") { p.parent().and_then(|x| x.parent()).unwrap_or(p).to_path_buf() } else { p.parent().unwrap_or(p).to_path_buf() }; serato(&d)? }
    };
    for t in tracks.iter_mut() { t.path = remap(&t.raw, maps); t.exists = Path::new(&t.path).is_file(); }
    Ok(ExtLib { source: src, file: p.display().to_string(), tracks, root })
}
fn dirs(p: &Path) -> Vec<PathBuf> { std::fs::read_dir(p).map(|r| r.flatten().map(|e| e.path()).filter(|p| p.is_dir()).collect()).unwrap_or_default() }
/// Usual library locations under the home folder and mounted drives (incl. Windows / macOS user folders).
pub fn detect(home: &Path, mounts: &[PathBuf]) -> Vec<PathBuf> {
    let mut homes = vec![home.to_path_buf()]; let mut drives = vec![home.join("Music")];
    for m in mounts { for d in dirs(m) { drives.push(d.clone()); for u in ["Users", "home"] { for h in dirs(&d.join(u)) { homes.push(h); } } } }
    let mut out = vec![];
    for h in &homes {
        for c in ["rekordbox.xml", "Documents/rekordbox.xml", "Music/rekordbox.xml", "Desktop/rekordbox.xml", "Music/iTunes/iTunes Music Library.xml", "Music/iTunes/iTunes Library.xml", "Music/Music/Library.xml", "Music/Library.xml",
            "Music/_Serato_/database V2", "Music/Engine Library/Database2/m.db"] { out.push(h.join(c)); }
        for t in dirs(&h.join("Documents/Native Instruments")) { if t.file_name().is_some_and(|n| n.to_string_lossy().starts_with("Traktor")) { out.push(t.join("collection.nml")); } }
    }
    for d in &drives { out.push(d.join("_Serato_/database V2")); out.push(d.join("Engine Library/Database2/m.db")); }
    let mut seen = std::collections::HashSet::new();
    out.into_iter().filter(|p| p.is_file() && seen.insert(p.clone())).collect()
}
pub fn mount_roots() -> Vec<PathBuf> {
    let u = std::env::var("USER").unwrap_or_default();
    let mut v = vec![PathBuf::from("/mnt"), PathBuf::from("/media")];
    if !u.is_empty() { v.push(PathBuf::from(format!("/media/{u}"))); v.push(PathBuf::from(format!("/run/media/{u}"))); }
    v
}

/// Beat times from an imported grid (tempo segments); beat 0 is a downbeat. None without a usable grid.
pub fn grid_beats(g: &[(f64, f64, i32)], dur: f64) -> Option<Vec<f64>> {
    let mut g: Vec<(f64, f64, i32)> = g.iter().copied().filter(|x| x.1 > 20.0 && x.1 < 400.0 && x.0.is_finite()).collect();
    g.sort_by(|a, b| a.0.partial_cmp(&b.0).unwrap()); if g.is_empty() { return None; }
    let p0 = 60.0 / g[0].1; let mut pre = vec![]; let mut t = g[0].0 - p0; while t >= 0.0 { pre.push(t); t -= p0; } pre.reverse();
    let down = pre.len() + ((5 - g[0].2.clamp(1, 4)) % 4) as usize;
    let mut beats = pre;
    for (k, s) in g.iter().enumerate() { let end = g.get(k + 1).map(|n| n.0).unwrap_or(dur + 60.0 / s.1); let p = 60.0 / s.1; let mut t = s.0; while t < end - 1e-6 { beats.push(t); t += p; } }
    let drop = down % 4; if beats.len() < drop + 16 { return None; }
    Some(beats[drop..].to_vec())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn deeply_nested_xml_is_rejected_without_overflowing_the_stack() {
        let mut deep=String::new(); for _ in 0..100_000 { deep.push_str("<a>"); }
        assert!(parse_xml(&deep).unwrap_err().contains("nested"));
        let dir=std::env::temp_dir().join(format!("mixstudio-import-deep-{}",std::process::id())); std::fs::create_dir_all(&dir).unwrap();
        let p=dir.join("rekordbox.xml");
        let mut s=String::from(r#"<?xml version="1.0"?><DJ_PLAYLISTS><PLAYLISTS>"#); for _ in 0..100_000 { s.push_str(r#"<NODE Type="0" Name="x">"#); }
        std::fs::write(&p,&s).unwrap();
        assert!(load(&p,&[]).is_err(),"a damaged export must fail as an error, not abort the app");
        let shallow=parse_xml(r#"<a x="1"><b>t&amp;</b><c/></a>"#).unwrap();
        assert_eq!(shallow.kids[0].kids.len(),2); assert_eq!(shallow.kids[0].kids[0].text,"t&");
        std::fs::remove_dir_all(dir).ok();
    }
}
