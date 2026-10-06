//! Linux: read TOC / raw CDDA sectors from /dev/sr* via ioctl (no root needed if the user can open the device).
use crate::{Toc, LEADIN, SECTOR_BYTES};
use std::fs::{File, OpenOptions};
use std::os::unix::fs::OpenOptionsExt;
use std::os::unix::io::AsRawFd;

// <linux/cdrom.h>
const CDROMREADTOCHDR: libc::c_ulong = 0x5305;
const CDROMREADTOCENTRY: libc::c_ulong = 0x5306;
const CDROMREADAUDIO: libc::c_ulong = 0x530E;
const CDROM_MSF: u8 = 0x02;
const CDROM_LBA: u8 = 0x01;
const CDROM_LEADOUT: u8 = 0xAA;
/// sectors per CDROMREADAUDIO call
const CHUNK: u32 = 16;

#[repr(C)]
#[derive(Default)]
struct TocHdr {
    first: u8,
    last: u8,
}

#[repr(C)]
#[derive(Default)]
struct TocEntry {
    track: u8,
    adr_ctrl: u8,
    format: u8,
    addr: [u8; 4], // msf: minute, second, frame, 0   /  lba: i32
    datamode: u8,
}

#[repr(C)]
struct ReadAudio {
    addr: [u8; 4],
    addr_format: u8,
    nframes: i32,
    buf: *mut u8,
}

/// e.g. ["/dev/sr0"]
pub fn list_drives() -> Vec<String> {
    let mut out = Vec::new();
    if let Ok(rd) = std::fs::read_dir("/dev") {
        for e in rd.flatten() {
            let name = e.file_name().to_string_lossy().to_string();
            if name.starts_with("sr") && name[2..].chars().all(|c| c.is_ascii_digit()) && name.len() > 2 {
                out.push(format!("/dev/{name}"));
            }
        }
    }
    out.sort();
    out
}

pub struct Drive(File);

impl Drive {
    pub fn open(path: &str) -> Result<Drive, String> {
        if !path.starts_with("/dev/sr") || path[7..].is_empty() || !path[7..].chars().all(|c| c.is_ascii_digit()) {
            return Err("invalid drive".into());
        }
        // O_NONBLOCK: open succeeds even when no disc is inserted; errors then show up in the ioctl
        let f = OpenOptions::new()
            .read(true)
            .custom_flags(libc::O_NONBLOCK)
            .open(path)
            .map_err(|e| format!("open {path} failed: {e}"))?;
        Ok(Drive(f))
    }

    pub fn read_toc(&self) -> Result<Toc, String> {
        let fd = self.0.as_raw_fd();
        let mut hdr = TocHdr::default();
        if unsafe { libc::ioctl(fd, CDROMREADTOCHDR, &mut hdr as *mut TocHdr) } != 0 {
            return Err(format!("READ_TOC failed ({}) - no disc?", std::io::Error::last_os_error()));
        }
        let (first, last) = (hdr.first, hdr.last);
        if first == 0 || last < first || last > 99 {
            return Err(format!("invalid track range {first}..{last}"));
        }
        let entry = |track: u8| -> Result<i64, String> {
            let mut e = TocEntry { track, format: CDROM_MSF, ..Default::default() };
            if unsafe { libc::ioctl(fd, CDROMREADTOCENTRY, &mut e as *mut TocEntry) } != 0 {
                return Err(format!("READ_TOC_ENTRY {track} failed ({})", std::io::Error::last_os_error()));
            }
            let (m, s, f) = (e.addr[0] as i64, e.addr[1] as i64, e.addr[2] as i64);
            Ok(((m * 60 + s) * 75 + f - LEADIN as i64).max(0))
        };
        let mut starts = Vec::new();
        for t in first..=last {
            starts.push(entry(t)? as u32);
        }
        let leadout = entry(CDROM_LEADOUT)? as u32;
        if leadout == 0 {
            return Err("TOC entries inconsistent".into());
        }
        Ok(Toc { first, last, starts, leadout })
    }

    /// Read `count` sectors starting at `lba` into one buffer (count*2352 bytes).
    pub fn read_pcm(&self, lba: u32, count: u32) -> Result<Vec<u8>, String> {
        let mut out = Vec::with_capacity(count as usize * SECTOR_BYTES);
        self.read_audio(lba, count, |b| {
            out.extend_from_slice(b);
            Ok(())
        })?;
        Ok(out)
    }

    /// Read `count` CDDA sectors starting at `lba`; calls `sink` with each chunk of PCM bytes.
    pub fn read_audio(
        &self,
        lba: u32,
        count: u32,
        mut sink: impl FnMut(&[u8]) -> Result<(), String>,
    ) -> Result<(), String> {
        let fd = self.0.as_raw_fd();
        let mut buf = vec![0u8; CHUNK as usize * SECTOR_BYTES];
        let mut done = 0u32;
        while done < count {
            let n = CHUNK.min(count - done);
            let want = n as usize * SECTOR_BYTES;
            let mut ok = false;
            // retry a few times: scratched discs often succeed on retry
            for _ in 0..3 {
                let mut ra = ReadAudio {
                    addr: ((lba + done) as i32).to_ne_bytes(),
                    addr_format: CDROM_LBA,
                    nframes: n as i32,
                    buf: buf.as_mut_ptr(),
                };
                if unsafe { libc::ioctl(fd, CDROMREADAUDIO, &mut ra as *mut ReadAudio) } == 0 {
                    ok = true;
                    break;
                }
            }
            if !ok {
                return Err(format!(
                    "READAUDIO failed at LBA {} ({})",
                    lba + done,
                    std::io::Error::last_os_error()
                ));
            }
            sink(&buf[..want])?;
            done += n;
        }
        Ok(())
    }
}
