// Resourcify's browser screen. It is drawn in place of the Options screen while the "+" button on
// the Resource Packs screen has asked for it (`native_resourcify`), so Horizon's own window, mouse,
// wheel, keys and Escape all reach it through the Options screen's hooks.
// The JSON, the layout and the checks are in `native_resourcify`; this file draws them and talks
// to Modrinth on threads of its own, so the game never waits for the network.
use super::native_resourcify as r;
use super::Game;
use horizon_client::{
    Result,
    clipboard::EditShortcut,
    hud::{HudAtlas, HudBatch, PACK_ICON_SLOTS, rgba},
    options_ui::Event,
    text::Text,
    ui,
};
use std::{
    collections::{BTreeMap, BTreeSet},
    sync::{
        Arc, Mutex, MutexGuard, OnceLock,
        mpsc::{Receiver, Sender, channel},
    },
    time::{Duration, Instant},
};
use winit::keyboard::KeyCode;

type Res<T> = std::result::Result<T, String>;

/// How a pack's versions are doing.
enum VState {
    Loading,
    Failed(String),
    Loaded(Vec<r::Version>),
}

/// The install that is running.
struct Install {
    project: String,
    done: u64,
    total: u64,
}

/// What the screen remembers while it is open.
struct State {
    query: String,
    /// When the search box last changed; the search starts a moment after.
    last_change: Option<Instant>,
    /// What the packs on screen were asked for: text, order, only this game version.
    searched: Option<(String, r::Sort, bool)>,
    sort: r::Sort,
    only_game: bool,
    /// Which search the answers belong to (an older answer is thrown away).
    generation: u64,
    loading: bool,
    error: Option<String>,
    hits: Vec<r::Hit>,
    total: u64,
    selected: Option<usize>,
    list_scroll: f32,
    detail_scroll: f32,
    /// By project and game filter.
    versions: BTreeMap<String, VState>,
    /// The version the player picked, by project.
    chosen: BTreeMap<String, String>,
    /// The pictures, 128 by 128 RGBA; none when the picture would not load.
    icons: BTreeMap<String, Option<Arc<[u8]>>>,
    icons_loading: BTreeSet<String>,
    install: Option<Install>,
    notice: Option<(String, r::Tone)>,
    records: Vec<r::Record>,
    records_read: bool,
}

impl State {
    const fn new() -> Self {
        State {
            query: String::new(),
            last_change: None,
            searched: None,
            sort: r::Sort::Relevance,
            only_game: true,
            generation: 0,
            loading: false,
            error: None,
            hits: Vec::new(),
            total: 0,
            selected: None,
            list_scroll: 0.0,
            detail_scroll: 0.0,
            versions: BTreeMap::new(),
            chosen: BTreeMap::new(),
            icons: BTreeMap::new(),
            icons_loading: BTreeSet::new(),
            install: None,
            notice: None,
            records: Vec::new(),
            records_read: false,
        }
    }

    /// A browser that has just been asked for starts with an empty search; the pictures, the
    /// versions and the installed list stay.
    fn restart(&mut self) {
        self.query.clear();
        self.last_change = None;
        self.searched = None;
        self.sort = r::Sort::Relevance;
        self.only_game = true;
        self.generation += 1;
        self.loading = false;
        self.error = None;
        self.hits.clear();
        self.total = 0;
        self.selected = None;
        self.list_scroll = 0.0;
        self.detail_scroll = 0.0;
        self.notice = None;
    }

    fn game_filter(&self) -> Option<&'static str> {
        self.only_game.then_some(horizon_core::version::VANILLA_VERSION)
    }

    fn versions_key(&self, project: &str) -> String {
        format!("{project}|{}", self.game_filter().unwrap_or("all"))
    }
}

/// What the threads tell the screen.
enum Msg {
    Search { generation: u64, append: bool, result: Res<(Vec<r::Hit>, u64)> },
    Versions { key: String, result: Res<Vec<r::Version>> },
    Icon { project: String, rgba: Option<Arc<[u8]>> },
    Progress { project: String, done: u64, total: u64 },
    Installed { project: String, result: Res<r::Record> },
}

fn inbox() -> &'static (Sender<Msg>, Mutex<Receiver<Msg>>) {
    static INBOX: OnceLock<(Sender<Msg>, Mutex<Receiver<Msg>>)> = OnceLock::new();
    INBOX.get_or_init(|| {
        let (tx, rx) = channel();
        (tx, Mutex::new(rx))
    })
}

static STATE: Mutex<State> = Mutex::new(State::new());

/// The state, with what the threads have said put in; a browser just asked for starts over.
fn state() -> MutexGuard<'static, State> {
    let mut st = STATE.lock().unwrap_or_else(|e| e.into_inner());
    if r::take_fresh() {
        st.restart();
    }
    if !st.records_read {
        st.records = r::parse_records(&std::fs::read_to_string(records_path()).unwrap_or_default());
        st.records_read = true;
    }
    let rx = inbox().1.lock().unwrap_or_else(|e| e.into_inner());
    while let Ok(msg) = rx.try_recv() {
        apply(&mut st, msg);
    }
    st
}

fn apply(st: &mut State, msg: Msg) {
    match msg {
        Msg::Search { generation, append, result } => {
            if generation != st.generation {
                return;
            }
            st.loading = false;
            match result {
                Ok((hits, total)) => {
                    st.error = None;
                    st.total = total;
                    if append {
                        let known: BTreeSet<String> = st.hits.iter().map(|h| h.id.clone()).collect();
                        st.hits.extend(hits.into_iter().filter(|h| !known.contains(&h.id)));
                    } else {
                        st.hits = hits;
                        st.list_scroll = 0.0;
                        st.selected = None;
                    }
                }
                Err(e) => st.error = Some(e),
            }
        }
        Msg::Versions { key, result } => {
            st.versions.insert(key, match result {
                Ok(list) => VState::Loaded(list),
                Err(e) => VState::Failed(e),
            });
        }
        Msg::Icon { project, rgba } => {
            st.icons_loading.remove(&project);
            st.icons.insert(project, rgba);
        }
        Msg::Progress { project, done, total } => {
            if let Some(i) = st.install.as_mut().filter(|i| i.project == project) {
                i.done = done;
                i.total = total;
            }
        }
        Msg::Installed { project, result } => {
            if st.install.as_ref().is_some_and(|i| i.project == project) {
                st.install = None;
            }
            match result {
                Ok(record) => {
                    st.notice = Some((format!("Installed {}. It is in your Resource Packs list.", record.filename), r::Tone::Good));
                    r::put_record(&mut st.records, record);
                    let _ = std::fs::create_dir_all(records_path().parent().unwrap_or(std::path::Path::new(".")));
                    let _ = std::fs::write(records_path(), r::format_records(&st.records));
                }
                Err(e) => st.notice = Some((e, r::Tone::Bad)),
            }
        }
    }
}

fn records_path() -> std::path::PathBuf {
    std::env::current_dir().unwrap_or_default().join("resourcify").join("installed.txt")
}

fn color(rgb: u32) -> u32 {
    rgba(((rgb >> 16) & 0xFF) as u8, ((rgb >> 8) & 0xFF) as u8, (rgb & 0xFF) as u8, 255)
}

fn rgb_of(c: [u8; 3]) -> u32 {
    rgba(c[0], c[1], c[2], 255)
}

fn tone_color(tone: r::Tone) -> u32 {
    match tone {
        r::Tone::Text => color(0xD8D8D8),
        r::Tone::Dim => color(0x8C8C8C),
        r::Tone::Good => color(0x7CE08A),
        r::Tone::Bad => color(0xFF7070),
        r::Tone::Heading => color(0xFFFFFF),
    }
}

// ---------------------------------------------------------------------------------------------
// The network, on threads of their own.

fn get(url: &str) -> Res<ureq::Response> {
    ureq::AgentBuilder::new()
        .timeout_connect(Duration::from_secs(10))
        .timeout_read(Duration::from_secs(20))
        .build()
        .get(url)
        .set("User-Agent", r::AGENT)
        .call()
        .map_err(|e| match e {
            ureq::Error::Status(code, _) => format!("Modrinth answered {code}"),
            _ => "No connection to Modrinth".to_owned(),
        })
}

fn get_text(url: &str) -> Res<String> {
    get(url)?.into_string().map_err(|_| "Modrinth's answer could not be read".to_owned())
}

fn spawn(name: &str, work: impl FnOnce() + Send + 'static) {
    let _ = std::thread::Builder::new().name(name.into()).spawn(work);
}

fn start_search(st: &mut State, append: bool) {
    let key = (st.query.trim().to_owned(), st.sort, st.only_game);
    let offset = if append { st.hits.len() } else { 0 };
    if !append {
        st.generation += 1;
        st.searched = Some(key.clone());
        st.error = None;
    }
    st.loading = true;
    let generation = st.generation;
    let url = r::search_url(&key.0, key.1, st.game_filter(), offset);
    spawn("resourcify-search", move || {
        let result = get_text(&url).and_then(|body| r::parse_search(&body));
        let _ = inbox().0.send(Msg::Search { generation, append, result });
    });
}

fn start_versions(st: &mut State, project: &str) {
    let key = st.versions_key(project);
    if st.versions.contains_key(&key) {
        return;
    }
    st.versions.insert(key.clone(), VState::Loading);
    let url = r::versions_url(project, st.game_filter());
    spawn("resourcify-versions", move || {
        let result = get_text(&url).and_then(|body| r::parse_versions(&body));
        let _ = inbox().0.send(Msg::Versions { key, result });
    });
}

fn start_icon(st: &mut State, project: &str, url: &str) {
    if st.icons.contains_key(project) || st.icons_loading.contains(project) || st.icons_loading.len() >= 6 {
        return;
    }
    if !r::icon_url_ok(url) {
        st.icons.insert(project.to_owned(), None);
        return;
    }
    st.icons_loading.insert(project.to_owned());
    let (project, url) = (project.to_owned(), url.to_owned());
    spawn("resourcify-icon", move || {
        let picture = get(&url).ok().and_then(|response| {
            use std::io::Read;
            let mut bytes = Vec::new();
            response.into_reader().take(2 * 1024 * 1024).read_to_end(&mut bytes).ok()?;
            horizon_client::pack_screen::load_icon(&bytes)
        });
        let _ = inbox().0.send(Msg::Icon { project, rgba: picture });
    });
}

/// Downloads the picked version into `resourcepacks/`.
fn start_install(st: &mut State, hit: &r::Hit, version: &r::Version) {
    let Some(file) = version.file.clone() else {
        st.notice = Some(("This version has no file.".into(), r::Tone::Bad));
        return;
    };
    if let Err(e) = r::file_ok(&file) {
        st.notice = Some((e, r::Tone::Bad));
        return;
    }
    st.notice = None;
    st.install = Some(Install { project: hit.id.clone(), done: 0, total: file.size });
    let (project, version_id) = (hit.id.clone(), version.id.clone());
    spawn("resourcify-install", move || {
        let result = install(&project, &version_id, &file);
        let _ = inbox().0.send(Msg::Installed { project, result });
    });
}

fn install(project: &str, version_id: &str, file: &r::File) -> Res<r::Record> {
    use horizon_client::server_packs::{Request, download::{Progress, download_file}};
    let dir = std::env::current_dir().unwrap_or_default().join("downloads").join("resourcify");
    let (tx, rx) = channel::<Progress>();
    let (watch_project, total) = (project.to_owned(), file.size);
    spawn("resourcify-progress", move || {
        let mut size = total;
        for step in rx {
            match step {
                Progress::DownloadStart(Some(len)) => size = len,
                Progress::Bytes(done) => {
                    let _ = inbox().0.send(Msg::Progress { project: watch_project.clone(), done, total: size });
                }
                _ => {}
            }
        }
    });
    let request = Request { id: 0, url: file.url.clone(), sha1: Some(file.sha1.clone()) };
    let headers = vec![("User-Agent", r::AGENT.to_owned())];
    let cached = download_file(&dir, &request, &headers, r::MAX_PACK_BYTES, &tx)
        .map_err(|e| format!("Download failed: {e}"))?;
    drop(tx);
    let packs = super::packs::packs_dir();
    std::fs::create_dir_all(&packs).map_err(|e| format!("Could not make the resourcepacks folder: {e}"))?;
    let wanted = r::safe_filename(&file.filename).ok_or("The file is not a .zip resource pack.")?;
    let name = r::unique_name(&wanted, &|n| packs.join(n).exists());
    let part = packs.join(format!("{name}.part"));
    std::fs::copy(&cached, &part).map_err(|e| format!("Could not save the pack: {e}"))?;
    std::fs::rename(&part, packs.join(&name)).map_err(|e| {
        let _ = std::fs::remove_file(&part);
        format!("Could not save the pack: {e}")
    })?;
    Ok(r::Record { project: project.to_owned(), version: version_id.to_owned(), filename: name })
}

/// Starts the search when the box has been quiet for a moment, and the next page near the end.
fn keep_going(st: &mut State, list_h: f32) {
    let key = (st.query.trim().to_owned(), st.sort, st.only_game);
    let changed = st.searched.as_ref() != Some(&key);
    let quiet = st.last_change.is_none_or(|t| t.elapsed().as_millis() >= r::DEBOUNCE_MS);
    if changed && quiet {
        // An answer for older text may still be on its way; it is thrown away when it comes.
        start_search(st, false);
    } else if !changed
        && !st.loading
        && st.error.is_none()
        && (st.hits.len() as u64) < st.total
        && r::near_end(st.list_scroll, st.hits.len(), list_h)
    {
        start_search(st, true);
    }
}

// ---------------------------------------------------------------------------------------------
// Drawing.

/// The notice under the Install button, in at most three lines.
fn notice_lines(st: &State, width: f32, measure: &dyn Fn(&str) -> f32) -> Vec<String> {
    st.notice.as_ref().map_or_else(Vec::new, |(text, _)| r::wrap(text, width, measure).into_iter().take(3).collect())
}

/// How far the notice pushes the versions and the description down.
fn notice_height(lines: &[String]) -> f32 {
    if lines.is_empty() { 0.0 } else { lines.len() as f32 * r::LINE + 4.0 }
}

/// The pictures drawn this frame: slot to project.
struct Slots {
    by_project: BTreeMap<String, usize>,
    patches: Vec<horizon_client::font::Patch>,
}

impl Slots {
    fn new() -> Self {
        Slots { by_project: BTreeMap::new(), patches: Vec::new() }
    }

    /// The sprite for a project's picture, or none while it is not there.
    fn sprite(&mut self, atlas: &HudAtlas, st: &State, project: &str) -> Option<String> {
        let picture = st.icons.get(project)?.as_ref()?;
        let next = self.by_project.len();
        let slot = *self.by_project.entry(project.to_owned()).or_insert(next);
        if slot >= PACK_ICON_SLOTS {
            return None;
        }
        let sprite = format!("horizon/pack-icon{slot}");
        if self.patches.len() <= slot {
            let rect = atlas.sprites.get(&sprite)?;
            self.patches.push(horizon_client::font::Patch {
                x: (rect.u0 * atlas.width as f32).round() as u32,
                y: (rect.v0 * atlas.height as f32).round() as u32,
                width: horizon_client::hud::PACK_ICON_SIZE,
                height: horizon_client::hud::PACK_ICON_SIZE,
                rgba: picture.to_vec(),
            });
        }
        Some(sprite)
    }
}

fn draw_icon(batch: &mut HudBatch, atlas: &HudAtlas, slots: &mut Slots, st: &State, hit: &r::Hit, x: f32, y: f32) {
    if let Some(sprite) = slots.sprite(atlas, st, &hit.id) {
        batch.sprite_scaled(atlas, &sprite, x, y, r::ICON, r::ICON, u32::MAX);
        return;
    }
    // While the picture is not there (or never will be): a tile in the pack's colour with its first letter.
    let base = r::icon_rgb(&hit.id);
    batch.fill(atlas, x, y, r::ICON, r::ICON, rgba(70, 70, 82, 255));
    batch.fill(atlas, x + 1.0, y + 1.0, r::ICON - 2.0, r::ICON - 2.0, rgb_of(r::shade(base, 0.6)));
    let letter: String = hit.title.chars().next().map(|c| c.to_uppercase().collect()).unwrap_or_default();
    let lw = HudBatch::text_width(atlas, &letter);
    batch.text(atlas, &letter, x + ((r::ICON - lw) / 2.0).floor(), y + 12.0, u32::MAX, true);
}

#[allow(clippy::too_many_arguments)]
fn draw_row(
    batch: &mut HudBatch,
    atlas: &HudAtlas,
    slots: &mut Slots,
    st: &State,
    hit: &r::Hit,
    x: f32,
    y: f32,
    w: f32,
    selected: bool,
    hover: bool,
) {
    let measure = |s: &str| HudBatch::text_width(atlas, s);
    if selected {
        batch.fill(atlas, x + 2.0, y, w - 4.0, r::ROW, u32::MAX);
        batch.fill(atlas, x + 3.0, y + 1.0, w - 6.0, r::ROW - 2.0, rgba(0, 0, 0, 255));
    } else if hover {
        batch.fill(atlas, x + 2.0, y, w - 4.0, r::ROW, rgba(255, 255, 255, 40));
    }
    draw_icon(batch, atlas, slots, st, hit, x + 4.0, y + 2.0);
    let text_x = x + 4.0 + r::ICON + 4.0;
    let avail = (w - (text_x - x) - 8.0).max(30.0);
    let installed = st.records.iter().any(|rec| rec.project == hit.id);
    let tag = if installed { " (installed)" } else { "" };
    let name = r::ellipsize(&hit.title, (avail - measure(tag)).max(30.0), &measure);
    let name_w = batch.text(atlas, &name, text_x, y + 4.0, u32::MAX, true);
    if installed {
        batch.text(atlas, tag, text_x + name_w, y + 4.0, tone_color(r::Tone::Good), true);
    }
    let by = r::ellipsize(&format!("by {} - {} downloads", hit.author, r::count(hit.downloads)), avail, &measure);
    batch.text(atlas, &by, text_x, y + 14.0, color(0xA0A0A0), true);
    let about = r::ellipsize(hit.summary.trim(), avail, &measure);
    batch.text(atlas, &about, text_x, y + 24.0, color(0x808080), true);
}

pub fn draw(game: &mut Game<'_>, batch: &mut HudBatch) {
    let mut st = state();
    let (w, h) = (batch.width, batch.height);
    let l = r::layout(w, h);
    keep_going(&mut st, l.list.h);
    st.list_scroll = st.list_scroll.clamp(0.0, r::max_scroll(st.hits.len(), l.list.h));
    if st.selected.is_none() && !st.hits.is_empty() {
        st.selected = Some(0);
        st.detail_scroll = 0.0;
    }
    let selected_hit = st.selected.and_then(|i| st.hits.get(i)).cloned();
    if let Some(hit) = &selected_hit {
        start_versions(&mut st, &hit.id);
    }
    // Pictures for the rows on screen and for the picked pack.
    let first_row = (st.list_scroll / r::ROW).floor() as usize;
    let last_row = (((st.list_scroll + l.list.h) / r::ROW).ceil() as usize + 1).min(st.hits.len());
    let wanted: Vec<(String, String)> = st
        .hits
        .iter()
        .enumerate()
        .filter(|(i, _)| (first_row..last_row).contains(i) || Some(*i) == st.selected)
        .filter_map(|(_, hit)| Some((hit.id.clone(), hit.icon_url.clone()?)))
        .collect();
    for (id, url) in wanted {
        start_icon(&mut st, &id, &url);
    }

    let atlas = &game.gui.atlas;
    let cursor = game.cursor;
    let in_world = game.title.is_none();
    let measure = |s: &str| HudBatch::text_width(atlas, s);
    let mut slots = Slots::new();

    ui::menu_background(batch, atlas, 0.0, 0.0, w, h, in_world);
    ui::list_background(batch, atlas, l.list.y, l.list.y + l.list.h, in_world);

    // The search box, always ready for typing.
    ui::nine_slice(
        batch,
        atlas,
        "gui/sprites/widget/text_field_highlighted",
        l.search.x,
        l.search.y,
        l.search.w,
        l.search.h,
        u32::MAX,
    );
    let (tx, ty) = (l.search.x + 4.0, l.search.y + 6.0);
    let mut caret = tx;
    if st.query.is_empty() {
        batch.text(atlas, "Search Modrinth...", tx, ty, color(0x6C6C6C), true);
    } else {
        let tail = r::tail_fit(&st.query, l.search.w - 10.0, &measure);
        caret += batch.text(atlas, &tail, tx, ty, color(0xE0E0E0), true);
    }
    let blink = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(true, |d| (d.as_millis() / 500) % 2 == 0);
    if blink {
        batch.fill(atlas, caret, ty - 1.0, 1.0, 10.0, color(0xE0E0E0));
    }

    // Order, and only this game version or all.
    let sort_label = r::ellipsize(&format!("Sort: {}", st.sort.label()), l.sort.w - 8.0, &measure);
    ui::button(batch, atlas, l.sort.x, l.sort.y, l.sort.w, l.sort.h, &Text::literal(sort_label), true, l.sort.contains(cursor.0, cursor.1));
    let game_label = if st.only_game {
        format!("For {}", horizon_core::version::VANILLA_VERSION)
    } else {
        "All versions".to_owned()
    };
    let game_label = r::ellipsize(&game_label, l.game.w - 8.0, &measure);
    ui::button(batch, atlas, l.game.x, l.game.y, l.game.w, l.game.h, &Text::literal(game_label), true, l.game.contains(cursor.0, cursor.1));

    let status = r::status_text(st.loading, st.error.as_deref(), st.hits.len(), st.total);
    let status_color = if st.error.is_some() { tone_color(r::Tone::Bad) } else { color(0xA0A0A0) };
    let status = r::ellipsize(&status, l.list.w - 12.0, &measure);
    batch.text(atlas, &status, l.status[0], l.status[1], status_color, true);

    // The list.
    let first = batch.vertices.len();
    for row in first_row..last_row {
        let y = l.list.y + row as f32 * r::ROW - st.list_scroll;
        let hover = l.list.contains(cursor.0, cursor.1) && cursor.1 >= y && cursor.1 < y + r::ROW && cursor.0 < l.list.w - 6.0;
        draw_row(batch, atlas, &mut slots, &st, &st.hits[row], l.list.x, y, l.list.w - 6.0, st.selected == Some(row), hover);
    }
    batch.clip_since(first, l.list.x, l.list.y, l.list.x + l.list.w, l.list.y + l.list.h);
    ui::scrollbar(
        batch,
        atlas,
        l.list.x + l.list.w - 6.0,
        l.list.y,
        l.list.y + l.list.h,
        st.hits.len() as f32 * r::ROW,
        st.list_scroll.trunc(),
    );

    // The detail pane.
    if let Some(hit) = &selected_hit {
        let key = st.versions_key(&hit.id);
        let (list, failed, loading): (Vec<r::Version>, Option<String>, bool) = match st.versions.get(&key) {
            Some(VState::Loaded(list)) => (list.clone(), None, false),
            Some(VState::Failed(e)) => (Vec::new(), Some(e.clone()), false),
            _ => (Vec::new(), None, true),
        };
        let versions = if loading {
            r::Versions::Loading
        } else if let Some(e) = &failed {
            r::Versions::Failed(e)
        } else {
            r::Versions::Loaded(&list)
        };
        let picked = r::pick_version(&list, st.chosen.get(&hit.id).map(String::as_str));
        let width = r::detail(&l, 0.0).width;
        let lines = r::body_lines(hit, versions, picked, st.game_filter(), width, &measure);
        let notice = notice_lines(&st, width, &measure);
        let push = notice_height(&notice);
        let content = r::detail_height(lines.len()) + push;
        st.detail_scroll = st.detail_scroll.clamp(0.0, (content - l.detail.h).max(0.0));
        let d = r::detail(&l, st.detail_scroll);
        let first = batch.vertices.len();
        draw_icon(batch, atlas, &mut slots, &st, hit, d.icon.x, d.icon.y);
        let name = r::ellipsize(&hit.title, l.detail.x + l.detail.w - d.name[0] - 10.0, &measure);
        batch.text(atlas, &name, d.name[0], d.name[1], u32::MAX, true);
        let info = [
            r::ellipsize(&format!("By {}", hit.author), l.detail.x + l.detail.w - d.name[0] - 10.0, &measure),
            r::ellipsize(
                &format!("{} downloads, {} followers", r::count(hit.downloads), r::count(hit.follows)),
                l.detail.x + l.detail.w - d.name[0] - 10.0,
                &measure,
            ),
        ];
        for (line, at) in info.iter().zip(d.info) {
            batch.text(atlas, line, at[0], at[1], color(0xA0A0A0), true);
        }

        // The Install button and what the install is doing.
        let record = st.records.iter().find(|rec| rec.project == hit.id);
        let chosen_version = picked.and_then(|i| list.get(i));
        let installing = st.install.as_ref().filter(|i| i.project == hit.id);
        let busy = st.install.is_some();
        let same = matches!((record, chosen_version), (Some(rec), Some(v)) if rec.version == v.id);
        let can = chosen_version.is_some_and(|v| v.file.is_some()) && !busy && !same;
        let label = match (installing, same, record) {
            (Some(i), _, _) => match r::percent(i.done, i.total) {
                Some(p) => format!("{p}%"),
                None => "Downloading".to_owned(),
            },
            (None, true, _) => "Installed".to_owned(),
            (None, false, Some(_)) => "Update".to_owned(),
            (None, false, None) => "Install".to_owned(),
        };
        let inside = l.detail.contains(cursor.0, cursor.1);
        let b = d.install;
        ui::button(batch, atlas, b.x, b.y, b.w, b.h, &Text::literal(label), can, inside && can && b.contains(cursor.0, cursor.1));
        if let Some(file) = chosen_version.and_then(|v| v.file.as_ref()) {
            let size = format!("{} - {}", file.filename, r::human_size(file.size));
            let size = r::ellipsize(&size, l.detail.x + l.detail.w - d.status[0] - 10.0, &measure);
            batch.text(atlas, &size, d.status[0], d.status[1] - 5.0, color(0x909090), true);
        }
        if let Some((_, tone)) = &st.notice {
            for (i, line) in notice.iter().enumerate() {
                batch.text(atlas, line, d.body[0], d.body[1] + r::LINE * i as f32, tone_color(*tone), true);
            }
        }
        let body_y = d.body[1] + push;
        for (i, line) in lines.iter().enumerate() {
            let hover = line.version.is_some() && inside && {
                let ly = body_y + r::LINE * i as f32;
                cursor.1 >= ly && cursor.1 < ly + r::LINE
            };
            let tone = if hover { color(0xFFFFA0) } else { tone_color(line.tone) };
            batch.text(atlas, &line.text, d.body[0], body_y + r::LINE * i as f32, tone, true);
        }
        batch.clip_since(first, l.detail.x, l.detail.y, l.detail.x + l.detail.w, l.detail.y + l.detail.h);
        ui::scrollbar(
            batch,
            atlas,
            l.detail.x + l.detail.w - 6.0,
            l.detail.y,
            l.detail.y + l.detail.h,
            content,
            st.detail_scroll.trunc(),
        );
    } else {
        let note = if st.loading { "Searching..." } else { "Pick a pack to see its versions" };
        let nx = l.detail.x + ((l.detail.w - HudBatch::text_width(atlas, note)) / 2.0).floor();
        batch.text(atlas, note, nx, l.detail.y + 20.0, color(0x8C8C8C), true);
    }

    ui::separators(batch, atlas, l.list.y, l.list.y + l.list.h, in_world);

    // The footer.
    let done_hover = l.done.contains(cursor.0, cursor.1);
    ui::button(batch, atlas, l.done.x, l.done.y, l.done.w, l.done.h, &Text::translate("gui.done", &[]), true, done_hover);
    let hint = r::ellipsize(
        "Resource packs from Modrinth. Inspired by Resourcify by DeDiamondPro",
        w - 12.0,
        &measure,
    );
    let hx = ((w - HudBatch::text_width(atlas, &hint)) / 2.0).floor();
    batch.text(atlas, &hint, hx, l.hint_y, color(0x808080), true);

    let patches = std::mem::take(&mut slots.patches);
    drop(st);
    game.map_patches.extend(patches);
}

/// The "+" button on the Resource Packs screen.
pub fn draw_plus(game: &mut Game<'_>, batch: &mut HudBatch) {
    if !game.ui.options.resource_packs_open() {
        return;
    }
    let atlas = &game.gui.atlas;
    let b = r::plus_button(batch.width);
    let hover = b.contains(game.cursor.0, game.cursor.1);
    ui::button(batch, atlas, b.x, b.y, b.w, b.h, &Text::literal("+"), true, hover);
    if hover {
        ui::tooltip(batch, atlas, &[Text::literal("Find resource packs on Modrinth")], game.cursor);
    }
}

/// A press on the "+" button; true when it opened the browser.
pub fn press_plus(game: &mut Game<'_>) -> bool {
    if !game.ui.options.resource_packs_open() {
        return false;
    }
    let (w, _) = game.gui_size();
    let b = r::plus_button(w);
    if !b.contains(game.cursor.0, game.cursor.1) {
        return false;
    }
    r::open();
    true
}

// ---------------------------------------------------------------------------------------------
// Input.

/// Leaves the browser for the Resource Packs screen behind it.
fn done(game: &mut Game<'_>) -> Result<()> {
    r::close();
    game.handle_option_events(vec![Event::ButtonClick])
}

/// A press on the screen (a release or a right click does nothing).
pub fn mouse(game: &mut Game<'_>, pressed: bool, right: bool) -> Result<()> {
    if !pressed || right {
        game.dragging = false;
        return Ok(());
    }
    let (w, h) = game.gui_size();
    let l = r::layout(w, h);
    let (x, y) = game.cursor;
    let mut click = false;
    let mut leave = false;
    {
        let mut st = state();
        if l.done.contains(x, y) {
            leave = true;
        } else if l.sort.contains(x, y) {
            st.sort = st.sort.next();
            st.last_change = None;
            click = true;
        } else if l.game.contains(x, y) {
            st.only_game = !st.only_game;
            st.last_change = None;
            click = true;
        } else if let Some(row) = r::row_at(&l, st.list_scroll, st.hits.len(), x, y) {
            st.selected = Some(row);
            st.detail_scroll = 0.0;
            st.notice = None;
        } else if l.detail.contains(x, y)
            && let Some(hit) = st.selected.and_then(|i| st.hits.get(i)).cloned()
        {
            let key = st.versions_key(&hit.id);
            let list: Vec<r::Version> = match st.versions.get(&key) {
                Some(VState::Loaded(list)) => list.clone(),
                _ => Vec::new(),
            };
            let picked = r::pick_version(&list, st.chosen.get(&hit.id).map(String::as_str));
            let d = r::detail(&l, st.detail_scroll);
            if d.install.contains(x, y) {
                let same = matches!(
                    (st.records.iter().find(|rec| rec.project == hit.id), picked.and_then(|i| list.get(i))),
                    (Some(rec), Some(v)) if rec.version == v.id
                );
                if let Some(version) = picked.and_then(|i| list.get(i))
                    && st.install.is_none()
                    && !same
                {
                    start_install(&mut st, &hit, version);
                    click = true;
                }
            } else {
                let measure = |s: &str| HudBatch::text_width(&game.gui.atlas, s);
                let lines = r::body_lines(&hit, r::Versions::Loaded(&list), picked, st.game_filter(), d.width, &measure);
                let push = notice_height(&notice_lines(&st, d.width, &measure));
                let body_y = d.body[1] + push;
                if let Some(i) = r::version_at(&lines, body_y, y)
                    && let Some(v) = list.get(i)
                {
                    st.chosen.insert(hit.id.clone(), v.id.clone());
                    click = true;
                }
            }
        }
    }
    if leave {
        return done(game);
    }
    if click {
        game.handle_option_events(vec![Event::ButtonClick])?;
    }
    Ok(())
}

/// The wheel: the detail pane when the pointer is over it, the list otherwise.
pub fn scroll(game: &mut Game<'_>, amount: f32) -> Result<()> {
    let (w, h) = game.gui_size();
    let l = r::layout(w, h);
    let (x, y) = game.cursor;
    let mut st = state();
    if l.detail.contains(x, y) {
        // The most it can scroll is known when it is drawn.
        st.detail_scroll = (st.detail_scroll - amount * r::SCROLL_STEP).max(0.0);
    } else {
        let count = st.hits.len();
        st.list_scroll = (st.list_scroll - amount * r::SCROLL_STEP).clamp(0.0, r::max_scroll(count, l.list.h));
    }
    Ok(())
}

/// Keys: typing searches, the arrows pick a pack, Escape leaves.
pub fn key(game: &mut Game<'_>, key: KeyCode, text: Option<&str>) -> Result<()> {
    let ctrl = game.has_control_down();
    if key == KeyCode::Escape {
        return done(game);
    }
    let (w, h) = game.gui_size();
    let l = r::layout(w, h);
    let mut st = state();
    let count = st.hits.len();
    let page = ((l.list.h / r::ROW).floor() as i32 - 1).max(1);
    let moved = match key {
        KeyCode::ArrowDown => r::step(count, st.selected, 1),
        KeyCode::ArrowUp => r::step(count, st.selected, -1),
        KeyCode::PageDown => r::step(count, st.selected, page),
        KeyCode::PageUp => r::step(count, st.selected, -page),
        KeyCode::Home => r::step(count, None, 1),
        KeyCode::End => r::step(count, None, -1),
        _ => None,
    };
    if let Some(row) = moved {
        st.selected = Some(row);
        st.detail_scroll = 0.0;
        st.notice = None;
        st.list_scroll = r::scroll_into_view(st.list_scroll, row, l.list.h);
    } else if key == KeyCode::Backspace {
        r::backspace(&mut st.query, ctrl);
        st.last_change = Some(Instant::now());
    } else if !ctrl && let Some(typed) = text {
        r::type_text(&mut st.query, typed);
        st.last_change = Some(Instant::now());
    }
    Ok(())
}

/// Ctrl+A, C, V and X in the search box.
pub fn shortcut(game: &mut Game<'_>, shortcut: EditShortcut) -> bool {
    use horizon_client::clipboard::Clipboard;
    let mut st = state();
    match shortcut {
        EditShortcut::Paste => {
            let pasted = game.clipboard.get().replace(['\r', '\n'], " ");
            r::type_text(&mut st.query, &pasted);
            st.last_change = Some(Instant::now());
        }
        EditShortcut::Copy => {
            let text = st.query.clone();
            game.clipboard.set(&text);
        }
        EditShortcut::Cut => {
            let text = std::mem::take(&mut st.query);
            game.clipboard.set(&text);
            st.last_change = Some(Instant::now());
        }
        EditShortcut::SelectAll => {}
    }
    true
}
