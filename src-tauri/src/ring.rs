//! In-memory replay buffer.
//!
//! FFmpeg writes one MPEG-TS stream to stdout. `Ring` cuts that stream at
//! each video keyframe (every 2 s) and keeps the newest chunks in RAM. A
//! slice of chunks is a valid TS stream once the latest PAT and PMT are put
//! in front, so saving a clip writes those bytes to one file and remuxes it
//! without re-encoding. Nothing is written to disk while you play.

use parking_lot::{Condvar, Mutex};
use std::collections::VecDeque;
use std::io::{Read, Write};
use std::sync::Arc;
use std::time::Duration;

const TS: usize = 188;
const PTS_WRAP: f64 = (1u64 << 33) as f64 / 90_000.0;

/// About two seconds of video, starting at a keyframe.
#[derive(Clone)]
pub struct Chunk {
    /// Counts up from 1 for each chunk in a run.
    pub seq: u64,
    pub start: f64,
    pub end: f64,
    pub bytes: Arc<[u8]>,
}

#[derive(Default)]
struct State {
    chunks: VecDeque<Chunk>,
    keep_secs: f64,
    /// The chunk in progress and the time of its keyframe.
    open: Vec<u8>,
    open_start: Option<f64>,
    next_seq: u64,
    pat: Option<[u8; TS]>,
    pmt: Option<[u8; TS]>,
    pmt_pid: Option<u16>,
    pcr_pid: Option<u16>,
    /// Adds 2^33 ticks each time the 33-bit PTS wraps (every ~26.5 h).
    pts_base: f64,
    last_pts: f64,
    ended: bool,
}

pub struct Ring {
    state: Mutex<State>,
    changed: Condvar,
}

impl Ring {
    /// Keeps at least `keep_secs` of closed chunks.
    pub fn new(keep_secs: f64) -> Arc<Self> {
        Arc::new(Self {
            state: Mutex::new(State { keep_secs, next_seq: 1, ..Default::default() }),
            changed: Condvar::new(),
        })
    }

    /// Reads a TS stream until EOF on a new thread.
    pub fn pump(self: &Arc<Self>, mut input: impl Read + Send + 'static) {
        let ring = self.clone();
        let spawned = std::thread::Builder::new().name("ring-reader".into()).spawn(move || {
            let mut buf = vec![0u8; TS * 512];
            let mut filled = 0;
            loop {
                match input.read(&mut buf[filled..]) {
                    Ok(0) | Err(_) => break,
                    Ok(n) => filled += n,
                }
                let whole = filled - filled % TS;
                ring.push(&buf[..whole]);
                buf.copy_within(whole..filled, 0);
                filled -= whole;
            }
            ring.state.lock().ended = true;
            ring.changed.notify_all();
        });
        if spawned.is_err() {
            self.state.lock().ended = true;
        }
    }

    /// Adds whole TS packets.
    pub fn push(&self, data: &[u8]) {
        let mut st = self.state.lock();
        let mut closed = false;
        for pkt in data.chunks_exact(TS) {
            if pkt[0] != 0x47 {
                continue;
            }
            let pid = (u16::from(pkt[1] & 0x1f) << 8) | u16::from(pkt[2]);
            let pusi = pkt[1] & 0x40 != 0;
            if pid == 0 && pusi {
                st.pmt_pid = pat_pmt_pid(pkt).or(st.pmt_pid);
                st.pat = pkt.try_into().ok();
            } else if Some(pid) == st.pmt_pid && pusi {
                st.pcr_pid = pmt_pcr_pid(pkt).or(st.pcr_pid);
                st.pmt = pkt.try_into().ok();
            } else if Some(pid) == st.pcr_pid && pusi && random_access(pkt) {
                if let Some(pts) = pes_pts(pkt) {
                    let mut t = pts + st.pts_base;
                    if t + PTS_WRAP / 2.0 < st.last_pts {
                        st.pts_base += PTS_WRAP;
                        t += PTS_WRAP;
                    }
                    st.last_pts = t;
                    closed |= st.close(t);
                    st.open_start = Some(t);
                }
            }
            if st.open_start.is_some() {
                st.open.extend_from_slice(pkt);
            }
        }
        drop(st);
        if closed {
            self.changed.notify_all();
        }
    }

    /// Seq of the newest closed chunk, or 0.
    pub fn last_seq(&self) -> u64 {
        self.state.lock().chunks.back().map_or(0, |c| c.seq)
    }

    /// Waits until a chunk newer than `seq` closes. False on timeout or end of stream.
    pub fn wait_past(&self, seq: u64, timeout: Duration) -> bool {
        let mut st = self.state.lock();
        let deadline = std::time::Instant::now() + timeout;
        loop {
            if st.chunks.back().is_some_and(|c| c.seq > seq) {
                return true;
            }
            if st.ended || self.changed.wait_until(&mut st, deadline).timed_out() {
                return st.chunks.back().is_some_and(|c| c.seq > seq);
            }
        }
    }

    /// The newest chunks that add up to at least `seconds`, oldest first.
    pub fn last(&self, seconds: f64) -> Vec<Chunk> {
        let st = self.state.lock();
        let mut out = Vec::new();
        let mut total = 0.0;
        for c in st.chunks.iter().rev() {
            if total >= seconds - 0.5 {
                break;
            }
            total += c.end - c.start;
            out.push(c.clone());
        }
        out.reverse();
        out
    }

    /// Closed chunks with a seq above `seq`, oldest first.
    pub fn after(&self, seq: u64) -> Vec<Chunk> {
        self.state.lock().chunks.iter().filter(|c| c.seq > seq).cloned().collect()
    }

    /// PAT and PMT packets that make a slice of chunks readable on its own.
    pub fn headers(&self) -> Vec<u8> {
        let st = self.state.lock();
        st.pat.iter().chain(st.pmt.iter()).flatten().copied().collect()
    }
}

impl State {
    /// Closes the open chunk at time `end`. True when a chunk was added.
    fn close(&mut self, end: f64) -> bool {
        let Some(start) = self.open_start.take() else {
            return false;
        };
        let bytes: Arc<[u8]> = std::mem::take(&mut self.open).into();
        self.chunks.push_back(Chunk { seq: self.next_seq, start, end: end.max(start), bytes });
        self.next_seq += 1;
        // Drop the oldest chunk while the rest still cover `keep_secs`.
        while let Some(first) = self.chunks.front() {
            let total: f64 = self.chunks.iter().map(|c| c.end - c.start).sum();
            if total - (first.end - first.start) < self.keep_secs {
                break;
            }
            self.chunks.pop_front();
        }
        true
    }
}

/// Writes `chunks` as one TS file that FFmpeg can read on its own.
pub fn write_ts(out: &mut impl Write, headers: &[u8], chunks: &[Chunk]) -> std::io::Result<()> {
    out.write_all(headers)?;
    for c in chunks {
        out.write_all(&c.bytes)?;
    }
    out.flush()
}

// ---------------------------------------------------------------- TS parsing

/// Payload of a packet, after the header and adaptation field.
fn payload(pkt: &[u8]) -> Option<&[u8]> {
    let afc = (pkt[3] >> 4) & 3;
    let start = match afc {
        1 => 4,
        3 => 5 + usize::from(pkt[4]),
        _ => return None,
    };
    pkt.get(start..)
}

fn random_access(pkt: &[u8]) -> bool {
    (pkt[3] >> 4) & 2 != 0 && pkt[4] > 0 && pkt[5] & 0x40 != 0
}

/// Start of a PSI section, after the pointer field.
fn section(pkt: &[u8]) -> Option<&[u8]> {
    let p = payload(pkt)?;
    p.get(1 + usize::from(*p.first()?)..)
}

/// First program's PMT PID in a PAT.
fn pat_pmt_pid(pkt: &[u8]) -> Option<u16> {
    let s = section(pkt)?;
    if *s.first()? != 0x00 {
        return None;
    }
    let len = (usize::from(s[1] & 0x0f) << 8) | usize::from(s[2]);
    // Program loop between the 8-byte header and the 4-byte CRC.
    let progs = s.get(8..(3 + len).checked_sub(4)?)?;
    progs.chunks_exact(4).find_map(|p| {
        let program = u16::from_be_bytes([p[0], p[1]]);
        (program != 0).then(|| (u16::from(p[2] & 0x1f) << 8) | u16::from(p[3]))
    })
}

/// PCR PID in a PMT. FFmpeg puts the PCR on the video stream.
fn pmt_pcr_pid(pkt: &[u8]) -> Option<u16> {
    let s = section(pkt)?;
    if *s.first()? != 0x02 {
        return None;
    }
    Some((u16::from(*s.get(8)? & 0x1f) << 8) | u16::from(*s.get(9)?))
}

/// PTS in seconds from a packet that starts a PES.
fn pes_pts(pkt: &[u8]) -> Option<f64> {
    let p = payload(pkt)?;
    if p.get(..3)? != [0, 0, 1] || p.get(7)? & 0x80 == 0 {
        return None;
    }
    let b = p.get(9..14)?;
    let pts = (u64::from(b[0] >> 1 & 7) << 30)
        | (u64::from(b[1]) << 22)
        | (u64::from(b[2] >> 1) << 15)
        | (u64::from(b[3]) << 7)
        | u64::from(b[4] >> 1);
    Some(pts as f64 / 90_000.0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::process::{Command, Stdio};

    /// `secs` of 30 fps test video with audio, a keyframe every 2 s, as MPEG-TS.
    fn sample(secs: u32) -> Option<Vec<u8>> {
        let out = Command::new("ffmpeg")
            .args(["-hide_banner", "-loglevel", "error", "-nostdin"])
            .args(["-f", "lavfi", "-i", &format!("testsrc2=s=320x180:r=30:d={secs}")])
            .args(["-f", "lavfi", "-i", &format!("sine=f=440:d={secs}")])
            .args(["-c:v", "libx264", "-preset", "ultrafast", "-g", "60", "-bf", "0"])
            .args(["-force_key_frames", "expr:gte(t,n_forced*2)", "-c:a", "aac"])
            .args(["-f", "mpegts", "pipe:1"])
            .stderr(Stdio::inherit())
            .output()
            .ok()?;
        out.status.success().then_some(out.stdout)
    }

    fn duration(path: &std::path::Path) -> f64 {
        let out = Command::new("ffprobe")
            .args(["-v", "error", "-show_entries", "format=duration", "-of", "csv=p=0"])
            .arg(path)
            .output()
            .unwrap();
        String::from_utf8_lossy(&out.stdout).trim().parse().unwrap()
    }

    #[test]
    fn cuts_at_keyframes_and_keeps_window() {
        let Some(ts) = sample(20) else {
            eprintln!("ffmpeg not found, skipping");
            return;
        };
        let ring = Ring::new(6.0);
        // Feed in odd sizes, as a pipe would.
        for part in ts.chunks(TS * 37) {
            ring.push(part);
        }
        let all = ring.after(0);
        assert!(all.len() >= 3 && all.len() <= 5, "kept {} chunks", all.len());
        for c in &all {
            assert!((c.end - c.start - 2.0).abs() < 0.05, "chunk {}..{}", c.start, c.end);
            assert_eq!(c.bytes.len() % TS, 0);
        }
        assert!(all.windows(2).all(|w| w[1].seq == w[0].seq + 1 && (w[1].start - w[0].end).abs() < 1e-9));
        assert_eq!(ring.last(4.0).len(), 2);
        assert_eq!(ring.headers().len(), TS * 2);

        // A slice from the middle of the stream must remux on its own.
        let dir = std::env::temp_dir().join(format!("rewind-ring-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let ts_path = dir.join("clip.ts");
        let mp4 = dir.join("clip.mp4");
        let mut f = std::fs::File::create(&ts_path).unwrap();
        write_ts(&mut f, &ring.headers(), &ring.last(4.0)).unwrap();
        drop(f);
        let ok = Command::new("ffmpeg")
            .args(["-hide_banner", "-loglevel", "error", "-y", "-i"])
            .arg(&ts_path)
            .args(["-map", "0", "-c", "copy"])
            .arg(&mp4)
            .status()
            .unwrap()
            .success();
        assert!(ok);
        let d = duration(&mp4);
        assert!((d - 4.0).abs() < 0.3, "clip is {d} s");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn wait_past_sees_new_chunk() {
        let Some(ts) = sample(6) else {
            return;
        };
        let ring = Ring::new(60.0);
        let half = ts.len() / 2 / TS * TS;
        ring.push(&ts[..half]);
        let seq = ring.last_seq();
        assert!(!ring.wait_past(seq, Duration::from_millis(50)));
        ring.pump(std::io::Cursor::new(ts[half..].to_vec()));
        assert!(ring.wait_past(seq, Duration::from_secs(5)));
    }
}
