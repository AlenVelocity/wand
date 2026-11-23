//! Recording and replaying controller traffic.
//!
//! A `.wandrec` file is JSON lines. The first line is a header; every other
//! line is `[microseconds, "hex of one 63-byte input report"]`. Raw reports
//! are stored, not decoded values, so a replay exercises the parser, the
//! fusion, the projection and the recogniser exactly as the live controller
//! does.

use crate::protocol::InputReport;
use serde::{Deserialize, Serialize};
use std::io::{BufRead, BufWriter, Write};
use std::path::Path;

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct Header {
    pub wandrec: u32,
    /// "joycon" for captured traffic, "synthetic" for generated.
    pub source: String,
    #[serde(default)]
    pub product_id: u16,
    #[serde(default)]
    pub note: String,
}

pub struct Recorder {
    out: BufWriter<std::fs::File>,
    t0: Option<f64>,
    pub reports: u64,
}

impl Recorder {
    pub fn create(path: &Path, header: &Header) -> anyhow::Result<Recorder> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let mut out = BufWriter::new(std::fs::File::create(path)?);
        writeln!(out, "{}", serde_json::to_string(header)?)?;
        Ok(Recorder { out, t0: None, reports: 0 })
    }

    /// `host_s`: monotonic arrival time in seconds.
    pub fn push(&mut self, host_s: f64, raw: &[u8]) -> anyhow::Result<()> {
        let t0 = *self.t0.get_or_insert(host_s);
        let us = ((host_s - t0) * 1e6).max(0.0).round() as u64;
        writeln!(self.out, "[{us},\"{}\"]", hex(raw))?;
        self.reports += 1;
        Ok(())
    }

    pub fn flush(&mut self) -> anyhow::Result<()> {
        self.out.flush()?;
        Ok(())
    }

    pub fn finish(mut self) -> anyhow::Result<u64> {
        self.out.flush()?;
        Ok(self.reports)
    }
}

fn hex(bytes: &[u8]) -> String {
    const DIGITS: &[u8; 16] = b"0123456789abcdef";
    let mut s = String::with_capacity(bytes.len() * 2);
    for b in bytes {
        s.push(DIGITS[(b >> 4) as usize] as char);
        s.push(DIGITS[(b & 15) as usize] as char);
    }
    s
}

fn unhex(s: &str) -> Option<Vec<u8>> {
    if s.len() % 2 != 0 {
        return None;
    }
    (0..s.len()).step_by(2).map(|i| u8::from_str_radix(s.get(i..i + 2)?, 16).ok()).collect()
}

#[derive(Clone, Debug)]
pub struct Recording {
    pub header: Header,
    /// (microseconds from start, raw report)
    pub reports: Vec<(u64, Vec<u8>)>,
}

impl Recording {
    pub fn load(path: &Path) -> anyhow::Result<Recording> {
        let file = std::io::BufReader::new(std::fs::File::open(path)?);
        let mut lines = file.lines();
        let first = lines.next().ok_or_else(|| anyhow::anyhow!("empty recording"))??;
        let header: Header = serde_json::from_str(&first).map_err(|e| anyhow::anyhow!("not a wandrec file: {e}"))?;
        if header.wandrec != 1 {
            anyhow::bail!("unsupported wandrec version {}", header.wandrec);
        }
        let mut reports = Vec::new();
        for (i, line) in lines.enumerate() {
            let line = line?;
            if line.trim().is_empty() {
                continue;
            }
            let (us, hex): (u64, String) =
                serde_json::from_str(&line).map_err(|e| anyhow::anyhow!("line {}: {e}", i + 2))?;
            let raw = unhex(&hex).ok_or_else(|| anyhow::anyhow!("line {}: bad hex", i + 2))?;
            reports.push((us, raw));
        }
        Ok(Recording { header, reports })
    }

    pub fn duration_s(&self) -> f64 {
        self.reports.last().map_or(0.0, |(us, _)| *us as f64 * 1e-6)
    }

    /// Parsed reports; lines that are not valid input reports are skipped.
    pub fn parsed(&self) -> impl Iterator<Item = (u64, InputReport)> + '_ {
        self.reports.iter().filter_map(|(us, raw)| InputReport::parse(raw).map(|r| (*us, r)))
    }
}

pub fn write_synthetic(path: &Path, note: &str, reports: &[(u64, InputReport)]) -> anyhow::Result<()> {
    let header = Header {
        wandrec: 1,
        source: "synthetic".into(),
        product_id: crate::protocol::PID_JOYCON_R,
        note: note.into(),
    };
    let mut rec = Recorder::create(path, &header)?;
    for (us, r) in reports {
        rec.push(*us as f64 * 1e-6, &r.encode())?;
    }
    rec.finish()?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::protocol::btn;

    #[test]
    fn round_trip() {
        let dir = std::env::temp_dir().join(format!("wand-rec-test-{}", std::process::id()));
        let path = dir.join("a.wandrec");
        let reports: Vec<(u64, InputReport)> = (0..50u32)
            .map(|i| (i as u64 * 8000, InputReport { counter: i, buttons: if i > 10 { btn::ZR } else { 0 }, gyro: [i as i16, -3, 9], ..Default::default() }))
            .collect();
        write_synthetic(&path, "test", &reports).unwrap();
        let rec = Recording::load(&path).unwrap();
        assert_eq!(rec.header.source, "synthetic");
        assert_eq!(rec.reports.len(), 50);
        assert!((rec.duration_s() - 0.392).abs() < 1e-6);
        let back: Vec<(u64, InputReport)> = rec.parsed().collect();
        assert_eq!(back, reports);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn rejects_other_files() {
        let dir = std::env::temp_dir().join(format!("wand-rec-test2-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("b.wandrec");
        std::fs::write(&path, "hello\n").unwrap();
        assert!(Recording::load(&path).is_err());
        std::fs::write(&path, "{\"wandrec\":1,\"source\":\"joycon\"}\n[0,\"zz\"]\n").unwrap();
        assert!(Recording::load(&path).is_err());
        let _ = std::fs::remove_dir_all(&dir);
    }
}
