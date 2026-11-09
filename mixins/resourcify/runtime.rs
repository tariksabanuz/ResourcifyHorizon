// Resourcify for Horizon: browse and install resource packs from Modrinth without leaving the game.
// A "+" button on the Resource Packs screen opens a browser: a search box, a list of packs with
// their icons, a detail pane with the versions made for this game, and an Install button.
//
// This is not the Java mod's code: it is written from Modrinth's public API (docs.modrinth.com)
// and from what the Java Resourcify shows its players. Resourcify is by DeDiamondPro, LGPL-3.0.
//
// This file is both the standalone library (with its tests) and the module injected into
// `Game` (`native_resourcify`). It needs nothing from the game: the JSON, the Modrinth answers,
// the layout, the wrapping and the like are plain functions. The drawing, the input and the
// network are in `ui.rs`.
use std::sync::atomic::{AtomicBool, Ordering};

pub const ID: &str = "resourcify";
pub const NAME: &str = "Resourcify";
pub const VERSION: &str = "0.1.1";
pub const AUTHOR: &str = "tariksabanuz";
pub const DESCRIPTION: &str = "By tariksabanuz. Adds a + button to the Resource Packs screen that opens a browser for Modrinth's resource packs: search, pick a version made for this game, install it.";
/// Modrinth's API.
pub const API: &str = "https://api.modrinth.com/v2";
/// Who is asking, as Modrinth's API terms want.
pub const AGENT: &str = "tariksabanuz/resourcify-horizon/0.1.1 (a Horizon native mod)";
/// Where Modrinth keeps its files and icons. Nothing else is downloaded.
pub const FILE_HOST: &str = "https://cdn.modrinth.com/";
/// Packs asked for at a time.
pub const PAGE: usize = 20;
/// The search box takes this many characters.
pub const MAX_SEARCH: usize = 64;
/// The biggest pack taken (what the game takes from a server).
pub const MAX_PACK_BYTES: u64 = 262_144_000;
/// How long after the last key the search starts, in milliseconds.
pub const DEBOUNCE_MS: u128 = 450;

// ---------------------------------------------------------------------------------------------
// Whether the browser is open, shared by the game's hooks.

static OPEN: AtomicBool = AtomicBool::new(false);
/// The browser has just been asked for, so it starts with an empty search box.
static FRESH: AtomicBool = AtomicBool::new(false);

pub fn open() {
    OPEN.store(true, Ordering::SeqCst);
    FRESH.store(true, Ordering::SeqCst);
}

pub fn close() {
    OPEN.store(false, Ordering::SeqCst);
}

pub fn is_open() -> bool {
    OPEN.load(Ordering::SeqCst)
}

/// True once after [`open`]: the browser starts over.
pub fn take_fresh() -> bool {
    FRESH.swap(false, Ordering::SeqCst)
}

/// Lets a hook call the function it hooks: the first call in sets the guard and runs the original
/// through the hook again; that second call finds the guard set and lets the original run.
pub struct Guard(AtomicBool);

impl Guard {
    pub const fn new() -> Self {
        Self(AtomicBool::new(false))
    }

    /// True when the guard was already set (this is the call the hook made itself).
    pub fn enter(&self) -> bool {
        self.0.swap(true, Ordering::SeqCst)
    }

    pub fn leave(&self) {
        self.0.store(false, Ordering::SeqCst);
    }
}

pub static DRAW: Guard = Guard::new();

// ---------------------------------------------------------------------------------------------
// A small JSON reader, so the answers of the API need no other crate.

#[derive(Clone, Debug, PartialEq)]
pub enum Json {
    Null,
    Bool(bool),
    Num(f64),
    Str(String),
    Arr(Vec<Json>),
    Obj(Vec<(String, Json)>),
}

static NULL: Json = Json::Null;

impl Json {
    /// A field of an object; null for anything else.
    pub fn get(&self, key: &str) -> &Json {
        match self {
            Json::Obj(fields) => fields.iter().find(|(k, _)| k == key).map_or(&NULL, |(_, v)| v),
            _ => &NULL,
        }
    }

    pub fn as_str(&self) -> Option<&str> {
        match self {
            Json::Str(s) => Some(s),
            _ => None,
        }
    }

    pub fn as_u64(&self) -> Option<u64> {
        match self {
            Json::Num(n) if n.is_finite() && *n >= 0.0 => Some(*n as u64),
            _ => None,
        }
    }

    pub fn as_bool(&self) -> Option<bool> {
        match self {
            Json::Bool(b) => Some(*b),
            _ => None,
        }
    }

    /// The elements of an array; none for anything else.
    pub fn items(&self) -> &[Json] {
        match self {
            Json::Arr(items) => items,
            _ => &[],
        }
    }
}

pub fn parse_json(text: &str) -> Result<Json, String> {
    let mut p = Reader { b: text.as_bytes(), i: 0, depth: 0 };
    let value = p.value()?;
    p.skip_space();
    if p.i < p.b.len() {
        return Err(p.at("something after the end"));
    }
    Ok(value)
}

struct Reader<'a> {
    b: &'a [u8],
    i: usize,
    depth: usize,
}

impl Reader<'_> {
    fn at(&self, what: &str) -> String {
        format!("bad JSON at byte {}: {what}", self.i)
    }

    fn skip_space(&mut self) {
        while self.i < self.b.len() && matches!(self.b[self.i], b' ' | b'\n' | b'\r' | b'\t') {
            self.i += 1;
        }
    }

    fn expect(&mut self, c: u8) -> Result<(), String> {
        if self.b.get(self.i) == Some(&c) {
            self.i += 1;
            Ok(())
        } else {
            Err(self.at(&format!("expected {}", c as char)))
        }
    }

    fn value(&mut self) -> Result<Json, String> {
        self.skip_space();
        if self.depth > 64 {
            return Err(self.at("too deep"));
        }
        match self.b.get(self.i).copied() {
            None => Err(self.at("the text ends")),
            Some(b'{') => {
                self.i += 1;
                self.depth += 1;
                let mut fields = Vec::new();
                self.skip_space();
                if self.b.get(self.i) == Some(&b'}') {
                    self.i += 1;
                    self.depth -= 1;
                    return Ok(Json::Obj(fields));
                }
                loop {
                    self.skip_space();
                    let key = self.string()?;
                    self.skip_space();
                    self.expect(b':')?;
                    let value = self.value()?;
                    fields.push((key, value));
                    self.skip_space();
                    match self.b.get(self.i).copied() {
                        Some(b',') => self.i += 1,
                        Some(b'}') => {
                            self.i += 1;
                            break;
                        }
                        _ => return Err(self.at("expected , or }")),
                    }
                }
                self.depth -= 1;
                Ok(Json::Obj(fields))
            }
            Some(b'[') => {
                self.i += 1;
                self.depth += 1;
                let mut items = Vec::new();
                self.skip_space();
                if self.b.get(self.i) == Some(&b']') {
                    self.i += 1;
                    self.depth -= 1;
                    return Ok(Json::Arr(items));
                }
                loop {
                    items.push(self.value()?);
                    self.skip_space();
                    match self.b.get(self.i).copied() {
                        Some(b',') => self.i += 1,
                        Some(b']') => {
                            self.i += 1;
                            break;
                        }
                        _ => return Err(self.at("expected , or ]")),
                    }
                }
                self.depth -= 1;
                Ok(Json::Arr(items))
            }
            Some(b'"') => Ok(Json::Str(self.string()?)),
            Some(b't') => self.word("true", Json::Bool(true)),
            Some(b'f') => self.word("false", Json::Bool(false)),
            Some(b'n') => self.word("null", Json::Null),
            Some(b'-' | b'0'..=b'9') => self.number(),
            Some(_) => Err(self.at("unexpected character")),
        }
    }

    fn word(&mut self, word: &str, value: Json) -> Result<Json, String> {
        if self.b.get(self.i..).is_some_and(|rest| rest.starts_with(word.as_bytes())) {
            self.i += word.len();
            Ok(value)
        } else {
            Err(self.at("unexpected word"))
        }
    }

    fn number(&mut self) -> Result<Json, String> {
        let start = self.i;
        while self.i < self.b.len() && matches!(self.b[self.i], b'-' | b'+' | b'.' | b'e' | b'E' | b'0'..=b'9') {
            self.i += 1;
        }
        std::str::from_utf8(&self.b[start..self.i])
            .ok()
            .and_then(|t| t.parse::<f64>().ok())
            .map(Json::Num)
            .ok_or_else(|| self.at("bad number"))
    }

    fn hex4(&mut self) -> Result<u32, String> {
        let digits = self.b.get(self.i..self.i + 4).ok_or_else(|| self.at("short \\u escape"))?;
        let text = std::str::from_utf8(digits).map_err(|_| self.at("bad \\u escape"))?;
        self.i += 4;
        u32::from_str_radix(text, 16).map_err(|_| self.at("bad \\u escape"))
    }

    fn string(&mut self) -> Result<String, String> {
        self.expect(b'"')?;
        let mut out: Vec<u8> = Vec::new();
        loop {
            let Some(&c) = self.b.get(self.i) else {
                return Err(self.at("a string never ends"));
            };
            self.i += 1;
            match c {
                b'"' => break,
                b'\\' => {
                    let Some(&e) = self.b.get(self.i) else {
                        return Err(self.at("a string never ends"));
                    };
                    self.i += 1;
                    match e {
                        b'"' => out.push(b'"'),
                        b'\\' => out.push(b'\\'),
                        b'/' => out.push(b'/'),
                        b'b' => out.push(8),
                        b'f' => out.push(12),
                        b'n' => out.push(b'\n'),
                        b'r' => out.push(b'\r'),
                        b't' => out.push(b'\t'),
                        b'u' => {
                            let hi = self.hex4()?;
                            let ch = if (0xD800..0xDC00).contains(&hi) {
                                // A pair: the next escape is the low half.
                                if self.b.get(self.i..).is_some_and(|rest| rest.starts_with(b"\\u")) {
                                    self.i += 2;
                                    let lo = self.hex4()?;
                                    if (0xDC00..0xE000).contains(&lo) {
                                        char::from_u32(0x10000 + ((hi - 0xD800) << 10) + (lo - 0xDC00))
                                    } else {
                                        None
                                    }
                                } else {
                                    None
                                }
                            } else {
                                char::from_u32(hi)
                            };
                            let mut buf = [0u8; 4];
                            out.extend_from_slice(ch.unwrap_or('\u{FFFD}').encode_utf8(&mut buf).as_bytes());
                        }
                        _ => return Err(self.at("bad escape")),
                    }
                }
                _ => out.push(c),
            }
        }
        Ok(String::from_utf8_lossy(&out).into_owned())
    }
}

// ---------------------------------------------------------------------------------------------
// What Modrinth answers.

/// One pack in the search results.
#[derive(Clone, Debug, PartialEq)]
pub struct Hit {
    pub id: String,
    pub title: String,
    pub author: String,
    pub summary: String,
    pub downloads: u64,
    pub follows: u64,
    pub icon_url: Option<String>,
    pub categories: Vec<String>,
}

/// The file of a version that is the pack.
#[derive(Clone, Debug, PartialEq)]
pub struct File {
    pub url: String,
    pub filename: String,
    pub size: u64,
    pub sha1: String,
}

/// One version of a pack.
#[derive(Clone, Debug, PartialEq)]
pub struct Version {
    pub id: String,
    pub number: String,
    pub name: String,
    /// `release`, `beta` or `alpha`.
    pub kind: String,
    pub games: Vec<String>,
    pub date: String,
    pub file: Option<File>,
}

fn strings(v: &Json) -> Vec<String> {
    v.items().iter().filter_map(|s| s.as_str().map(str::to_owned)).collect()
}

/// An error the API sent as its answer (`{"error": ..., "description": ...}`).
fn api_error(root: &Json) -> Option<String> {
    let error = root.get("error").as_str()?;
    let why = root.get("description").as_str().unwrap_or("");
    Some(format!("Modrinth: {error} {why}").trim().to_owned())
}

/// A search answer: the packs and how many there are in all.
pub fn parse_search(body: &str) -> Result<(Vec<Hit>, u64), String> {
    let root = parse_json(body)?;
    if let Some(e) = api_error(&root) {
        return Err(e);
    }
    let Json::Arr(hits) = root.get("hits") else {
        return Err("Modrinth's answer has no list of packs".into());
    };
    let found: Vec<Hit> = hits
        .iter()
        .filter_map(|h| {
            let id = h.get("project_id").as_str()?.to_owned();
            Some(Hit {
                id,
                title: h.get("title").as_str().unwrap_or("Unnamed").to_owned(),
                author: h.get("author").as_str().unwrap_or("unknown").to_owned(),
                summary: h.get("description").as_str().unwrap_or("").to_owned(),
                downloads: h.get("downloads").as_u64().unwrap_or(0),
                follows: h.get("follows").as_u64().unwrap_or(0),
                icon_url: h.get("icon_url").as_str().filter(|u| icon_url_ok(u)).map(str::to_owned),
                categories: strings(h.get("display_categories")),
            })
        })
        .collect();
    let total = root.get("total_hits").as_u64().unwrap_or(found.len() as u64);
    Ok((found, total))
}

/// The versions of a pack, newest first as Modrinth lists them.
pub fn parse_versions(body: &str) -> Result<Vec<Version>, String> {
    let root = parse_json(body)?;
    if let Some(e) = api_error(&root) {
        return Err(e);
    }
    let Json::Arr(list) = &root else {
        return Err("Modrinth's answer has no list of versions".into());
    };
    Ok(list
        .iter()
        .filter_map(|v| {
            let id = v.get("id").as_str()?.to_owned();
            let files = v.get("files").items();
            let chosen = files.iter().find(|f| f.get("primary").as_bool() == Some(true)).or_else(|| files.first());
            let file = chosen.and_then(|f| {
                Some(File {
                    url: f.get("url").as_str()?.to_owned(),
                    filename: f.get("filename").as_str()?.to_owned(),
                    size: f.get("size").as_u64().unwrap_or(0),
                    sha1: f.get("hashes").get("sha1").as_str()?.to_ascii_lowercase(),
                })
            });
            Some(Version {
                id,
                number: v.get("version_number").as_str().unwrap_or("?").to_owned(),
                name: v.get("name").as_str().unwrap_or("").to_owned(),
                kind: v.get("version_type").as_str().unwrap_or("release").to_owned(),
                games: strings(v.get("game_versions")),
                date: v.get("date_published").as_str().unwrap_or("").to_owned(),
                file,
            })
        })
        .collect())
}

/// Icons and files come from Modrinth's own host and nowhere else.
pub fn icon_url_ok(url: &str) -> bool {
    url.starts_with(FILE_HOST)
}

/// A file may be downloaded when it is on Modrinth's host, ends in `.zip`, has a SHA-1 to check
/// it by and is not too big.
pub fn file_ok(f: &File) -> Result<(), String> {
    if !f.url.starts_with(FILE_HOST) {
        return Err("The file is not on Modrinth's own server, so it is not downloaded.".into());
    }
    if f.sha1.len() != 40 || !f.sha1.bytes().all(|b| b.is_ascii_hexdigit()) {
        return Err("Modrinth gave no checksum for the file, so it is not downloaded.".into());
    }
    if f.size > MAX_PACK_BYTES {
        return Err(format!("The file is {}, more than the {} the game takes.", human_size(f.size), human_size(MAX_PACK_BYTES)));
    }
    if safe_filename(&f.filename).is_none() {
        return Err("The file is not a .zip resource pack.".into());
    }
    Ok(())
}

// ---------------------------------------------------------------------------------------------
// What is asked of Modrinth.

/// Percent-encoding for a query value.
pub fn encode(s: &str) -> String {
    s.bytes()
        .map(|b| match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => (b as char).to_string(),
            _ => format!("%{b:02X}"),
        })
        .collect()
}

/// How the search orders what it finds.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Sort {
    Relevance,
    Downloads,
    Newest,
    Updated,
}

impl Sort {
    pub fn next(self) -> Sort {
        match self {
            Sort::Relevance => Sort::Downloads,
            Sort::Downloads => Sort::Newest,
            Sort::Newest => Sort::Updated,
            Sort::Updated => Sort::Relevance,
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Sort::Relevance => "Relevance",
            Sort::Downloads => "Downloads",
            Sort::Newest => "Newest",
            Sort::Updated => "Updated",
        }
    }

    /// Modrinth's name for it.
    pub fn index(self) -> &'static str {
        match self {
            Sort::Relevance => "relevance",
            Sort::Downloads => "downloads",
            Sort::Newest => "newest",
            Sort::Updated => "updated",
        }
    }
}

/// The search for `query` among resource packs, from `offset`. With `game`, only packs that have a
/// version for that game version.
pub fn search_url(query: &str, sort: Sort, game: Option<&str>, offset: usize) -> String {
    let mut facets = String::from("[[\"project_type:resourcepack\"]");
    if let Some(game) = game {
        facets.push_str(&format!(",[\"versions:{game}\"]"));
    }
    facets.push(']');
    format!(
        "{API}/search?query={}&facets={}&index={}&limit={PAGE}&offset={offset}",
        encode(query.trim()),
        encode(&facets),
        sort.index()
    )
}

/// The versions of a pack, only those for `game` when given.
pub fn versions_url(project: &str, game: Option<&str>) -> String {
    let mut url = format!("{API}/project/{}/version", encode(project));
    if let Some(game) = game {
        url.push_str(&format!("?game_versions={}", encode(&format!("[\"{game}\"]"))));
    }
    url
}

// ---------------------------------------------------------------------------------------------
// Files.

/// The name a downloaded pack is saved under: no folders, only plain characters, a `.zip`.
pub fn safe_filename(name: &str) -> Option<String> {
    let base = name.rsplit(['/', '\\']).next().unwrap_or("");
    let cleaned: String = base
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() || " ._()+-".contains(c) { c } else { '_' })
        .collect();
    let cleaned = cleaned.trim_matches(|c| c == '.' || c == ' ').to_owned();
    let lower = cleaned.to_ascii_lowercase();
    if cleaned.len() > 120 || !lower.ends_with(".zip") || lower.len() <= 4 {
        return None;
    }
    Some(cleaned)
}

/// `name`, or `name (2).zip`, `name (3).zip` ... when `taken` says the file is there already.
pub fn unique_name(name: &str, taken: &dyn Fn(&str) -> bool) -> String {
    if !taken(name) {
        return name.to_owned();
    }
    let stem = name.strip_suffix(".zip").or_else(|| name.strip_suffix(".ZIP")).unwrap_or(name);
    for n in 2..100 {
        let candidate = format!("{stem} ({n}).zip");
        if !taken(&candidate) {
            return candidate;
        }
    }
    format!("{stem} (100).zip")
}

/// One installed pack, kept in a small list so the browser can say "Installed".
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Record {
    pub project: String,
    pub version: String,
    pub filename: String,
}

pub fn parse_records(text: &str) -> Vec<Record> {
    text.lines()
        .filter_map(|line| {
            let mut parts = line.split('\t');
            let (project, version, filename) = (parts.next()?, parts.next()?, parts.next()?);
            (!project.is_empty() && !filename.is_empty()).then(|| Record {
                project: project.to_owned(),
                version: version.to_owned(),
                filename: filename.to_owned(),
            })
        })
        .collect()
}

pub fn format_records(records: &[Record]) -> String {
    records
        .iter()
        .map(|r| {
            let clean = |s: &str| s.replace(['\t', '\n', '\r'], " ");
            format!("{}\t{}\t{}\n", clean(&r.project), clean(&r.version), clean(&r.filename))
        })
        .collect()
}

/// Sets the record of a pack (one per pack).
pub fn put_record(records: &mut Vec<Record>, new: Record) {
    records.retain(|r| r.project != new.project);
    records.push(new);
}

// ---------------------------------------------------------------------------------------------
// Numbers as words.

/// 950, 1.2K, 3.4M.
pub fn count(n: u64) -> String {
    fn one(n: f64, unit: &str) -> String {
        let text = format!("{n:.1}");
        format!("{}{unit}", text.strip_suffix(".0").unwrap_or(&text))
    }
    match n {
        0..=999 => n.to_string(),
        1_000..=999_999 => one(n as f64 / 1_000.0, "K"),
        1_000_000..=999_999_999 => one(n as f64 / 1_000_000.0, "M"),
        _ => one(n as f64 / 1_000_000_000.0, "B"),
    }
}

/// 812 B, 1.5 MiB.
pub fn human_size(bytes: u64) -> String {
    const UNITS: [&str; 4] = ["B", "KiB", "MiB", "GiB"];
    let mut value = bytes as f64;
    let mut unit = 0;
    while value >= 1024.0 && unit < UNITS.len() - 1 {
        value /= 1024.0;
        unit += 1;
    }
    if unit == 0 {
        format!("{bytes} B")
    } else {
        format!("{value:.1} {}", UNITS[unit])
    }
}

/// How far a download is, 0 to 100; None when the size is not known.
pub fn percent(done: u64, total: u64) -> Option<u32> {
    (total > 0).then(|| ((done.min(total) as u128 * 100) / total as u128) as u32)
}

// ---------------------------------------------------------------------------------------------
// The screen: everything about it that needs no game.

/// A rectangle in GUI pixels.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Rect {
    pub x: f32,
    pub y: f32,
    pub w: f32,
    pub h: f32,
}

impl Rect {
    pub const fn new(x: f32, y: f32, w: f32, h: f32) -> Self {
        Self { x, y, w, h }
    }

    pub fn contains(&self, x: f32, y: f32) -> bool {
        x >= self.x && x < self.x + self.w && y >= self.y && y < self.y + self.h
    }
}

/// Where the list starts (below the search box) and how tall the footer is, in GUI pixels.
pub const HEADER: f32 = 48.0;
pub const FOOTER: f32 = 44.0;
/// One list row: a 32 pixel icon with a pixel of air above and below.
pub const ROW: f32 = 36.0;
pub const ICON: f32 = 32.0;
pub const BUTTON_H: f32 = 20.0;
pub const LINE: f32 = 10.0;
/// What one wheel step scrolls.
pub const SCROLL_STEP: f32 = 18.0;

/// The "+" button on the Resource Packs screen, for a screen `w` wide.
pub fn plus_button(w: f32) -> Rect {
    Rect::new((w - 26.0).max(0.0), 6.0, BUTTON_H, BUTTON_H)
}

/// Where everything on the screen is, for a screen `w` by `h` GUI pixels.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Layout {
    pub search: Rect,
    pub sort: Rect,
    pub game: Rect,
    /// Where the line "Showing 20 of 312 packs" starts.
    pub status: [f32; 2],
    pub list: Rect,
    pub detail: Rect,
    pub done: Rect,
    /// The top of the line of small print under the footer button.
    pub hint_y: f32,
}

pub fn layout(w: f32, h: f32) -> Layout {
    let list_w = (w * 0.45).clamp(150.0, 320.0).min((w - 100.0).max(100.0));
    let body_h = (h - HEADER - FOOTER).max(40.0);
    let detail = Rect::new(list_w + 6.0, HEADER, (w - list_w - 12.0).max(40.0), body_h);
    let bw = ((detail.w - 4.0) / 2.0).floor().max(30.0);
    let done_w = 200.0_f32.min((w - 16.0).max(60.0));
    Layout {
        search: Rect::new(6.0, 12.0, list_w - 12.0, BUTTON_H),
        sort: Rect::new(detail.x, 12.0, bw, BUTTON_H),
        game: Rect::new(detail.x + bw + 4.0, 12.0, bw, BUTTON_H),
        status: [6.0, 36.0],
        list: Rect::new(0.0, HEADER, list_w, body_h),
        detail,
        done: Rect::new((w / 2.0).floor() - done_w / 2.0, h - FOOTER + 8.0, done_w, BUTTON_H),
        hint_y: h - 12.0,
    }
}

/// How far the list of `rows` rows scrolls in a view `view_h` tall.
pub fn max_scroll(rows: usize, view_h: f32) -> f32 {
    (rows as f32 * ROW - view_h).max(0.0)
}

/// True when the list is scrolled close enough to its end to ask for the next page.
pub fn near_end(scroll: f32, rows: usize, view_h: f32) -> bool {
    rows > 0 && scroll + view_h + 2.0 * ROW >= rows as f32 * ROW
}

/// The row under `(x, y)` of a list scrolled by `scroll` (the scroll bar's strip is not a row).
pub fn row_at(l: &Layout, scroll: f32, count: usize, x: f32, y: f32) -> Option<usize> {
    if x < l.list.x || x >= l.list.x + l.list.w - 6.0 || y < l.list.y || y >= l.list.y + l.list.h {
        return None;
    }
    let row = ((y - l.list.y + scroll) / ROW).floor();
    if row < 0.0 {
        return None;
    }
    let row = row as usize;
    (row < count).then_some(row)
}

/// The row `delta` rows from `current` (none picks the first or the last), kept in the list.
pub fn step(count: usize, current: Option<usize>, delta: i32) -> Option<usize> {
    if count == 0 {
        return None;
    }
    let at = current.map_or(if delta < 0 { count as i32 } else { -1 }, |c| c as i32);
    Some((at + delta).clamp(0, count as i32 - 1) as usize)
}

/// The scroll that shows row `row` of a view `view_h` tall.
pub fn scroll_into_view(scroll: f32, row: usize, view_h: f32) -> f32 {
    let top = row as f32 * ROW;
    if top < scroll {
        top
    } else if top + ROW > scroll + view_h {
        top + ROW - view_h
    } else {
        scroll
    }
}

/// Words broken into lines no wider than `max` (`measure` is the text's width); `\n` starts a
/// line, a word wider than a line is cut where it stops fitting.
pub fn wrap(text: &str, max: f32, measure: &dyn Fn(&str) -> f32) -> Vec<String> {
    let mut lines: Vec<String> = Vec::new();
    for paragraph in text.split('\n') {
        let mut line = String::new();
        for word in paragraph.split_whitespace() {
            let mut word = word.to_owned();
            while measure(&word) > max && word.chars().count() > 1 {
                let mut cut = String::new();
                for c in word.chars() {
                    let mut next = cut.clone();
                    next.push(c);
                    if measure(&next) > max && !cut.is_empty() {
                        break;
                    }
                    cut = next;
                }
                if !line.is_empty() {
                    lines.push(std::mem::take(&mut line));
                }
                let rest: String = word.chars().skip(cut.chars().count()).collect();
                lines.push(cut);
                word = rest;
            }
            if word.is_empty() {
                continue;
            }
            let joined = if line.is_empty() { word.clone() } else { format!("{line} {word}") };
            if line.is_empty() || measure(&joined) <= max {
                line = joined;
            } else {
                lines.push(std::mem::replace(&mut line, word));
            }
        }
        lines.push(line);
    }
    while lines.last().is_some_and(|l| l.is_empty()) {
        lines.pop();
    }
    lines
}

/// The end of `text` that fits `max` wide: a search box shows what you are typing.
pub fn tail_fit(text: &str, max: f32, measure: &dyn Fn(&str) -> f32) -> String {
    let chars: Vec<char> = text.chars().collect();
    let mut start = 0;
    while start < chars.len() && measure(&chars[start..].iter().collect::<String>()) > max {
        start += 1;
    }
    chars[start..].iter().collect()
}

/// `text`, or its start and "..." when it is wider than `max`.
pub fn ellipsize(text: &str, max: f32, measure: &dyn Fn(&str) -> f32) -> String {
    if measure(text) <= max {
        return text.to_owned();
    }
    let mut out = String::new();
    for c in text.chars() {
        let mut next = out.clone();
        next.push(c);
        if measure(&format!("{next}...")) > max {
            break;
        }
        out = next;
    }
    format!("{}...", out.trim_end())
}

/// Typing into the search box: printable characters only, up to [`MAX_SEARCH`].
pub fn type_text(search: &mut String, text: &str) {
    for c in text.chars().filter(|c| !c.is_control()) {
        if search.chars().count() >= MAX_SEARCH {
            break;
        }
        search.push(c);
    }
}

/// Backspace, or Ctrl+Backspace for the last word.
pub fn backspace(search: &mut String, word: bool) {
    if !word {
        search.pop();
        return;
    }
    while search.ends_with(' ') {
        search.pop();
    }
    while search.chars().last().is_some_and(|c| c != ' ') {
        search.pop();
    }
}

/// A pack's icon colour while its picture is not there: the same id is always the same hue.
pub fn icon_rgb(id: &str) -> [u8; 3] {
    let mut hash: u32 = 2_166_136_261;
    for b in id.bytes() {
        hash ^= u32::from(b);
        hash = hash.wrapping_mul(16_777_619);
    }
    hsv((hash % 360) as f32, 0.5, 0.9)
}

fn hsv(h: f32, s: f32, v: f32) -> [u8; 3] {
    let c = v * s;
    let x = c * (1.0 - ((h / 60.0) % 2.0 - 1.0).abs());
    let m = v - c;
    let (r, g, b) = match (h / 60.0) as u32 {
        0 => (c, x, 0.0),
        1 => (x, c, 0.0),
        2 => (0.0, c, x),
        3 => (0.0, x, c),
        4 => (x, 0.0, c),
        _ => (c, 0.0, x),
    };
    [((r + m) * 255.0).round() as u8, ((g + m) * 255.0).round() as u8, ((b + m) * 255.0).round() as u8]
}

/// `rgb` made `k` times as bright (above 1 lightens, below 1 darkens).
pub fn shade(rgb: [u8; 3], k: f32) -> [u8; 3] {
    rgb.map(|c| (f32::from(c) * k).round().clamp(0.0, 255.0) as u8)
}

/// How a line of the detail pane is coloured.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Tone {
    Text,
    Dim,
    Good,
    Bad,
    Heading,
}

/// A line of the detail pane under the Install button; `version` is the version it picks.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Line {
    pub text: String,
    pub tone: Tone,
    pub version: Option<usize>,
}

fn plain(text: impl Into<String>, tone: Tone) -> Line {
    Line { text: text.into(), tone, version: None }
}

/// How the versions of the picked pack are doing.
#[derive(Clone, Copy, Debug)]
pub enum Versions<'a> {
    Loading,
    Failed(&'a str),
    Loaded(&'a [Version]),
}

/// The newest game versions a version is for, "26.3, 26.2 +4".
pub fn games_short(games: &[String]) -> String {
    let shown: Vec<&str> = games.iter().rev().take(3).map(String::as_str).collect();
    let more = games.len().saturating_sub(3);
    let mut text = shown.join(", ");
    if more > 0 {
        text.push_str(&format!(" +{more}"));
    }
    text
}

/// The version that is picked: the one the player chose, else the newest release, else the newest.
pub fn pick_version(versions: &[Version], chosen: Option<&str>) -> Option<usize> {
    if let Some(id) = chosen
        && let Some(i) = versions.iter().position(|v| v.id == id)
    {
        return Some(i);
    }
    versions.iter().position(|v| v.kind == "release").or(if versions.is_empty() { None } else { Some(0) })
}

/// The detail pane's text under the Install button: the versions, then what the pack is.
pub fn body_lines(
    hit: &Hit,
    versions: Versions<'_>,
    picked: Option<usize>,
    game: Option<&str>,
    width: f32,
    measure: &dyn Fn(&str) -> f32,
) -> Vec<Line> {
    let mut out = vec![plain("Versions", Tone::Heading)];
    match versions {
        Versions::Loading => out.push(plain("Loading...", Tone::Dim)),
        Versions::Failed(why) => {
            for line in wrap(&format!("Could not load them: {why}"), width, measure) {
                out.push(plain(line, Tone::Bad));
            }
        }
        Versions::Loaded([]) => {
            let text = match game {
                Some(g) => format!("No version made for {g}. \"All versions\" shows the others."),
                None => "This pack has no versions.".to_owned(),
            };
            for line in wrap(&text, width, measure) {
                out.push(plain(line, Tone::Dim));
            }
        }
        Versions::Loaded(list) => {
            for (i, v) in list.iter().enumerate().take(12) {
                let mark = if Some(i) == picked { ">" } else { " " };
                let tag = if v.kind == "release" { String::new() } else { format!(" [{}]", v.kind) };
                let text = format!("{mark} {}{tag} ({})", v.number, games_short(&v.games));
                out.push(Line {
                    text: ellipsize(&text, width, measure),
                    tone: if Some(i) == picked { Tone::Heading } else { Tone::Text },
                    version: Some(i),
                });
            }
            if list.len() > 12 {
                out.push(plain(format!("... and {} older", list.len() - 12), Tone::Dim));
            }
        }
    }
    out.push(plain("", Tone::Text));
    if hit.summary.trim().is_empty() {
        out.extend(wrap("This pack has no description.", width, measure).into_iter().map(|t| plain(t, Tone::Dim)));
    } else {
        out.extend(wrap(hit.summary.trim(), width, measure).into_iter().map(|t| plain(t, Tone::Text)));
    }
    if !hit.categories.is_empty() {
        out.push(plain("", Tone::Text));
        out.extend(wrap(&format!("Tags: {}", hit.categories.join(", ")), width, measure).into_iter().map(|t| plain(t, Tone::Dim)));
    }
    out
}

/// The version a click at `y` picks, from the lines [`body_lines`] gave and where the body starts.
pub fn version_at(lines: &[Line], body_y: f32, y: f32) -> Option<usize> {
    if y < body_y {
        return None;
    }
    lines.get(((y - body_y) / LINE).floor() as usize)?.version
}

/// Where the detail pane puts things, for a pane scrolled by `scroll`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Detail {
    pub icon: Rect,
    pub name: [f32; 2],
    pub info: [[f32; 2]; 2],
    pub install: Rect,
    /// What the install is doing, beside the button.
    pub status: [f32; 2],
    /// The first line of [`body_lines`].
    pub body: [f32; 2],
    /// How wide those lines may be.
    pub width: f32,
}

pub fn detail(l: &Layout, scroll: f32) -> Detail {
    let px = l.detail.x + 4.0;
    let top = l.detail.y + 4.0 - scroll;
    let text_x = px + ICON + 6.0;
    Detail {
        icon: Rect::new(px, top, ICON, ICON),
        name: [text_x, top + 2.0],
        info: [[text_x, top + 14.0], [text_x, top + 24.0]],
        install: Rect::new(px, top + 40.0, 90.0, BUTTON_H),
        status: [px + 96.0, top + 46.0],
        body: [px, top + 68.0],
        width: (l.detail.w - 8.0 - 8.0).max(40.0),
    }
}

/// The detail pane's whole height, to scroll it.
pub fn detail_height(lines: usize) -> f32 {
    4.0 + 68.0 + lines as f32 * LINE + 4.0
}

/// "Showing 20 of 312 packs", or what the browser is doing instead.
pub fn status_text(loading: bool, error: Option<&str>, shown: usize, total: u64) -> String {
    if let Some(e) = error {
        return e.to_owned();
    }
    if loading && shown == 0 {
        return "Searching...".to_owned();
    }
    if shown == 0 {
        return "No packs found".to_owned();
    }
    let packs = if total == 1 { "pack" } else { "packs" };
    let more = if loading { " ..." } else { "" };
    format!("Showing {shown} of {total} {packs}{more}")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn six(s: &str) -> f32 {
        s.chars().count() as f32 * 6.0
    }

    #[test]
    fn json_reads_what_modrinth_sends() {
        let j = parse_json(r#" {"a": [1, 2.5, -3e2, true, false, null], "b": {"c": "x\ty\"z\\"}, "d": "\u00e9\ud83d\ude00", "e": ""} "#).unwrap();
        assert_eq!(j.get("a").items().len(), 6);
        assert_eq!(j.get("a").items()[1], Json::Num(2.5));
        assert_eq!(j.get("a").items()[2], Json::Num(-300.0));
        assert_eq!(j.get("a").items()[3].as_bool(), Some(true));
        assert_eq!(j.get("b").get("c").as_str(), Some("x\ty\"z\\"));
        assert_eq!(j.get("d").as_str(), Some("\u{e9}\u{1F600}"));
        assert_eq!(j.get("e").as_str(), Some(""));
        assert_eq!(*j.get("missing").get("deeper"), Json::Null);
        assert_eq!(Json::Num(-1.0).as_u64(), None);
        assert_eq!(Json::Num(7.0).as_u64(), Some(7));
        assert_eq!(parse_json("[]").unwrap(), Json::Arr(vec![]));
        assert_eq!(parse_json("{}").unwrap(), Json::Obj(vec![]));
    }

    #[test]
    fn json_refuses_what_is_broken() {
        for bad in ["", "{", "[1,", "{\"a\" 1}", "[1 2]", "\"abc", "tru", "{\"a\":1} x", "\"\\q\"", "[\"\\u12\"]"] {
            assert!(parse_json(bad).is_err(), "{bad:?} should not parse");
        }
        // A lone half of a surrogate pair becomes the replacement character, not a panic.
        assert_eq!(parse_json("\"\\ud83d\"").unwrap(), Json::Str("\u{FFFD}".into()));
        // Deep nesting stops instead of overflowing the stack.
        let deep = "[".repeat(200) + &"]".repeat(200);
        assert!(parse_json(&deep).is_err());
    }

    const SEARCH: &str = r#"{"hits":[
        {"project_id":"AANobbMI","slug":"faithful-32x","author":"Faithful Team","title":"Faithful 32x","description":"A pack \"for\" you","downloads":1234567,"follows":8900,
         "icon_url":"https://cdn.modrinth.com/data/AANobbMI/icon.png","display_categories":["32x","vanilla-like"],"project_type":"resourcepack"},
        {"project_id":"x2","title":"Plain","author":"me","description":"","downloads":12,"follows":0,"icon_url":null,"display_categories":[]},
        {"project_id":"x3","title":"Evil","author":"it","description":"","downloads":1,"follows":1,"icon_url":"https://evil.example/icon.png","display_categories":[]},
        {"title":"no id"}
      ],"offset":0,"limit":20,"total_hits":312}"#;

    #[test]
    fn a_search_answer_becomes_packs() {
        let (hits, total) = parse_search(SEARCH).unwrap();
        assert_eq!(total, 312);
        assert_eq!(hits.len(), 3, "a hit without an id is skipped");
        assert_eq!(hits[0].title, "Faithful 32x");
        assert_eq!(hits[0].author, "Faithful Team");
        assert_eq!(hits[0].summary, "A pack \"for\" you");
        assert_eq!(hits[0].downloads, 1_234_567);
        assert_eq!(hits[0].categories, ["32x", "vanilla-like"]);
        assert_eq!(hits[0].icon_url.as_deref(), Some("https://cdn.modrinth.com/data/AANobbMI/icon.png"));
        assert_eq!(hits[1].icon_url, None);
        assert_eq!(hits[2].icon_url, None, "an icon from another host is not fetched");
        assert!(parse_search(r#"{"error":"invalid_input","description":"bad facets"}"#).unwrap_err().contains("bad facets"));
        assert!(parse_search("[]").is_err());
        assert!(parse_search("nope").is_err());
    }

    const VERSIONS: &str = r#"[
        {"id":"v2","name":"Release 2","version_number":"2.0","version_type":"release","game_versions":["1.21","26.2","26.3"],"date_published":"2026-10-01T10:00:00Z",
         "files":[{"hashes":{"sha1":"ABCDEF0123456789ABCDEF0123456789ABCDEF01","sha512":"x"},"url":"https://cdn.modrinth.com/data/a/versions/v2/pack.zip","filename":"Pack 2.0.zip","primary":false,"size":10},
                  {"hashes":{"sha1":"0123456789abcdef0123456789abcdef01234567"},"url":"https://cdn.modrinth.com/data/a/versions/v2/Pack.zip","filename":"Pack.zip","primary":true,"size":2048}]},
        {"id":"v1","name":"Beta","version_number":"1.9","version_type":"beta","game_versions":["26.3"],"date_published":"2026-09-01T10:00:00Z","files":[]}
      ]"#;

    #[test]
    fn a_version_answer_picks_the_primary_file() {
        let v = parse_versions(VERSIONS).unwrap();
        assert_eq!(v.len(), 2);
        let f = v[0].file.as_ref().unwrap();
        assert_eq!(f.filename, "Pack.zip");
        assert_eq!(f.sha1, "0123456789abcdef0123456789abcdef01234567");
        assert_eq!(f.size, 2048);
        assert_eq!(v[0].number, "2.0");
        assert_eq!(v[0].games, ["1.21", "26.2", "26.3"]);
        assert!(v[1].file.is_none());
        assert_eq!(v[1].kind, "beta");
        assert!(parse_versions(r#"{"error":"not_found","description":"no such project"}"#).is_err());
        assert!(parse_versions("{}").is_err());
        assert_eq!(parse_versions("[]").unwrap(), vec![]);
    }

    #[test]
    fn a_file_is_only_taken_when_it_is_safe() {
        let ok = File {
            url: "https://cdn.modrinth.com/data/a/versions/v/Pack.zip".into(),
            filename: "Pack.zip".into(),
            size: 1000,
            sha1: "0123456789abcdef0123456789abcdef01234567".into(),
        };
        assert!(file_ok(&ok).is_ok());
        assert!(file_ok(&File { url: "https://evil.example/Pack.zip".into(), ..ok.clone() }).is_err());
        assert!(file_ok(&File { url: "http://cdn.modrinth.com/x.zip".into(), ..ok.clone() }).is_err());
        assert!(file_ok(&File { sha1: "short".into(), ..ok.clone() }).is_err());
        assert!(file_ok(&File { sha1: "z".repeat(40), ..ok.clone() }).is_err());
        assert!(file_ok(&File { size: MAX_PACK_BYTES + 1, ..ok.clone() }).is_err());
        assert!(file_ok(&File { filename: "pack.mrpack".into(), ..ok.clone() }).is_err());
    }

    #[test]
    fn the_questions_are_built_right() {
        let url = search_url("faithful 32x", Sort::Downloads, Some("26.3"), 40);
        assert!(url.starts_with("https://api.modrinth.com/v2/search?query=faithful%2032x&facets="), "{url}");
        assert!(url.contains("%22project_type%3Aresourcepack%22"), "{url}");
        assert!(url.contains("%22versions%3A26.3%22"), "{url}");
        assert!(url.ends_with("&index=downloads&limit=20&offset=40"), "{url}");
        let all = search_url("  ", Sort::Relevance, None, 0);
        assert!(all.contains("query=&facets") && !all.contains("versions"), "{all}");
        assert_eq!(
            versions_url("AANobbMI", Some("26.3")),
            "https://api.modrinth.com/v2/project/AANobbMI/version?game_versions=%5B%2226.3%22%5D"
        );
        assert_eq!(versions_url("a/b", None), "https://api.modrinth.com/v2/project/a%2Fb/version");
        assert_eq!(encode("a b&c=d/é"), "a%20b%26c%3Dd%2F%C3%A9");
        let mut s = Sort::Relevance;
        for _ in 0..4 {
            s = s.next();
        }
        assert_eq!(s, Sort::Relevance);
        assert_eq!(Sort::Newest.index(), "newest");
    }

    #[test]
    fn file_names_are_made_safe() {
        assert_eq!(safe_filename("Pack 2.0.zip").as_deref(), Some("Pack 2.0.zip"));
        assert_eq!(safe_filename("../../evil.zip").as_deref(), Some("evil.zip"));
        assert_eq!(safe_filename("C:\\x\\y\\evil.ZIP").as_deref(), Some("evil.ZIP"));
        assert_eq!(safe_filename("a:b*c?.zip").as_deref(), Some("a_b_c_.zip"));
        assert_eq!(safe_filename("Pack Ü.zip").as_deref(), Some("Pack _.zip"));
        assert_eq!(safe_filename("...zip"), None, "nothing but the extension");
        assert_eq!(safe_filename(".zip"), None);
        assert_eq!(safe_filename("pack.jar"), None);
        assert_eq!(safe_filename(""), None);
        assert_eq!(safe_filename(&format!("{}.zip", "a".repeat(200))), None);
        let taken = |n: &str| ["a.zip", "a (2).zip"].contains(&n);
        assert_eq!(unique_name("b.zip", &taken), "b.zip");
        assert_eq!(unique_name("a.zip", &taken), "a (3).zip");
    }

    #[test]
    fn the_list_of_installed_packs_survives_a_round_trip() {
        let mut records = vec![Record { project: "p1".into(), version: "v1".into(), filename: "A.zip".into() }];
        put_record(&mut records, Record { project: "p2".into(), version: "v9".into(), filename: "B\tC.zip".into() });
        put_record(&mut records, Record { project: "p1".into(), version: "v2".into(), filename: "A 2.zip".into() });
        assert_eq!(records.len(), 2, "one record per pack");
        let text = format_records(&records);
        let back = parse_records(&text);
        assert_eq!(back.len(), 2);
        assert_eq!(back[1], Record { project: "p1".into(), version: "v2".into(), filename: "A 2.zip".into() });
        assert_eq!(back[0].filename, "B C.zip", "a tab in a name can't break the file");
        assert!(parse_records("garbage\n\n\t\t\nonly\ttwo\n").is_empty());
    }

    #[test]
    fn numbers_read_as_words() {
        assert_eq!(count(0), "0");
        assert_eq!(count(999), "999");
        assert_eq!(count(1_000), "1K");
        assert_eq!(count(1_234), "1.2K");
        assert_eq!(count(999_999), "1000K");
        assert_eq!(count(1_234_567), "1.2M");
        assert_eq!(count(2_500_000_000), "2.5B");
        assert_eq!(human_size(812), "812 B");
        assert_eq!(human_size(1536), "1.5 KiB");
        assert_eq!(human_size(262_144_000), "250.0 MiB");
        assert_eq!(percent(50, 200), Some(25));
        assert_eq!(percent(300, 200), Some(100));
        assert_eq!(percent(1, 0), None);
    }

    #[test]
    fn the_layout_fits_every_screen_size() {
        for (w, h) in [(320.0, 240.0), (427.0, 240.0), (480.0, 270.0), (640.0, 360.0), (960.0, 540.0), (1280.0, 720.0)] {
            let l = layout(w, h);
            assert!(l.search.x + l.search.w <= l.list.w, "{w}x{h}: search leaves the list");
            assert!(l.list.x + l.list.w <= l.detail.x, "{w}x{h}: list and detail overlap");
            assert!(l.detail.x + l.detail.w <= w, "{w}x{h}: detail leaves the screen");
            assert!(l.sort.x + l.sort.w < l.game.x, "{w}x{h}: the two buttons overlap");
            assert!(l.game.x + l.game.w <= w, "{w}x{h}: a button leaves the screen");
            assert!(l.list.y + l.list.h <= l.done.y, "{w}x{h}: list reaches the button");
            assert!(l.done.x >= 0.0 && l.done.x + l.done.w <= w, "{w}x{h}: Done leaves the screen");
            assert!(l.done.y + l.done.h <= l.hint_y && l.hint_y + 8.0 <= h, "{w}x{h}: small print is cut");
            let plus = plus_button(w);
            assert!(plus.x + plus.w <= w && plus.x >= 0.0);
        }
    }

    #[test]
    fn rows_scroll_and_get_picked() {
        let l = layout(427.0, 240.0);
        assert_eq!(max_scroll(2, l.list.h), 0.0);
        assert!(max_scroll(100, l.list.h) > 0.0);
        assert_eq!(row_at(&l, 0.0, 5, 10.0, l.list.y + 1.0), Some(0));
        assert_eq!(row_at(&l, 0.0, 5, 10.0, l.list.y + ROW + 1.0), Some(1));
        assert_eq!(row_at(&l, 0.0, 5, 10.0, l.list.y + 6.0 * ROW), None, "past the last row");
        assert_eq!(row_at(&l, 0.0, 5, l.list.w - 3.0, l.list.y + 1.0), None, "the scroll bar is not a row");
        assert_eq!(row_at(&l, ROW, 5, 10.0, l.list.y + 1.0), Some(1), "scrolled");
        assert_eq!(row_at(&l, 0.0, 5, 10.0, l.list.y - 1.0), None);
        assert_eq!(step(5, None, 1), Some(0));
        assert_eq!(step(5, None, -1), Some(4));
        assert_eq!(step(5, Some(4), 1), Some(4));
        assert_eq!(step(0, None, 1), None);
        assert_eq!(scroll_into_view(0.0, 10, 100.0), 10.0 * ROW + ROW - 100.0);
        assert_eq!(scroll_into_view(500.0, 2, 100.0), 2.0 * ROW);
        assert_eq!(scroll_into_view(0.0, 1, 100.0), 0.0);
    }

    #[test]
    fn the_next_page_is_asked_for_near_the_end() {
        assert!(!near_end(0.0, 0, 100.0), "nothing listed yet");
        assert!(near_end(0.0, 3, 100.0), "everything fits");
        assert!(!near_end(0.0, 20, 100.0));
        assert!(near_end(20.0 * ROW - 100.0, 20, 100.0));
        assert!(near_end(20.0 * ROW - 100.0 - ROW, 20, 100.0), "a row early");
    }

    #[test]
    fn text_wraps_to_the_width() {
        assert_eq!(wrap("hello brave new world", 66.0, &six), ["hello brave", "new world"]);
        assert_eq!(wrap("a\n\nb", 100.0, &six), ["a", "", "b"]);
        assert!(wrap("aaaaaaaaaaaaaaaaaaaa", 30.0, &six).iter().all(|l| six(l) <= 30.0));
        assert_eq!(wrap("", 100.0, &six), Vec::<String>::new());
    }

    #[test]
    fn long_text_is_cut_to_fit() {
        assert_eq!(ellipsize("short", 100.0, &six), "short");
        let cut = ellipsize("a rather long pack name", 60.0, &six);
        assert!(cut.ends_with("...") && six(&cut) <= 60.0, "{cut}");
        assert_eq!(tail_fit("hello world", 36.0, &six), " world");
        assert_eq!(tail_fit("hi", 100.0, &six), "hi");
    }

    #[test]
    fn the_search_box_takes_what_you_type() {
        let mut s = String::new();
        type_text(&mut s, "foo\n bar\u{7}");
        assert_eq!(s, "foo bar");
        backspace(&mut s, true);
        assert_eq!(s, "foo ");
        backspace(&mut s, true);
        assert_eq!(s, "");
        type_text(&mut s, "ab");
        backspace(&mut s, false);
        assert_eq!(s, "a");
        type_text(&mut s, &"x".repeat(200));
        assert_eq!(s.chars().count(), MAX_SEARCH);
    }

    #[test]
    fn icons_keep_their_colour() {
        assert_eq!(icon_rgb("AANobbMI"), icon_rgb("AANobbMI"));
        assert_ne!(icon_rgb("AANobbMI"), icon_rgb("x2"));
        assert_eq!(shade([100, 200, 250], 0.5), [50, 100, 125]);
    }

    fn two_versions() -> Vec<Version> {
        parse_versions(VERSIONS).unwrap()
    }

    fn a_hit() -> Hit {
        parse_search(SEARCH).unwrap().0.remove(0)
    }

    #[test]
    fn a_version_is_picked_by_choice_then_release_then_first() {
        let v = two_versions();
        assert_eq!(pick_version(&v, None), Some(0));
        assert_eq!(pick_version(&v, Some("v1")), Some(1));
        assert_eq!(pick_version(&v, Some("gone")), Some(0));
        assert_eq!(pick_version(&[], None), None);
        let mut only_beta = v;
        only_beta.remove(0);
        assert_eq!(pick_version(&only_beta, None), Some(0), "no release: the newest");
        assert_eq!(games_short(&["1.20".into(), "1.21".into(), "26.1".into(), "26.2".into(), "26.3".into()]), "26.3, 26.2, 26.1 +2");
        assert_eq!(games_short(&["26.3".into()]), "26.3");
    }

    #[test]
    fn the_detail_pane_lists_versions_then_the_pack() {
        let v = two_versions();
        let lines = body_lines(&a_hit(), Versions::Loaded(&v), Some(0), Some("26.3"), 300.0, &six);
        let texts: Vec<&str> = lines.iter().map(|l| l.text.as_str()).collect();
        assert_eq!(texts[0], "Versions");
        assert_eq!(texts[1], "> 2.0 (26.3, 26.2, 1.21)");
        assert_eq!(texts[2], "  1.9 [beta] (26.3)");
        assert_eq!(lines[1].tone, Tone::Heading);
        assert_eq!(lines[2].tone, Tone::Text);
        assert_eq!(lines[1].version, Some(0));
        assert_eq!(lines[2].version, Some(1));
        assert!(texts.contains(&"A pack \"for\" you"));
        assert!(texts.iter().any(|t| t.starts_with("Tags: 32x, vanilla-like")));
        // A click on the second version line picks it; the heading and the text pick nothing.
        let body_y = 100.0;
        assert_eq!(version_at(&lines, body_y, body_y + LINE + 1.0), Some(0));
        assert_eq!(version_at(&lines, body_y, body_y + 2.0 * LINE + 1.0), Some(1));
        assert_eq!(version_at(&lines, body_y, body_y + 1.0), None);
        assert_eq!(version_at(&lines, body_y, body_y - 5.0), None);
        assert_eq!(version_at(&lines, body_y, body_y + 500.0), None);
        let loading = body_lines(&a_hit(), Versions::Loading, None, None, 300.0, &six);
        assert_eq!(loading[1].text, "Loading...");
        let failed = body_lines(&a_hit(), Versions::Failed("no internet"), None, None, 300.0, &six);
        assert!(failed.iter().any(|l| l.tone == Tone::Bad && l.text.contains("no internet")));
        let empty = body_lines(&a_hit(), Versions::Loaded(&[]), None, Some("26.3"), 300.0, &six);
        assert!(empty.iter().any(|l| l.text.contains("26.3")));
        assert!(lines.iter().all(|l| six(&l.text) <= 300.0));
        let many: Vec<Version> = (0..20).map(|i| Version { id: format!("v{i}"), ..two_versions().remove(0) }).collect();
        let lines = body_lines(&a_hit(), Versions::Loaded(&many), None, None, 300.0, &six);
        assert_eq!(lines.iter().filter(|l| l.version.is_some()).count(), 12);
        assert!(lines.iter().any(|l| l.text == "... and 8 older"));
    }

    #[test]
    fn the_detail_pane_scrolls_with_its_contents() {
        let l = layout(427.0, 240.0);
        let top = detail(&l, 0.0);
        let scrolled = detail(&l, 12.0);
        assert_eq!(top.icon.y - scrolled.icon.y, 12.0);
        assert_eq!(top.install.y - scrolled.install.y, 12.0);
        assert_eq!(top.body[1] - scrolled.body[1], 12.0);
        assert!(top.install.x + top.install.w < top.status[0]);
        assert!(top.install.y + top.install.h < top.body[1]);
        assert!(top.width <= l.detail.w);
        assert_eq!(detail_height(0), 76.0);
        assert_eq!(detail_height(3), 106.0);
    }

    #[test]
    fn the_status_line_says_what_is_going_on() {
        assert_eq!(status_text(true, None, 0, 0), "Searching...");
        assert_eq!(status_text(false, None, 0, 0), "No packs found");
        assert_eq!(status_text(false, None, 20, 312), "Showing 20 of 312 packs");
        assert_eq!(status_text(false, None, 1, 1), "Showing 1 of 1 pack");
        assert_eq!(status_text(true, None, 20, 312), "Showing 20 of 312 packs ...");
        assert_eq!(status_text(false, Some("No internet"), 20, 312), "No internet");
    }

    #[test]
    fn the_flags_and_the_guard() {
        // One test: the flags are shared by the whole process.
        close();
        take_fresh();
        assert!(!is_open() && !take_fresh());
        open();
        assert!(is_open() && take_fresh());
        assert!(!take_fresh(), "fresh only once");
        close();
        assert!(!is_open());
        let guard = Guard::new();
        assert!(!guard.enter(), "the first call is the hook's");
        assert!(guard.enter(), "the call the hook made finds it set");
        guard.leave();
        assert!(!guard.enter(), "and it is free again afterwards");
    }

    #[test]
    fn the_plus_button_sits_in_the_top_right() {
        let b = plus_button(427.0);
        assert!(b.contains(b.x + 1.0, b.y + 1.0) && !b.contains(b.x - 1.0, b.y + 1.0));
        assert_eq!(b.w, BUTTON_H);
        assert!(b.x > 427.0 / 2.0);
    }
}
