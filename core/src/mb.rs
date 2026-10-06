//! MusicBrainz JSON -> flat candidate list (pure, testable).
use serde::Serialize;
use serde_json::Value;

#[derive(Debug, Serialize, Clone, PartialEq)]
pub struct TrackInfo {
    pub title: String,
    pub artist: String,
}

#[derive(Debug, Serialize, Clone, PartialEq)]
pub struct Candidate {
    pub release_id: String,
    pub title: String,
    pub artist: String,
    pub date: String,
    pub tracks: Vec<TrackInfo>,
}

fn s(v: &Value, k: &str) -> String {
    v.get(k).and_then(|x| x.as_str()).unwrap_or("").to_string()
}

fn credit(v: &Value) -> String {
    v.get("artist-credit")
        .and_then(|a| a.as_array())
        .map(|a| {
            a.iter()
                .map(|c| format!("{}{}", s(c, "name"), s(c, "joinphrase")))
                .collect::<String>()
        })
        .unwrap_or_default()
}

/// Works for both /discid/<id> and /discid/-?toc= responses.
/// Picks, per release, the medium that lists `disc_id` or else matches `ntracks`.
pub fn candidates(json: &Value, disc_id: &str, ntracks: usize) -> Vec<Candidate> {
    let mut out = Vec::new();
    let Some(rels) = json.get("releases").and_then(|r| r.as_array()) else {
        return out;
    };
    for r in rels {
        let Some(media) = r.get("media").and_then(|m| m.as_array()) else { continue };
        let by_id = media.iter().find(|m| {
            m.get("discs")
                .and_then(|d| d.as_array())
                .map(|d| d.iter().any(|x| s(x, "id") == disc_id))
                .unwrap_or(false)
        });
        let by_count = media.iter().find(|m| {
            m.get("tracks").and_then(|t| t.as_array()).map(|t| t.len()) == Some(ntracks)
        });
        let Some(m) = by_id.or(by_count) else { continue };
        let Some(tr) = m.get("tracks").and_then(|t| t.as_array()) else { continue };
        let rel_artist = credit(r);
        let tracks = tr
            .iter()
            .map(|t| {
                let a = credit(t);
                TrackInfo {
                    title: s(t, "title"),
                    artist: if a.is_empty() { rel_artist.clone() } else { a },
                }
            })
            .collect();
        out.push(Candidate {
            release_id: s(r, "id"),
            title: s(r, "title"),
            artist: rel_artist,
            date: s(r, "date"),
            tracks,
        });
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn parses() {
        let j: Value = serde_json::from_str(
            r#"{"releases":[{"id":"r1","title":"Alb","date":"2003","artist-credit":[{"name":"Art","joinphrase":""}],
            "media":[{"discs":[{"id":"D"}],"tracks":[{"title":"A"},{"title":"B","artist-credit":[{"name":"X","joinphrase":" & "},{"name":"Y","joinphrase":""}]}]}]}]}"#,
        ).unwrap();
        let c = candidates(&j, "D", 2);
        assert_eq!(c.len(), 1);
        assert_eq!(c[0].tracks[0].artist, "Art");
        assert_eq!(c[0].tracks[1].artist, "X & Y");
        assert_eq!(candidates(&j, "Z", 3).len(), 0);
    }
}
