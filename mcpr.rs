//! Replay file layer: 1:1 Rust port of ReplayStudio's replay-file constants
//! and the .tmcpr packet stream framing (ReplayInputStream.readPacket /
//! fillBuffer framing half, ReplayOutputStream.doWrite framing half,
//! ReplayMetaData's JSON schema; LGPL-3.0-or-later). This file: Copyright
//! (c) 2026 sparticle, LGPL-3.0-or-later.
//!
//! The zip container itself (ZipReplayFile's archive open/write + CRC cache)
//! is a Horizon-integration hook behind McprArchive below; the MC packet
//! decode half of fillBuffer (varint packet id, registry state transitions,
//! the JoinGame spectator rewrite, ViaVersion armor shims) is also a hook:
//! for the framing port, packets are opaque byte blobs, matching how the
//! sender port consumes them.
use crate::serialize::{JVal, json_parse}; // reuse the hand-rolled JSON reader

// Upstream AbstractReplayFile entry-name constants.
pub const ENTRY_META_DATA: &str = "metaData.json";
pub const ENTRY_RECORDING: &str = "recording.tmcpr";
pub const ENTRY_RECORDING_HASH: &str = "recording.tmcpr.crc32";
pub const ENTRY_RESOURCE_PACK_INDEX: &str = "resourcepack/index.json";
pub const ENTRY_THUMB_OLD: &str = "thumb";
pub const ENTRY_THUMB: &str = "thumb.jpg";
pub const ENTRY_VISIBILITY_OLD: &str = "visibility";
pub const ENTRY_VISIBILITY: &str = "visibility.json";
pub const ENTRY_MARKERS: &str = "markers.json";
pub const ENTRY_MODS: &str = "mods.json";
/// Upstream ENTRY_ASSET / ENTRY_RESOURCE_PACK are printf formats
/// ("asset/%s_%s.%s", "resourcepack/%s.zip"); port as helpers.
pub fn entry_asset(name: &str, typ: &str, ext: &str) -> String { format!("asset/{}_{}.{}", name, typ, ext) }
pub fn entry_resource_pack(name: &str) -> String { format!("resourcepack/{}.zip", name) }

/// Upstream ReplayMetaData.CURRENT_FILE_FORMAT_VERSION.
pub const CURRENT_FILE_FORMAT_VERSION: i32 = 14;

/// Upstream Utils.readInt: big-endian, any -1 byte means stream end.
pub fn read_int(buf: &[u8], pos: &mut usize) -> i32 {
    if *pos + 4 > buf.len() { *pos = buf.len(); return -1; }
    *pos += 4;
    let b = [buf[*pos - 4], buf[*pos - 3], buf[*pos - 2], buf[*pos - 1]];
    // Upstream checks (b0|b1|b2|b3) < 0 where the b* are in.read() results:
    // -1 only means end of stream, high-value bytes are legitimate.
    i32::from_be_bytes(b)
}

/// Upstream Utils.writeInt: big-endian.
pub fn write_int(out: &mut Vec<u8>, v: i32) { out.extend_from_slice(&v.to_be_bytes()); }

/// Upstream PacketData: opaque recorded packet bytes with replay timestamp.
/// (Upstream carries a decoded Packet; the decode is the Horizon hook.)
#[derive(Clone, Debug)]
pub struct McprPacketData { pub timestamp: i64, pub bytes: Vec<u8> }

/// Upstream ReplayInputStream.readPacket over the raw stream, framing half:
/// int timestamp, int length, then bytes; -1s end the stream; empty
/// segments are skipped. The MC decode/registry half stays a hook.
pub fn read_packets(stream: &[u8]) -> Vec<McprPacketData> {
    let mut out = Vec::new();
    let mut pos = 0usize;
    loop {
        let next = read_int(stream, &mut pos);
        let length = read_int(stream, &mut pos);
        if next == -1 || length == -1 { break; } // upstream: end of stream
        if length < 0 { break; } // upstream EOFException on malformed length
        if length == 0 { continue; } // upstream: skip empty segments
        if pos + length as usize > stream.len() { break; } // upstream EOFException
        out.push(McprPacketData { timestamp: next as i64, bytes: stream[pos..pos + length as usize].to_vec() });
        pos += length as usize;
    }
    out
}

/// Upstream ReplayOutputStream.doWrite: int time, int length, bytes.
pub fn write_packet(out: &mut Vec<u8>, time: i64, bytes: &[u8]) {
    write_int(out, time as i32);
    write_int(out, bytes.len() as i32);
    out.extend_from_slice(bytes);
}

/// Upstream ReplayMetaData, JSON schema fields verbatim.
#[derive(Clone, Debug)]
pub struct ReplayMetaData {
    pub singleplayer: bool,
    pub server_name: String,
    pub custom_server_name: String,
    pub duration: i32,
    pub date: i64,
    pub mc_version: String,
    pub file_format: String,
    pub file_format_version: i32,
    pub protocol: Option<i32>,
    pub generator: String,
    pub self_id: i32, // upstream default -1
    pub players: Vec<String>,
}

impl Default for ReplayMetaData {
    fn default() -> Self {
        Self { singleplayer: false, server_name: String::new(), custom_server_name: String::new(),
            duration: 0, date: 0, mc_version: String::new(),
            file_format: String::new(), file_format_version: 0,
            protocol: None, generator: String::new(), self_id: -1, players: Vec::new() }
    }
}

fn jstr(v: Option<&JVal>, d: &str) -> String { match v { Some(JVal::Str(s)) => s.clone(), _ => d.into() } }
fn jget<'v>(v: &'v JVal, k: &str) -> Option<&'v JVal> {
    match v { JVal::Obj(m) => m.iter().find(|(kk, _)| kk == k).map(|(_, vv)| vv), _ => None }
}
fn jnum(v: Option<&JVal>) -> Option<f64> { match v { Some(JVal::Num(n)) => Some(*n), _ => None } }
fn jbool(v: Option<&JVal>) -> bool { matches!(v, Some(JVal::Bool(b)) if *b) }

/// Upstream Gson deserialization of metaData.json.
pub fn parse_meta_data(json: &str) -> Result<ReplayMetaData, String> {
    let v = json_parse(json)?;
    let g = |k: &str| jget(&v, k);
    Ok(ReplayMetaData {
        singleplayer: jbool(g("singleplayer")),
        server_name: jstr(g("serverName"), ""),
        custom_server_name: jstr(g("customServerName"), ""),
        duration: jnum(g("duration")).unwrap_or(0.0) as i32,
        date: jnum(g("date")).unwrap_or(0.0) as i64,
        mc_version: jstr(g("mcversion"), ""),
        file_format: jstr(g("fileFormat"), ""),
        file_format_version: jnum(g("fileFormatVersion")).unwrap_or(0.0) as i32,
        protocol: jnum(g("protocol")).map(|n| n as i32),
        generator: jstr(g("generator"), ""),
        self_id: jnum(g("selfId")).unwrap_or(-1.0) as i32,
        players: match g("players") { Some(JVal::Arr(a)) => a.iter().filter_map(|p| if let JVal::Str(s) = p { Some(s.clone()) } else { None }).collect(), _ => Vec::new() },
    })
}

/// Upstream ZipReplayFile's archive container, abstracted: Horizon provides
/// the zip read/write; this trait is the seam.
pub trait McprArchive {
    fn read_entry(&self, name: &str) -> Option<Vec<u8>>;
    fn list_entries(&self) -> Vec<String>;
    fn write_entry(&mut self, name: &str, bytes: &[u8]);
    fn remove_entry(&mut self, name: &str);
}
