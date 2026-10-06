//! Windows: read TOC / raw CDDA sectors from an optical drive without admin rights.
use crate::{Toc, SECTOR_BYTES};
use windows_sys::Win32::Foundation::{CloseHandle, GetLastError, HANDLE, INVALID_HANDLE_VALUE};
use windows_sys::Win32::Storage::FileSystem::{
    CreateFileW, GetDriveTypeW, GetLogicalDrives, FILE_SHARE_READ, FILE_SHARE_WRITE, OPEN_EXISTING,
};
use windows_sys::Win32::System::IO::DeviceIoControl;

const GENERIC_READ: u32 = 0x8000_0000;
const DRIVE_CDROM: u32 = 5;
const IOCTL_CDROM_READ_TOC: u32 = 0x0002_4000;
const IOCTL_CDROM_RAW_READ: u32 = 0x0002_403E;
const TRACK_MODE_CDDA: u32 = 2;
/// sectors per raw read call (kept small; some drivers fail above ~25)
const CHUNK: u32 = 16;

fn wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(std::iter::once(0)).collect()
}

/// e.g. ["D:", "F:"]
pub fn list_drives() -> Vec<String> {
    let mask = unsafe { GetLogicalDrives() };
    let mut out = Vec::new();
    for i in 0..26u32 {
        if mask & (1 << i) != 0 {
            let letter = (b'A' + i as u8) as char;
            let root = wide(&format!("{letter}:\\"));
            if unsafe { GetDriveTypeW(root.as_ptr()) } == DRIVE_CDROM {
                out.push(format!("{letter}:"));
            }
        }
    }
    out
}

pub struct Drive(HANDLE);

// The handle is only ever used from one thread at a time (callers serialize with a Mutex).
unsafe impl Send for Drive {}

impl Drive {
    pub fn open(letter: &str) -> Result<Drive, String> {
        let letter = letter.trim_end_matches(['\\', ':']);
        if letter.len() != 1 || !letter.chars().all(|c| c.is_ascii_alphabetic()) {
            return Err("invalid drive".into());
        }
        let path = wide(&format!("\\\\.\\{}:", letter.to_ascii_uppercase()));
        let h = unsafe {
            CreateFileW(
                path.as_ptr(),
                GENERIC_READ,
                FILE_SHARE_READ | FILE_SHARE_WRITE,
                std::ptr::null(),
                OPEN_EXISTING,
                0,
                std::ptr::null_mut(),
            )
        };
        if h == INVALID_HANDLE_VALUE {
            return Err(format!("open failed (error {})", unsafe { GetLastError() }));
        }
        Ok(Drive(h))
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

    pub fn read_toc(&self) -> Result<Toc, String> {
        let mut buf = vec![0u8; 804];
        let mut ret = 0u32;
        let ok = unsafe {
            DeviceIoControl(
                self.0,
                IOCTL_CDROM_READ_TOC,
                std::ptr::null(),
                0,
                buf.as_mut_ptr() as *mut _,
                buf.len() as u32,
                &mut ret,
                std::ptr::null_mut(),
            )
        };
        if ok == 0 {
            return Err(format!("READ_TOC failed (error {}) - no disc?", unsafe { GetLastError() }));
        }
        Toc::parse_ioctl(&buf[..ret as usize])
    }

    /// Read `count` CDDA sectors starting at `lba`; calls `sink` with each chunk of PCM bytes.
    pub fn read_audio(
        &self,
        lba: u32,
        count: u32,
        mut sink: impl FnMut(&[u8]) -> Result<(), String>,
    ) -> Result<(), String> {
        let mut buf = vec![0u8; CHUNK as usize * SECTOR_BYTES];
        let mut done = 0u32;
        while done < count {
            let n = CHUNK.min(count - done);
            // RAW_READ_INFO { i64 DiskOffset; u32 SectorCount; u32 TrackMode } = 16 bytes
            let mut info = [0u8; 16];
            info[0..8].copy_from_slice(&(((lba + done) as i64) * 2048).to_le_bytes());
            info[8..12].copy_from_slice(&n.to_le_bytes());
            info[12..16].copy_from_slice(&TRACK_MODE_CDDA.to_le_bytes());
            let want = n as usize * SECTOR_BYTES;
            let mut ret = 0u32;
            let mut ok = 0;
            // retry a few times: scratched discs often succeed on retry
            for _ in 0..3 {
                ok = unsafe {
                    DeviceIoControl(
                        self.0,
                        IOCTL_CDROM_RAW_READ,
                        info.as_ptr() as *const _,
                        16,
                        buf.as_mut_ptr() as *mut _,
                        want as u32,
                        &mut ret,
                        std::ptr::null_mut(),
                    )
                };
                if ok != 0 {
                    break;
                }
            }
            if ok == 0 {
                return Err(format!(
                    "RAW_READ failed at LBA {} (error {})",
                    lba + done,
                    unsafe { GetLastError() }
                ));
            }
            sink(&buf[..want])?;
            done += n;
        }
        Ok(())
    }
}

impl Drop for Drive {
    fn drop(&mut self) {
        unsafe { CloseHandle(self.0) };
    }
}
