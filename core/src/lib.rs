//! DISC.TOC core: pure functions (no OS access). TOC parsing, disc IDs, WAV header.
use base64::{engine::general_purpose::STANDARD, Engine};
pub mod export;
pub mod mb;
#[cfg(windows)]
pub mod cd_win;
#[cfg(target_os = "linux")]
pub mod cd_linux;

#[cfg(windows)]
pub use cd_win as cd;
#[cfg(target_os = "linux")]
pub use cd_linux as cd;

use sha1::{Digest, Sha1};

pub const SECTOR_BYTES: usize = 2352;
pub const LEADIN: u32 = 150;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Toc {
    pub first: u8,
    pub last: u8,
    /// start LBA (without the 150 lead-in) for tracks first..=last
    pub starts: Vec<u32>,
    /// lead-out LBA (without 150)
    pub leadout: u32,
}

impl Toc {
    /// Parse the 804-byte buffer returned by IOCTL_CDROM_READ_TOC.
    pub fn parse_ioctl(buf: &[u8]) -> Result<Toc, String> {
        if buf.len() < 4 {
            return Err("TOC buffer too short".into());
        }
        let first = buf[2];
        let last = buf[3];
        if first == 0 || last < first || last > 99 {
            return Err(format!("invalid track range {first}..{last}"));
        }
        let n = (last - first + 2) as usize; // tracks + lead-out
        if buf.len() < 4 + 8 * n {
            return Err("TOC buffer truncated".into());
        }
        let lba = |e: &[u8]| (e[5] as i64 * 60 + e[6] as i64) * 75 + e[7] as i64 - LEADIN as i64;
        let mut starts = Vec::new();
        let mut leadout = 0u32;
        for i in 0..n {
            let e = &buf[4 + 8 * i..4 + 8 * i + 8];
            let l = lba(e).max(0) as u32;
            if e[2] == 0xAA {
                leadout = l;
            } else {
                starts.push(l);
            }
        }
        if starts.len() != (last - first + 1) as usize || leadout == 0 {
            return Err("TOC entries inconsistent".into());
        }
        Ok(Toc { first, last, starts, leadout })
    }

    pub fn track_count(&self) -> usize {
        self.starts.len()
    }

    /// (start_lba, sector_count) of 0-based track index
    pub fn track_range(&self, idx: usize) -> Option<(u32, u32)> {
        let s = *self.starts.get(idx)?;
        let e = *self.starts.get(idx + 1).unwrap_or(&self.leadout);
        Some((s, e.saturating_sub(s)))
    }

    /// offsets as MusicBrainz uses them (+150): [leadout, t1, t2, ...]
    pub fn mb_offsets(&self) -> Vec<u32> {
        let mut v = vec![self.leadout + LEADIN];
        v.extend(self.starts.iter().map(|s| s + LEADIN));
        v
    }

    pub fn musicbrainz_id(&self) -> String {
        let mut s = format!("{:02X}{:02X}", self.first, self.last);
        let off = self.mb_offsets();
        for i in 0..100usize {
            let o = if i == 0 {
                off[0]
            } else if i >= self.first as usize && i <= self.last as usize {
                self.starts[i - self.first as usize] + LEADIN
            } else {
                0
            };
            s.push_str(&format!("{:08X}", o));
        }
        let digest = Sha1::digest(s.as_bytes());
        STANDARD
            .encode(digest)
            .replace('+', ".")
            .replace('/', "_")
            .replace('=', "-")
    }

    pub fn cddb_id(&self) -> String {
        fn digit_sum(mut n: u32) -> u32 {
            let mut s = 0;
            while n > 0 {
                s += n % 10;
                n /= 10;
            }
            s
        }
        let n: u32 = self.starts.iter().map(|s| digit_sum((s + LEADIN) / 75)).sum();
        let t = (self.leadout + LEADIN) / 75 - (self.starts[0] + LEADIN) / 75;
        let id = ((n % 255) << 24) | (t << 8) | self.starts.len() as u32;
        format!("{:08x}", id)
    }

    /// Query string value for /ws/2/discid/-?toc=...
    pub fn mb_toc_param(&self) -> String {
        let mut p = vec![self.first.to_string(), self.last.to_string()];
        p.extend(self.mb_offsets().iter().map(|o| o.to_string()));
        p.join("+")
    }

    pub fn duration_secs(&self, idx: usize) -> u32 {
        self.track_range(idx).map(|(_, n)| n / 75).unwrap_or(0)
    }
}

/// 44-byte PCM WAV header for 44.1kHz/16bit/stereo.
pub fn wav_header(data_len: u32) -> [u8; 44] {
    let mut h = [0u8; 44];
    h[0..4].copy_from_slice(b"RIFF");
    h[4..8].copy_from_slice(&(36 + data_len).to_le_bytes());
    h[8..16].copy_from_slice(b"WAVEfmt ");
    h[16..20].copy_from_slice(&16u32.to_le_bytes());
    h[20..22].copy_from_slice(&1u16.to_le_bytes());
    h[22..24].copy_from_slice(&2u16.to_le_bytes());
    h[24..28].copy_from_slice(&44100u32.to_le_bytes());
    h[28..32].copy_from_slice(&(44100u32 * 4).to_le_bytes());
    h[32..34].copy_from_slice(&4u16.to_le_bytes());
    h[34..36].copy_from_slice(&16u16.to_le_bytes());
    h[36..40].copy_from_slice(b"data");
    h[40..44].copy_from_slice(&data_len.to_le_bytes());
    h
}

#[cfg(test)]
mod tests {
    use super::*;

    fn lain_toc() -> Toc {
        let offs: [u32; 25] = [
            150, 2250, 8675, 10000, 23612, 33440, 57187, 70012, 89795, 108100, 110732, 132450,
            135012, 163637, 174725, 184837, 200300, 213962, 217775, 230562, 235070, 254170,
            273487, 275950, 289812,
        ];
        Toc {
            first: 1,
            last: 25,
            starts: offs.iter().map(|o| o - 150).collect(),
            leadout: 297862,
        }
    }

    #[test]
    fn mb_id() {
        assert_eq!(lain_toc().musicbrainz_id(), "u00xXJbXMae4i7Yxq4Q5LV3gmjI-");
    }
    #[test]
    fn cddb() {
        assert_eq!(lain_toc().cddb_id(), "5c0f8319");
    }
    #[test]
    fn toc_param() {
        assert!(lain_toc().mb_toc_param().starts_with("1+25+298012+150+2250"));
    }
    #[test]
    fn parse_roundtrip() {
        let t = lain_toc();
        let mut buf = vec![0u8; 804];
        buf[2] = 1;
        buf[3] = 25;
        let put = |buf: &mut Vec<u8>, i: usize, tno: u8, lba: u32| {
            let a = lba + 150;
            let o = 4 + 8 * i;
            buf[o + 2] = tno;
            buf[o + 5] = (a / 75 / 60) as u8;
            buf[o + 6] = (a / 75 % 60) as u8;
            buf[o + 7] = (a % 75) as u8;
        };
        for (i, s) in t.starts.iter().enumerate() {
            put(&mut buf, i, i as u8 + 1, *s);
        }
        put(&mut buf, 25, 0xAA, t.leadout);
        assert_eq!(Toc::parse_ioctl(&buf).unwrap(), t);
    }
    #[test]
    fn track_range_last() {
        let t = lain_toc();
        assert_eq!(t.track_range(24), Some((289662, 297862 - 289662)));
    }
    #[test]
    fn wav() {
        let h = wav_header(1000);
        assert_eq!(&h[0..4], b"RIFF");
        assert_eq!(u32::from_le_bytes(h[4..8].try_into().unwrap()), 1036);
    }
}
