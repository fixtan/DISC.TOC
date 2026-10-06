use disctoc_core::{export::DiscMeta, mb, wav_header, Toc};
use tauri::{Emitter, Manager};
use tauri_plugin_dialog::DialogExt;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::time::Duration;

// ASCII only. Put a real contact (URL/mail) here before release; MusicBrainz asks for one.
const UA: &str = "DISC.TOC/0.1 ( https://lain-lab.com )";

#[derive(Serialize)]
struct TrackOut { no: u32, start: u32, sectors: u32, secs: u32 }

#[derive(Serialize)]
struct TocOut { tracks: Vec<TrackOut>, mb_id: String, cddb_id: String, mb_toc: String }

#[cfg(windows)]
fn open_toc(drive: &str) -> Result<(disctoc_core::cd_win::Drive, Toc), String> {
    let d = disctoc_core::cd_win::Drive::open(drive)?;
    let t = d.read_toc()?;
    Ok((d, t))
}

#[tauri::command]
fn win_raise_all(app: tauri::AppHandle, label: String) {
    for l in LABELS.iter() {
        if *l == label { continue; }
        if let Some(w) = app.get_webview_window(l) {
            if w.is_visible().unwrap_or(false) && !w.is_minimized().unwrap_or(false) {
                let _ = w.set_always_on_top(true);
                let _ = w.set_always_on_top(false);
            }
        }
    }
    if let Some(w) = app.get_webview_window(&label) {
        let _ = w.set_focus();
    }
}


#[tauri::command]
fn list_drives() -> Vec<String> {
    #[cfg(windows)]
    { disctoc_core::cd_win::list_drives() }
    #[cfg(not(windows))]
    { Vec::new() } // TODO: Linux (/dev/sr*, CDROMREADTOCHDR/ENTRY, CDROMREADAUDIO)
}

#[tauri::command]
fn read_toc(drive: String) -> Result<TocOut, String> {
    #[cfg(windows)]
    {
        let (_d, t) = open_toc(&drive)?;
        let tracks = (0..t.track_count()).map(|i| {
            let (start, sectors) = t.track_range(i).unwrap();
            TrackOut { no: t.first as u32 + i as u32, start, sectors, secs: sectors / 75 }
        }).collect();
        Ok(TocOut { tracks, mb_id: t.musicbrainz_id(), cddb_id: t.cddb_id(), mb_toc: t.mb_toc_param() })
    }
    #[cfg(not(windows))]
    { let _ = drive; Err("not supported on this OS yet".into()) }
}

async fn mb_get(client: &reqwest::Client, url: &str) -> Result<Option<Value>, String> {
    let r = client.get(url).send().await.map_err(|e| e.to_string())?;
    if r.status().as_u16() == 404 { return Ok(None); }
    if !r.status().is_success() { return Err(format!("MusicBrainz HTTP {}", r.status())); }
    Ok(Some(r.json().await.map_err(|e| e.to_string())?))
}

#[derive(Serialize)]
struct LookupOut { source: String, candidates: Vec<mb::Candidate> }

#[tauri::command]
async fn lookup(app: tauri::AppHandle, drive: String) -> Result<LookupOut, String> {
    #[cfg(not(windows))]
    { let _ = (app, drive); return Err("not supported on this OS yet".into()); }
    #[cfg(windows)]
    {
        let t = { let (_d, t) = open_toc(&drive)?; t };
        let id = t.musicbrainz_id();
        let n = t.track_count();
        // user-saved data wins over the network
        if let Some(m) = load_local(&app, &id) {
            if m.tracks.len() == n {
                return Ok(LookupOut { source: "local".into(), candidates: vec![to_candidate(&m)] });
            }
        }
        let client = reqwest::Client::builder().user_agent(UA).timeout(Duration::from_secs(20))
            .build().map_err(|e| e.to_string())?;
        let inc = "inc=recordings+artist-credits&fmt=json";
        if let Some(j) = mb_get(&client, &format!("https://musicbrainz.org/ws/2/discid/{id}?{inc}")).await? {
            let c = mb::candidates(&j, &id, n);
            if !c.is_empty() { return Ok(LookupOut { source: "discid".into(), candidates: c }); }
        }
        tokio_sleep().await; // MusicBrainz: max 1 request/sec
        let url = format!("https://musicbrainz.org/ws/2/discid/-?toc={}&{inc}", t.mb_toc_param());
        let c = match mb_get(&client, &url).await? { Some(j) => mb::candidates(&j, &id, n), None => vec![] };
        Ok(LookupOut { source: "toc".into(), candidates: c })
    }
}

fn meta_path(app: &tauri::AppHandle, id: &str) -> Result<std::path::PathBuf, String> {
    if id.is_empty() || !id.chars().all(|c| c.is_ascii_alphanumeric() || "._-".contains(c)) {
        return Err("invalid disc id".into());
    }
    let dir = app.path().app_data_dir().map_err(|e| e.to_string())?.join("discs");
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    Ok(dir.join(format!("{id}.json")))
}

#[cfg(windows)]
fn load_local(app: &tauri::AppHandle, id: &str) -> Option<DiscMeta> {
    let p = meta_path(app, id).ok()?;
    serde_json::from_str(&std::fs::read_to_string(p).ok()?).ok()
}

#[cfg(windows)]
fn to_candidate(m: &DiscMeta) -> mb::Candidate {
    mb::Candidate {
        release_id: "local".into(),
        title: m.album.clone(),
        artist: m.artist.clone(),
        date: m.date.clone(),
        tracks: m.tracks.iter().map(|t| mb::TrackInfo {
            title: t.title.clone(),
            artist: if t.artist.is_empty() { m.artist.clone() } else { t.artist.clone() },
        }).collect(),
    }
}

/// Save user-edited metadata for a disc (keyed by MusicBrainz disc ID).
#[tauri::command]
fn save_meta(app: tauri::AppHandle, meta: DiscMeta) -> Result<(), String> {
    let p = meta_path(&app, &meta.mb_id)?;
    std::fs::write(p, meta.to_json()).map_err(|e| e.to_string())
}

/// fmt: json | txt | csv | cue
#[tauri::command]
fn export_text(meta: DiscMeta, fmt: String) -> Result<String, String> {
    meta.export(&fmt)
}

/// Opens a save dialog and writes the export there. Returns the saved path, or None if cancelled.
#[tauri::command]
async fn export_file(app: tauri::AppHandle, meta: DiscMeta, fmt: String, name: String) -> Result<Option<String>, String> {
    let text = meta.export(&fmt)?;
    let bytes: Vec<u8> = if fmt == "csv" {
        let mut v = vec![0xEF, 0xBB, 0xBF]; // BOM for Excel
        v.extend_from_slice(text.as_bytes());
        v
    } else { text.into_bytes() };
    let default = format!("{name}.{fmt}");
    let picked = tauri::async_runtime::spawn_blocking(move || {
        app.dialog().file().add_filter(fmt.to_uppercase(), &[fmt.as_str()]).set_file_name(default).blocking_save_file()
    }).await.map_err(|e| e.to_string())?;
    let Some(fp) = picked else { return Ok(None) };
    let path = fp.into_path().map_err(|e| e.to_string())?;
    std::fs::write(&path, bytes).map_err(|e| e.to_string())?;
    Ok(Some(path.to_string_lossy().into_owned()))
}

#[cfg(windows)]
async fn tokio_sleep() {
    // tauri re-exports its async runtime; spawn_blocking keeps this dependency-free
    let _ = tauri::async_runtime::spawn_blocking(|| std::thread::sleep(Duration::from_millis(1100))).await;
}

#[cfg(windows)]
static DRIVE: std::sync::Mutex<Option<(String, disctoc_core::cd_win::Drive)>> = std::sync::Mutex::new(None);

/// Raw CDDA PCM (16bit LE stereo 44.1k) for `count` sectors from `lba`. The frontend streams a track by calling this ~1s at a time.
#[tauri::command]
async fn read_pcm(drive: String, lba: u32, count: u32) -> Result<tauri::ipc::Response, String> {
    #[cfg(not(windows))]
    { let _ = (drive, lba, count); return Err("not supported on this OS yet".into()); }
    #[cfg(windows)]
    tauri::async_runtime::spawn_blocking(move || {
        let mut g = DRIVE.lock().unwrap_or_else(|e| e.into_inner());
        if g.as_ref().map(|(d, _)| d != &drive).unwrap_or(true) {
            *g = Some((drive.clone(), disctoc_core::cd_win::Drive::open(&drive)?));
        }
        match g.as_ref().unwrap().1.read_pcm(lba, count.min(150)) {
            Ok(v) => Ok(tauri::ipc::Response::new(v)),
            Err(e) => { *g = None; Err(e) } // reopen next time (disc swap / drive hiccup)
        }
    }).await.map_err(|e| e.to_string())?
}

#[derive(Deserialize)]
struct RipTrack { start: u32, sectors: u32, name: String }

#[derive(Serialize, Clone)]
struct RipProgress { index: usize, total: usize, name: String, done: u32, sectors: u32 }

#[derive(Serialize)]
struct RipResult { dir: Option<String>, written: usize, bad_sectors: u32, cancelled: bool }

static RIP_CANCEL: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

#[tauri::command]
fn rip_cancel() { RIP_CANCEL.store(true, std::sync::atomic::Ordering::Relaxed); }

fn safe_name(s: &str) -> String {
    let t: String = s.chars().map(|c| if "\\/:*?\"<>|".contains(c) || c.is_control() { '_' } else { c }).collect();
    let t = t.trim().trim_end_matches('.').trim().to_string();
    if t.is_empty() { "track".into() } else { t.chars().take(120).collect() }
}

/// Rip tracks to WAV files in a folder the user picks. Emits "rip-progress". Unreadable sectors become silence (counted in bad_sectors).
#[tauri::command]
async fn rip_wav(app: tauri::AppHandle, drive: String, tracks: Vec<RipTrack>) -> Result<RipResult, String> {
    #[cfg(not(windows))]
    { let _ = (app, drive, tracks); return Err("not supported on this OS yet".into()); }
    #[cfg(windows)]
    {
        let app2 = app.clone();
        let picked = tauri::async_runtime::spawn_blocking(move || app2.dialog().file().blocking_pick_folder())
            .await.map_err(|e| e.to_string())?;
        let Some(fp) = picked else { return Ok(RipResult { dir: None, written: 0, bad_sectors: 0, cancelled: true }) };
        let dir = fp.into_path().map_err(|e| e.to_string())?;
        RIP_CANCEL.store(false, std::sync::atomic::Ordering::Relaxed);
        let dir2 = dir.clone();
        tauri::async_runtime::spawn_blocking(move || -> Result<RipResult, String> {
            use std::io::Write;
            use std::sync::atomic::Ordering::Relaxed;
            const SB: usize = 2352;
            let d = disctoc_core::cd_win::Drive::open(&drive)?; // own handle: independent of the playback handle
            let total = tracks.len();
            let (mut written, mut bad) = (0usize, 0u32);
            for (i, t) in tracks.iter().enumerate() {
                let fin = dir2.join(format!("{}.wav", safe_name(&t.name)));
                let part = dir2.join(format!("{}.wav.part", safe_name(&t.name)));
                let mut f = std::io::BufWriter::new(std::fs::File::create(&part).map_err(|e| format!("{}: {e}", part.display()))?);
                f.write_all(&wav_header(t.sectors * SB as u32)).map_err(|e| e.to_string())?;
                let mut lba = t.start; let end = t.start + t.sectors;
                while lba < end {
                    if RIP_CANCEL.load(Relaxed) { drop(f); let _ = std::fs::remove_file(&part); return Ok(RipResult { dir: Some(dir2.to_string_lossy().into_owned()), written, bad_sectors: bad, cancelled: true }); }
                    let n = 75u32.min(end - lba);
                    match d.read_pcm(lba, n) {
                        Ok(v) => f.write_all(&v).map_err(|e| e.to_string())?,
                        Err(_) => for k in 0..n { // retry one sector at a time; silence for the unreadable ones
                            match d.read_pcm(lba + k, 1) {
                                Ok(v) => f.write_all(&v).map_err(|e| e.to_string())?,
                                Err(_) => { bad += 1; f.write_all(&[0u8; SB]).map_err(|e| e.to_string())?; }
                            }
                        },
                    }
                    lba += n;
                    let _ = app.emit("rip-progress", RipProgress { index: i, total, name: t.name.clone(), done: lba - t.start, sectors: t.sectors });
                }
                f.flush().map_err(|e| e.to_string())?; drop(f);
                let _ = std::fs::remove_file(&fin);
                std::fs::rename(&part, &fin).map_err(|e| e.to_string())?;
                written += 1;
            }
            Ok(RipResult { dir: Some(dir2.to_string_lossy().into_owned()), written, bad_sectors: bad, cancelled: false })
        }).await.map_err(|e| e.to_string())?
    }
}

// ───────── ウィンドウ管理（MAIN / EQ / PLAYLIST の3枚） ─────────
const LABELS: [&str; 3] = ["main", "eq", "playlist"];

#[derive(Serialize, Deserialize, Clone)]
struct WinRect { label: String, x: i32, y: i32, w: u32, h: u32, visible: bool }

#[derive(Deserialize)]
struct WinMove { label: String, x: i32, y: i32 }

/// win_ready で表示するかどうか（レイアウト適用時に決める）
static WANT_VISIBLE: std::sync::Mutex<Vec<(String, bool)>> = std::sync::Mutex::new(Vec::new());
/// メインの最小化に合わせて隠した窓
static HIDDEN_BY_MINIMIZE: std::sync::Mutex<Vec<String>> = std::sync::Mutex::new(Vec::new());
static LAYOUT_TARGETS: std::sync::Mutex<Vec<WinRect>> = std::sync::Mutex::new(Vec::new());
static WAS_MINIMIZED: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

/// OS が返す窓の外枠（outer）には、見えない余白（リサイズ用の枠など）が含まれることがある。
/// そのまま積むと窓どうしに隙間ができるので、実際に見えている枠（DWM の extended frame bounds）との差を測る。
/// 戻り値は (左, 上, 右, 下)。測れないときは (0, 0, 0, 0)。
static INSET_CACHE: std::sync::Mutex<Vec<(String, (i32, i32, i32, i32))>> = std::sync::Mutex::new(Vec::new());

#[cfg(windows)]
fn measure_insets(w: &tauri::WebviewWindow) -> Option<(i32, i32, i32, i32)> {
    use windows_sys::Win32::Foundation::RECT;
    use windows_sys::Win32::Graphics::Dwm::{DwmGetWindowAttribute, DWMWA_EXTENDED_FRAME_BOUNDS};
    let (h, p, s) = (w.hwnd().ok()?, w.outer_position().ok()?, w.outer_size().ok()?);
    let mut fb = RECT { left: 0, top: 0, right: 0, bottom: 0 };
    let hr = unsafe {
        DwmGetWindowAttribute(
            h.0 as isize as *mut core::ffi::c_void,
            DWMWA_EXTENDED_FRAME_BOUNDS as _,
            &mut fb as *mut RECT as *mut core::ffi::c_void,
            std::mem::size_of::<RECT>() as u32,
        )
    };
    if hr != 0 { return None; }
    let v = (fb.left - p.x, fb.top - p.y, p.x + s.width as i32 - fb.right, p.y + s.height as i32 - fb.bottom);
    if [v.0, v.1, v.2, v.3].iter().any(|n| *n < 0 || *n > 40) { return None; }
    Some(v)
}
#[cfg(not(windows))]
fn measure_insets(_w: &tauri::WebviewWindow) -> Option<(i32, i32, i32, i32)> { None }

fn frame_insets(w: &tauri::WebviewWindow) -> (i32, i32, i32, i32) {
    let label = w.label().to_string();
    let mut cache = INSET_CACHE.lock().unwrap_or_else(|e| e.into_inner());
    if w.is_visible().unwrap_or(false) {
        if let Some(v) = measure_insets(w) {
            cache.retain(|(l, _)| *l != label);
            cache.push((label, v));
            return v;
        }
    }
    // 隠れている窓は測れない。前に測った値、無ければ他の窓の値を使う
    cache.iter().find(|(l, _)| *l == label).or_else(|| cache.first()).map(|(_, v)| *v).unwrap_or((0, 0, 0, 0))
}

/// 見えている枠で表した位置・大きさ
fn rect_of(app: &tauri::AppHandle, label: &str) -> Option<WinRect> {
    let w = app.get_webview_window(label)?;
    let p = w.outer_position().ok()?;
    let s = w.outer_size().ok()?;
    let (il, it, ir, ib) = frame_insets(&w);
    Some(WinRect {
        label: label.into(), x: p.x + il, y: p.y + it,
        w: (s.width as i32 - il - ir).max(1) as u32, h: (s.height as i32 - it - ib).max(1) as u32,
        visible: w.is_visible().unwrap_or(false),
    })
}

/// 「見えている枠」の座標で窓を置く（size を渡すと大きさも）
fn place_visible(app: &tauri::AppHandle, label: &str, x: i32, y: i32, size: Option<(u32, u32)>) {
    let Some(w) = app.get_webview_window(label) else { return };
    let (il, it, ir, ib) = frame_insets(&w);
    if let Some((vw, vh)) = size {
        let _ = w.set_size(tauri::Size::Physical(tauri::PhysicalSize::new((vw as i32 + il + ir).max(1) as u32, (vh as i32 + it + ib).max(1) as u32)));
    }
    let _ = w.set_position(tauri::Position::Physical(tauri::PhysicalPosition::new(x - il, y - it)));
}

fn all_rects(app: &tauri::AppHandle) -> Vec<WinRect> {
    LABELS.iter().filter_map(|l| rect_of(app, l)).collect()
}

fn layout_path(app: &tauri::AppHandle) -> Option<std::path::PathBuf> {
    app.path().app_data_dir().ok().map(|d| d.join("layout.json"))
}

fn place(app: &tauri::AppHandle, r: &WinRect) {
    let size = if r.label == "playlist" && r.w > 200 && r.h > 120 { Some((r.w, r.h)) } else { None };
    place_visible(app, &r.label, r.x, r.y, size);
}

/// 保存した位置が、いまつながっているどれかのモニターの中にあるか
fn on_some_monitor(app: &tauri::AppHandle, r: &WinRect) -> bool {
    let (px, py) = (r.x + 40, r.y + 10);
    app.available_monitors().map(|ms| ms.iter().any(|m| {
        let (mp, ms) = (m.position(), m.size());
        px >= mp.x && px < mp.x + ms.width as i32 && py >= mp.y && py < mp.y + ms.height as i32
    })).unwrap_or(false)
}

fn set_want(label: &str, v: bool) {
    let mut g = WANT_VISIBLE.lock().unwrap_or_else(|e| e.into_inner());
    g.retain(|(l, _)| l != label);
    g.push((label.to_string(), v));
}

/// 起動時に、保存したレイアウトを適用する。無ければ、画面の中央に縦に積む。
fn apply_layout(app: &tauri::AppHandle) {
    let saved: Option<Vec<WinRect>> = layout_path(app)
        .and_then(|p| std::fs::read_to_string(p).ok())
        .and_then(|t| serde_json::from_str(&t).ok())
        .filter(|v: &Vec<WinRect>| LABELS.iter().all(|l| v.iter().any(|r| r.label == *l)) && v.iter().all(|r| on_some_monitor(app, r)));
    if let Some(v) = saved {
        for r in &v { place(app, r); set_want(&r.label, r.visible || r.label == "main"); }
        *LAYOUT_TARGETS.lock().unwrap_or_else(|e| e.into_inner()) = v;
        return;
    }
    LAYOUT_TARGETS.lock().unwrap_or_else(|e| e.into_inner()).clear();
    let sizes: Vec<(String, u32, u32)> = LABELS.iter().filter_map(|l| rect_of(app, l).map(|r| (r.label, r.w, r.h))).collect();
    let total_h: u32 = sizes.iter().map(|s| s.2).sum();
    let (mx, my, mw, mh) = app.primary_monitor().ok().flatten()
        .map(|m| (m.position().x, m.position().y, m.size().width, m.size().height)).unwrap_or((0, 0, 1280, 800));
    let width = sizes.first().map(|s| s.1).unwrap_or(460);
    let x = mx + (mw as i32 - width as i32) / 2;
    let mut y = my + ((mh as i32 - total_h as i32) / 2 - 20).max(0);
    for (label, w, h) in sizes {
        place(app, &WinRect { label: label.clone(), x, y, w, h, visible: true });
        set_want(&label, true);
        y += h as i32;
    }
}

#[tauri::command]
fn win_rects(app: tauri::AppHandle) -> Vec<WinRect> { all_rects(&app) }

#[tauri::command]
fn win_move(app: tauri::AppHandle, moves: Vec<WinMove>) {
    for m in moves {
        place_visible(&app, &m.label, m.x, m.y, None);
    }
}

#[tauri::command]
fn layout_save(app: tauri::AppHandle) {
    if let (Some(p), Ok(t)) = (layout_path(&app), serde_json::to_string(&all_rects(&app))) {
        if let Some(d) = p.parent() { let _ = std::fs::create_dir_all(d); }
        let _ = std::fs::write(p, t);
    }
}


/// 窓が出てから、見えている枠を測り直して、位置をそろえ直す（起動直後に1回）。
/// 保存したレイアウトがあればその位置へ、無ければ画面の中央に隙間なく縦に積む。
#[tauri::command]
fn layout_settle(app: tauri::AppHandle) {
    let targets = LAYOUT_TARGETS.lock().unwrap_or_else(|e| e.into_inner()).clone();
    let mut dbg = String::new();
    if !targets.is_empty() {
        for r in &targets { place(&app, r); }
    } else {
        let rs = all_rects(&app);
        let total: i32 = rs.iter().map(|r| r.h as i32).sum();
        let (mx, my, mw, mh) = app.primary_monitor().ok().flatten()
            .map(|m| (m.position().x, m.position().y, m.size().width as i32, m.size().height as i32)).unwrap_or((0, 0, 1280, 800));
        let width = rs.first().map(|r| r.w as i32).unwrap_or(460);
        let x = mx + (mw - width) / 2;
        let mut y = my + ((mh - total) / 2 - 20).max(0);
        for r in &rs { place_visible(&app, &r.label, x, y, None); y += r.h as i32; }
    }
    for l in LABELS {
        if let Some(w) = app.get_webview_window(l) {
            if let (Ok(p), Ok(s)) = (w.outer_position(), w.outer_size()) {
                dbg += &format!("{l}: outer=({},{} {}x{}) insets={:?} visible={}\n", p.x, p.y, s.width, s.height, frame_insets(&w), w.is_visible().unwrap_or(false));
            }
        }
    }
    if let Some(p) = layout_path(&app).and_then(|p| p.parent().map(|d| d.join("layout-debug.txt"))) { let _ = std::fs::write(p, dbg); }
}

/// ページの準備ができたら、その窓を出す（読み込み中の白いちらつきを避ける）
#[tauri::command]
fn win_ready(app: tauri::AppHandle, label: String) {
    let want = WANT_VISIBLE.lock().unwrap_or_else(|e| e.into_inner())
        .iter().find(|(l, _)| *l == label).map(|(_, v)| *v).unwrap_or(true);
    if want || label == "main" {
        if let Some(w) = app.get_webview_window(&label) { let _ = w.show(); }
    }
}

#[tauri::command]
fn win_set_visible(app: tauri::AppHandle, label: String, visible: bool) {
    if label == "main" || !LABELS.contains(&label.as_str()) { return; }
    if let Some(w) = app.get_webview_window(&label) {
        if visible { let _ = w.show(); } else { let _ = w.hide(); }
    }
    set_want(&label, visible);
    let _ = app.emit("dt:vis", serde_json::json!({ "label": label, "visible": visible }));
    layout_save(app);
}

#[tauri::command]
fn win_minimize(app: tauri::AppHandle) {
    if let Some(w) = app.get_webview_window("main") { let _ = w.minimize(); }
}

#[tauri::command]
fn win_quit(app: tauri::AppHandle) { layout_save(app.clone()); app.exit(0); }

/// メインの最小化・復元に合わせて、EQ / PLAYLIST を隠す・戻す（この2枚はタスクバーに出ないので）
fn sync_minimize(window: &tauri::Window) {
    use std::sync::atomic::Ordering::SeqCst;
    let app = window.app_handle();
    let min = window.is_minimized().unwrap_or(false);
    if min == WAS_MINIMIZED.swap(min, SeqCst) { return; }
    let mut hidden = HIDDEN_BY_MINIMIZE.lock().unwrap_or_else(|e| e.into_inner());
    if min {
        hidden.clear();
        for l in ["eq", "playlist"] {
            if let Some(w) = app.get_webview_window(l) {
                if w.is_visible().unwrap_or(false) { hidden.push(l.to_string()); let _ = w.hide(); }
            }
        }
    } else {
        for l in hidden.drain(..) {
            if let Some(w) = app.get_webview_window(&l) { let _ = w.show(); }
        }
    }
}

pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_dialog::init())
        .setup(|app| { apply_layout(app.handle()); Ok(()) })
        .on_window_event(|window, event| {
            if window.label() != "main" { return; }
            match event {
                tauri::WindowEvent::CloseRequested { .. } => { layout_save(window.app_handle().clone()); window.app_handle().exit(0); }
                tauri::WindowEvent::Resized(_) => sync_minimize(window),
                _ => {}
            }
        })
        .invoke_handler(tauri::generate_handler![
            list_drives, read_toc, lookup, read_pcm, save_meta, export_text, export_file, rip_wav, rip_cancel,
            win_rects, win_move, layout_save, layout_settle, win_ready, win_set_visible, win_minimize, win_quit,win_raise_all
        ])
        .run(tauri::generate_context!())
        .expect("error while running DISC.TOC");
}
