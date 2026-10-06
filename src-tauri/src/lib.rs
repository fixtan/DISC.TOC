use disctoc_core::{export::DiscMeta, mb, Toc};
use tauri::Manager;
use tauri_plugin_dialog::DialogExt;
use serde::Serialize;
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

pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_dialog::init())
        .invoke_handler(tauri::generate_handler![list_drives, read_toc, lookup, read_pcm, save_meta, export_text, export_file])
        .run(tauri::generate_context!())
        .expect("error while running DISC.TOC");
}
