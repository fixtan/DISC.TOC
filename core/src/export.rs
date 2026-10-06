//! Disc metadata + export formats (pure, testable).
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct TrackMeta {
    pub title: String,
    #[serde(default)]
    pub artist: String,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct DiscMeta {
    pub mb_id: String,
    #[serde(default)]
    pub cddb_id: String,
    #[serde(default)]
    pub album: String,
    #[serde(default)]
    pub artist: String,
    #[serde(default)]
    pub date: String,
    pub tracks: Vec<TrackMeta>,
    /// track start LBAs (without 150), used for CUE. may be empty.
    #[serde(default)]
    pub starts: Vec<u32>,
}

fn csv_cell(s: &str) -> String {
    if s.contains([',', '"', '\n']) {
        format!("\"{}\"", s.replace('"', "\"\""))
    } else {
        s.to_string()
    }
}

fn cue_str(s: &str) -> String {
    s.replace('"', "'").replace(['\r', '\n'], " ")
}

fn msf(lba: u32) -> String {
    format!("{:02}:{:02}:{:02}", lba / 75 / 60, lba / 75 % 60, lba % 75)
}

impl DiscMeta {
    pub fn track_artist<'a>(&'a self, i: usize) -> &'a str {
        let a = self.tracks[i].artist.as_str();
        if a.is_empty() { &self.artist } else { a }
    }

    pub fn to_json(&self) -> String {
        serde_json::to_string_pretty(self).unwrap_or_default()
    }

    pub fn to_txt(&self) -> String {
        let mut s = format!("{} - {}\n", self.artist, self.album);
        for i in 0..self.tracks.len() {
            s += &format!("{:02}. {} - {}\n", i + 1, self.track_artist(i), self.tracks[i].title);
        }
        s
    }

    pub fn to_csv(&self) -> String {
        let mut s = String::from("no,title,artist,album\n");
        for i in 0..self.tracks.len() {
            s += &format!(
                "{},{},{},{}\n",
                i + 1,
                csv_cell(&self.tracks[i].title),
                csv_cell(self.track_artist(i)),
                csv_cell(&self.album)
            );
        }
        s
    }

    pub fn to_cue(&self) -> String {
        let mut s = format!(
            "REM DISCID {}\nREM DATE {}\nPERFORMER \"{}\"\nTITLE \"{}\"\nFILE \"disc.wav\" WAVE\n",
            self.cddb_id,
            cue_str(&self.date),
            cue_str(&self.artist),
            cue_str(&self.album)
        );
        let base = self.starts.first().copied().unwrap_or(0);
        for i in 0..self.tracks.len() {
            s += &format!(
                "  TRACK {:02} AUDIO\n    TITLE \"{}\"\n    PERFORMER \"{}\"\n",
                i + 1,
                cue_str(&self.tracks[i].title),
                cue_str(self.track_artist(i))
            );
            let pos = self.starts.get(i).map(|v| v - base).unwrap_or(0);
            s += &format!("    INDEX 01 {}\n", msf(pos));
        }
        s
    }

    pub fn export(&self, fmt: &str) -> Result<String, String> {
        match fmt {
            "json" => Ok(self.to_json()),
            "txt" => Ok(self.to_txt()),
            "csv" => Ok(self.to_csv()),
            "cue" => Ok(self.to_cue()),
            _ => Err(format!("unknown format {fmt}")),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn m() -> DiscMeta {
        DiscMeta {
            mb_id: "X".into(),
            cddb_id: "5c0f8319".into(),
            album: "Alb".into(),
            artist: "Art".into(),
            date: "2003".into(),
            tracks: vec![
                TrackMeta { title: "A,\"q\"".into(), artist: "".into() },
                TrackMeta { title: "B".into(), artist: "Z".into() },
            ],
            starts: vec![0, 2100],
        }
    }
    #[test]
    fn csv_escape() {
        assert_eq!(m().to_csv().lines().nth(1).unwrap(), "1,\"A,\"\"q\"\"\",Art,Alb");
    }
    #[test]
    fn txt() {
        assert!(m().to_txt().contains("02. Z - B"));
    }
    #[test]
    fn cue() {
        let c = m().to_cue();
        assert!(c.contains("INDEX 01 00:00:00"));
        assert!(c.contains("INDEX 01 00:28:00")); // 2100 sectors = 28s
        assert!(c.contains("TITLE \"A,'q'\""));
    }
    #[test]
    fn json_roundtrip() {
        let j = m().to_json();
        assert_eq!(serde_json::from_str::<DiscMeta>(&j).unwrap(), m());
        // old/minimal files still load
        let min: DiscMeta = serde_json::from_str(r#"{"mb_id":"Q","tracks":[{"title":"t"}]}"#).unwrap();
        assert_eq!(min.tracks.len(), 1);
    }
}
