//! Minimal ZIP container for .mcpr files: EOCD + central-directory parser,
//! hand-rolled DEFLATE (RFC 1951) inflate for STORE(0)/DEFLATE(8) entries,
//! and a STORE-method writer. Upstream uses java.util.zip.ZipFile/
//! ZipOutputStream; the format is public (PKWARE APPNOTE), no upstream code
//! involved. This file: Copyright (c) 2026 sparticle, LGPL-3.0-or-later.
use std::collections::BTreeMap;

const EOCD_SIG: u32 = 0x0605_4b50;
const CDH_SIG: u32 = 0x0201_4b50;
const LFH_SIG: u32 = 0x0403_4b50;

fn u16le(b: &[u8], o: usize) -> u16 { u16::from_le_bytes([b[o], b[o + 1]]) }
fn u32le(b: &[u8], o: usize) -> u32 { u32::from_le_bytes([b[o], b[o + 1], b[o + 2], b[o + 3]]) }

/// Parse a whole zip into name -> bytes. Supports method 0 (stored) and
/// method 8 (deflate, the mode ReplayMod records .mcpr entries with).
pub fn parse_zip(data: &[u8]) -> Result<BTreeMap<String, Vec<u8>>, String> {
    // Locate the EOCD from the end.
    let mut eocd = None;
    if data.len() >= 22 {
        let start = data.len().saturating_sub(22 + 0xFFFF);
        for i in (start..=data.len() - 22).rev() {
            if u32le(data, i) == EOCD_SIG { eocd = Some(i); break; }
        }
    }
    let eocd = eocd.ok_or("no EOCD found")?;
    let count = u16le(data, eocd + 10) as usize;
    let cd_off = u32le(data, eocd + 16) as usize;
    let mut entries = BTreeMap::new();
    let mut p = cd_off;
    for _ in 0..count {
        if u32le(data, p) != CDH_SIG { return Err("bad central directory entry".into()); }
        let method = u16le(data, p + 10);
        let csize = u32le(data, p + 20) as usize;
        let name_len = u16le(data, p + 28) as usize;
        let extra_len = u16le(data, p + 30) as usize;
        let comment_len = u16le(data, p + 32) as usize;
        let lfh_off = u32le(data, p + 42) as usize;
        let name = String::from_utf8_lossy(&data[p + 46..p + 46 + name_len]).into_owned();
        // Local header: the data starts after LFH + name + extra (its own
        // extra length, which may differ from the central one).
        if u32le(data, lfh_off) != LFH_SIG { return Err("bad local header".into()); }
        let lfh_name = u16le(data, lfh_off + 26) as usize;
        let lfh_extra = u16le(data, lfh_off + 28) as usize;
        let data_start = lfh_off + 30 + lfh_name + lfh_extra;
        let raw = &data[data_start..data_start + csize];
        let bytes = match method {
            0 => raw.to_vec(),
            8 => inflate(raw)?,
            m => return Err(format!("unsupported zip method {}", m)),
        };
        entries.insert(name, bytes);
        p += 46 + name_len + extra_len + comment_len;
    }
    Ok(entries)
}

/// DEFLATE (RFC 1951) decompression, puff.c-style canonical decoding.
struct BitReader<'a> { data: &'a [u8], pos: usize, bitbuf: u32, bitcnt: u32 }
impl<'a> BitReader<'a> {
    fn bits(&mut self, need: u32) -> Result<u32, String> {
        while self.bitcnt < need {
            if self.pos >= self.data.len() { return Err("inflate: out of input".into()); }
            self.bitbuf |= (self.data[self.pos] as u32) << self.bitcnt;
            self.pos += 1;
            self.bitcnt += 8;
        }
        let val = self.bitbuf & ((1u32 << need) - 1);
        self.bitbuf >>= need;
        self.bitcnt -= need;
        Ok(val)
    }
}

struct Huff { counts: [u16; 16], symbols: Vec<u16> }
impl Huff {
    fn construct(lengths: &[u8]) -> Result<Huff, String> {
        let mut h = Huff { counts: [0; 16], symbols: vec![0; lengths.len()] };
        for &l in lengths { h.counts[l as usize] += 1; }
        h.counts[0] = 0;
        let mut left = 1i32;
        for len in 1..=15 {
            left <<= 1;
            left -= h.counts[len] as i32;
            if left < 0 { return Err("inflate: over-subscribed code".into()); }
        }
        let mut offs = [0u16; 16];
        for len in 1..15 { offs[len + 1] = offs[len] + h.counts[len]; }
        for (sym, &l) in lengths.iter().enumerate() {
            if l != 0 { h.symbols[offs[l as usize] as usize] = sym as u16; offs[l as usize] += 1; }
        }
        Ok(h)
    }
    fn decode(&self, br: &mut BitReader) -> Result<i32, String> {
        let mut code: i32 = 0;
        let mut first: i32 = 0;
        let mut index: i32 = 0;
        for len in 1..=15u32 {
            code |= br.bits(1)? as i32;
            let count = self.counts[len as usize] as i32;
            if code - count < first { return Ok(self.symbols[(index + (code - first)) as usize] as i32); }
            index += count;
            first = (first + count) << 1;
            code <<= 1;
        }
        Err("inflate: invalid code".into())
    }
}

const LEN_BASE: [u16; 29] = [3,4,5,6,7,8,9,10,11,13,15,17,19,23,27,31,35,43,51,59,67,83,99,115,131,163,195,227,258];
const LEN_EXTRA: [u8; 29] = [0,0,0,0,0,0,0,0,1,1,1,1,2,2,2,2,3,3,3,3,4,4,4,4,5,5,5,5,0];
const DIST_BASE: [u16; 30] = [1,2,3,4,5,7,9,13,17,25,33,49,65,97,129,193,257,385,513,769,1025,1537,2049,3073,4097,6145,8193,12289,16385,24577];
const DIST_EXTRA: [u8; 30] = [0,0,0,0,1,1,2,2,3,3,4,4,5,5,6,6,7,7,8,8,9,9,10,10,11,11,12,12,13,13];
const CLEN_ORDER: [usize; 19] = [16,17,18,0,8,7,9,6,10,5,11,4,12,3,13,2,14,1,15];

pub fn inflate(src: &[u8]) -> Result<Vec<u8>, String> {
    let mut br = BitReader { data: src, pos: 0, bitbuf: 0, bitcnt: 0 };
    let mut out: Vec<u8> = Vec::new();
    loop {
        let bfinal = br.bits(1)?;
        let btype = br.bits(2)?;
        match btype {
            0 => {
                // Stored block: skip to byte boundary, LEN/NLEN, raw copy.
                br.bitbuf = 0;
                br.bitcnt = 0;
                if br.pos + 4 > src.len() { return Err("inflate: stored header past end".into()); }
                let len = u16le(src, br.pos) as usize;
                let nlen = u16le(src, br.pos + 2) as usize;
                br.pos += 4;
                if len ^ 0xFFFF != nlen { return Err("inflate: stored LEN/NLEN mismatch".into()); }
                if br.pos + len > src.len() { return Err("inflate: stored block past end".into()); }
                out.extend_from_slice(&src[br.pos..br.pos + len]);
                br.pos += len;
            }
            1 => {
                // Fixed Huffman.
                let mut ll = [0u8; 288];
                for (i, l) in ll.iter_mut().enumerate() {
                    *l = if i < 144 { 8 } else if i < 256 { 9 } else if i < 280 { 7 } else { 8 };
                }
                let dl = [5u8; 30];
                inflate_block(&mut br, &mut out, &Huff::construct(&ll)?, &Huff::construct(&dl)?)?;
            }
            2 => {
                // Dynamic Huffman.
                let hlit = br.bits(5)? as usize + 257;
                let hdist = br.bits(5)? as usize + 1;
                let hclen = br.bits(4)? as usize + 4;
                let mut cl = [0u8; 19];
                for i in 0..hclen { cl[CLEN_ORDER[i]] = br.bits(3)? as u8; }
                let clh = Huff::construct(&cl)?;
                let mut lens = vec![0u8; hlit + hdist];
                let mut i = 0;
                while i < hlit + hdist {
                    let sym = clh.decode(&mut br)?;
                    match sym {
                        0..=15 => { lens[i] = sym as u8; i += 1; }
                        16 => {
                            if i == 0 { return Err("inflate: repeat with no previous".into()); }
                            let prev = lens[i - 1];
                            let rep = 3 + br.bits(2)? as usize;
                            for _ in 0..rep { if i >= lens.len() { break; } lens[i] = prev; i += 1; }
                        }
                        17 => { let rep = 3 + br.bits(3)? as usize; i = (i + rep).min(lens.len()); }
                        18 => { let rep = 11 + br.bits(7)? as usize; i = (i + rep).min(lens.len()); }
                        _ => return Err("inflate: bad code-length symbol".into()),
                    }
                }
                if lens[256] == 0 { return Err("inflate: no end-of-block code".into()); }
                let llh = Huff::construct(&lens[..hlit])?;
                let dlh = Huff::construct(&lens[hlit..])?;
                inflate_block(&mut br, &mut out, &llh, &dlh)?;
            }
            _ => return Err("inflate: invalid block type".into()),
        }
        if bfinal == 1 { break; }
    }
    Ok(out)
}

fn inflate_block(br: &mut BitReader, out: &mut Vec<u8>, llh: &Huff, dlh: &Huff) -> Result<(), String> {
    loop {
        let sym = llh.decode(br)?;
        if sym < 256 {
            out.push(sym as u8);
        } else if sym == 256 {
            return Ok(());
        } else {
            let li = (sym - 257) as usize;
            if li >= 29 { return Err("inflate: bad length symbol".into()); }
            let len = LEN_BASE[li] as usize + br.bits(LEN_EXTRA[li] as u32)? as usize;
            let dsym = dlh.decode(br)? as usize;
            if dsym >= 30 { return Err("inflate: bad distance symbol".into()); }
            let dist = DIST_BASE[dsym] as usize + br.bits(DIST_EXTRA[dsym] as u32)? as usize;
            if dist > out.len() { return Err("inflate: distance past output".into()); }
            let start = out.len() - dist;
            for k in 0..len { out.push(out[start + k]); }
        }
    }
}

/// Write a zip with stored (uncompressed) entries: valid for Java's
/// ZipFile and ReplayStudio, sized up front (no data descriptors).
pub fn write_zip_stored(entries: &[(String, Vec<u8>)], crc32: &dyn Fn(&[u8]) -> u32) -> Vec<u8> {
    let mut out: Vec<u8> = Vec::new();
    let mut cd: Vec<u8> = Vec::new();
    for (name, bytes) in entries {
        let lfh_off = out.len() as u32;
        out.extend_from_slice(&LFH_SIG.to_le_bytes());
        out.extend_from_slice(&20u16.to_le_bytes()); // version needed
        out.extend_from_slice(&0u16.to_le_bytes());  // flags
        out.extend_from_slice(&0u16.to_le_bytes());  // method 0 = stored
        out.extend_from_slice(&0u16.to_le_bytes());  // mod time
        out.extend_from_slice(&0u16.to_le_bytes());  // mod date
        out.extend_from_slice(&crc32(bytes).to_le_bytes());
        out.extend_from_slice(&(bytes.len() as u32).to_le_bytes());
        out.extend_from_slice(&(bytes.len() as u32).to_le_bytes());
        out.extend_from_slice(&(name.len() as u16).to_le_bytes());
        out.extend_from_slice(&0u16.to_le_bytes());  // extra len
        out.extend_from_slice(name.as_bytes());
        out.extend_from_slice(bytes);

        cd.extend_from_slice(&CDH_SIG.to_le_bytes());
        cd.extend_from_slice(&20u16.to_le_bytes());  // version made by
        cd.extend_from_slice(&20u16.to_le_bytes());  // version needed
        cd.extend_from_slice(&0u16.to_le_bytes());   // flags
        cd.extend_from_slice(&0u16.to_le_bytes());   // method
        cd.extend_from_slice(&0u16.to_le_bytes());   // time
        cd.extend_from_slice(&0u16.to_le_bytes());   // date
        cd.extend_from_slice(&crc32(bytes).to_le_bytes());
        cd.extend_from_slice(&(bytes.len() as u32).to_le_bytes());
        cd.extend_from_slice(&(bytes.len() as u32).to_le_bytes());
        cd.extend_from_slice(&(name.len() as u16).to_le_bytes());
        cd.extend_from_slice(&0u16.to_le_bytes());   // extra
        cd.extend_from_slice(&0u16.to_le_bytes());   // comment
        cd.extend_from_slice(&0u16.to_le_bytes());   // disk
        cd.extend_from_slice(&0u16.to_le_bytes());   // internal attrs
        cd.extend_from_slice(&0u32.to_le_bytes());   // external attrs
        cd.extend_from_slice(&lfh_off.to_le_bytes());
        cd.extend_from_slice(name.as_bytes());
    }
    let cd_off = out.len() as u32;
    let cd_len = cd.len() as u32;
    out.extend_from_slice(&cd);
    out.extend_from_slice(&EOCD_SIG.to_le_bytes());
    out.extend_from_slice(&0u16.to_le_bytes());      // disk
    out.extend_from_slice(&0u16.to_le_bytes());      // cd disk
    out.extend_from_slice(&(entries.len() as u16).to_le_bytes());
    out.extend_from_slice(&(entries.len() as u16).to_le_bytes());
    out.extend_from_slice(&cd_len.to_le_bytes());
    out.extend_from_slice(&cd_off.to_le_bytes());
    out.extend_from_slice(&0u16.to_le_bytes());      // comment len
    out
}

/// In-memory archive over parsed zip bytes: the concrete McprArchive
/// implementation that makes the whole .mcpr container stack run on real
/// zip data (open: parse_zip; save: write_zip_stored via the seam).
pub struct ZipArchiveMem { pub entries: BTreeMap<String, Vec<u8>> }
impl ZipArchiveMem {
    pub fn from_zip_bytes(data: &[u8]) -> Result<Self, String> {
        Ok(Self { entries: parse_zip(data)? })
    }
    pub fn to_zip_bytes(&self) -> Vec<u8> {
        let list: Vec<(String, Vec<u8>)> = self.entries.iter()
            .map(|(k, v)| (k.clone(), v.clone())).collect();
        write_zip_stored(&list, &crate::replayfile::crc32)
    }
}
impl crate::mcpr::McprArchive for ZipArchiveMem {
    fn read_entry(&self, entry: &str) -> Option<Vec<u8>> { self.entries.get(entry).cloned() }
    fn write_entry(&mut self, entry: &str, data: &[u8]) { self.entries.insert(entry.to_string(), data.to_vec()); }
    fn list_entries(&self) -> Vec<String> { self.entries.keys().cloned().collect() }
    fn remove_entry(&mut self, entry: &str) { self.entries.remove(entry); }
}
