//! Linux check: list drives, print TOC + disc IDs, rip one track to a WAV.
//!   cargo run --example linux_rip              -> TOC only
//!   cargo run --example linux_rip -- 1 out.wav -> also rip track 1 to out.wav
#[cfg(target_os = "linux")]
fn main() -> Result<(), String> {
    use disctoc_core::{cd, wav_header};
    use std::io::Write;

    let drives = cd::list_drives();
    println!("drives: {drives:?}");
    let path = drives.first().ok_or("no /dev/sr* found")?;
    let d = cd::Drive::open(path)?;
    let toc = d.read_toc()?;
    println!("tracks: {}  leadout LBA: {}", toc.track_count(), toc.leadout);
    println!("MusicBrainz ID: {}", toc.musicbrainz_id());
    println!("CDDB ID:        {}", toc.cddb_id());
    for i in 0..toc.track_count() {
        let (start, sectors) = toc.track_range(i).unwrap();
        println!("  {:2}: LBA {:6}  {:6} sectors  {:3}s", toc.first as usize + i, start, sectors, sectors / 75);
    }

    let args: Vec<String> = std::env::args().collect();
    if args.len() >= 3 {
        let no: usize = args[1].parse().map_err(|_| "track number")?;
        let idx = no.checked_sub(toc.first as usize).ok_or("bad track")?;
        let (start, sectors) = toc.track_range(idx).ok_or("no such track")?;
        println!("ripping track {no} ({sectors} sectors) -> {}", args[2]);
        let mut f = std::fs::File::create(&args[2]).map_err(|e| e.to_string())?;
        f.write_all(&wav_header(sectors * 2352)).map_err(|e| e.to_string())?;
        d.read_audio(start, sectors, |b| f.write_all(b).map_err(|e| e.to_string()))?;
        println!("done");
    }
    Ok(())
}

#[cfg(not(target_os = "linux"))]
fn main() {
    println!("Linux only");
}
