//! Replay file container: 1:1 Rust port of ReplayStudio's AbstractReplayFile
//! entry accessors + java.util.zip.CRC32 for the recording.tmcpr.crc32 entry
//! (LGPL-3.0-or-later). This file: Copyright (c) 2026 sparticle,
//! LGPL-3.0-or-later. The zip archive itself stays behind McprArchive
//! (ZipReplayFile's file/cache plumbing is Horizon integration).
use crate::mcpr::*;
use crate::serialize::{esc, JVal};

/// java.util.zip.CRC32 (IEEE 802.3, poly 0xEDB88320), 1:1 behavior.
fn crc_table() -> [u32; 256] {
    let mut t = [0u32; 256];
    let mut i = 0usize;
    while i < 256 {
        let mut c = i as u32;
        let mut k = 0usize;
        while k < 8 {
            c = if c & 1 != 0 { 0xEDB8_8320 ^ (c >> 1) } else { c >> 1 };
            k += 1;
        }
        t[i] = c;
        i += 1;
    }
    t
}
pub fn crc32(data: &[u8]) -> u32 {
    let t = crc_table();
    let mut c: u32 = 0xFFFF_FFFF;
    for &b in data {
        c = t[((c ^ b as u32) & 0xFF) as usize] ^ (c >> 8);
    }
    c ^ 0xFFFF_FFFF
}

/// Upstream TimelineSerialization.FILE_ENTRY.
pub const ENTRY_TIMELINES: &str = "timelines.json";

/// Upstream AbstractReplayFile.THUMB_MAGIC_NUMBERS.
pub const THUMB_MAGIC_NUMBERS: [u8; 7] = [0, 1, 1, 2, 3, 5, 8];

/// Upstream Marker (ReplayStudio replay.Marker fields).
#[derive(Clone, Debug, PartialEq)]
pub struct Marker {
    pub time: i32, pub x: f64, pub y: f64, pub z: f64,
    pub yaw: f32, pub pitch: f32, pub roll: f32, pub name: Option<String>,
}

/// Upstream ModInfo (id, name, version).
#[derive(Clone, Debug, PartialEq)]
pub struct ModInfo { pub id: String, pub name: String, pub version: String }

/// Upstream ReplayAssetEntry (uuid, extension incl. dot, name).
#[derive(Clone, Debug, PartialEq)]
pub struct ReplayAssetEntry { pub uuid: String, pub extension: String, pub name: String }

/// AbstractReplayFile's accessors, 1:1, over the archive seam.
pub struct ReplayFile<'a, A: McprArchive> { pub archive: &'a mut A }

impl<'a, A: McprArchive> ReplayFile<'a, A> {
    /// Upstream getMetaData: null (None) when the entry is absent.
    pub fn get_meta_data(&mut self) -> Option<ReplayMetaData> {
        self.archive.read_entry(ENTRY_META_DATA)
            .and_then(|b| parse_meta_data(&String::from_utf8_lossy(&b)).ok())
    }

    /// Upstream writeMetaData: fileFormat forced "MCPR"; fileFormatVersion +
    /// protocol set when a registry (protocol) is given; generator defaults.
    pub fn write_meta_data(&mut self, md: &mut ReplayMetaData, protocol: Option<i32>, studio_version: &str) {
        md.file_format = "MCPR".into();
        if let Some(p) = protocol {
            md.file_format_version = CURRENT_FILE_FORMAT_VERSION;
            md.protocol = Some(p);
        }
        if md.generator.is_empty() {
            md.generator = format!("ReplayStudio v{}", studio_version);
        }
        let mut json = String::from("{");
        let mut w = |first: &mut bool, k: &str, v: String| {
            if !*first { json.push(','); }
            *first = false;
            json.push_str(&format!("\"{}\":{}", k, v));
        };
        let mut first = true;
        w(&mut first, "singleplayer", md.singleplayer.to_string());
        w(&mut first, "serverName", format!("\"{}\"", esc(&md.server_name)));
        w(&mut first, "customServerName", format!("\"{}\"", esc(&md.custom_server_name)));
        w(&mut first, "duration", md.duration.to_string());
        w(&mut first, "date", md.date.to_string());
        w(&mut first, "mcversion", format!("\"{}\"", esc(&md.mc_version)));
        w(&mut first, "fileFormat", format!("\"{}\"", esc(&md.file_format)));
        w(&mut first, "fileFormatVersion", md.file_format_version.to_string());
        if let Some(p) = md.protocol { w(&mut first, "protocol", p.to_string()); }
        w(&mut first, "generator", format!("\"{}\"", esc(&md.generator)));
        w(&mut first, "selfId", md.self_id.to_string());
        let players: Vec<String> = md.players.iter().map(|p| format!("\"{}\"", esc(p))).collect();
        w(&mut first, "players", format!("[{}]", players.join(",")));
        json.push('}');
        self.archive.write_entry(ENTRY_META_DATA, json.as_bytes());
    }

    /// Upstream getPacketData: the recording bytes (framing in read_packets).
    pub fn get_packet_data(&mut self) -> Option<Vec<u8>> { self.archive.read_entry(ENTRY_RECORDING) }
    /// Upstream writePacketData entry target.
    pub const PACKET_DATA_ENTRY: &'static str = ENTRY_RECORDING;

    /// Upstream getResourcePackIndex: non-numeric keys skipped.
    pub fn get_resource_pack_index(&mut self) -> Option<std::collections::BTreeMap<i32, String>> {
        let b = self.archive.read_entry(ENTRY_RESOURCE_PACK_INDEX)?;
        let v = crate::serialize::json_parse(&String::from_utf8_lossy(&b)).ok()?;
        let mut out = std::collections::BTreeMap::new();
        if let JVal::Obj(pairs) = v {
            for (k, val) in pairs {
                if let (Ok(n), JVal::Str(s)) = (k.parse::<i32>(), val) { out.insert(n, s.clone()); }
            }
        }
        Some(out)
    }

    /// Upstream writeResourcePackIndex.
    pub fn write_resource_pack_index(&mut self, index: &std::collections::BTreeMap<i32, String>) {
        let mut json = String::from("{");
        for (i, (k, v)) in index.iter().enumerate() {
            if i > 0 { json.push(','); }
            json.push_str(&format!("\"{}\":\"{}\"", k, esc(v)));
        }
        json.push('}');
        self.archive.write_entry(ENTRY_RESOURCE_PACK_INDEX, json.as_bytes());
    }

    /// Upstream getThumbBytes: new entry as-is; the old "thumb" entry has the
    /// 7-byte magic numbers stripped when present (pushback semantics).
    pub fn get_thumb_bytes(&mut self) -> Option<Vec<u8>> {
        if let Some(b) = self.archive.read_entry(ENTRY_THUMB) { return Some(b); }
        let b = self.archive.read_entry(ENTRY_THUMB_OLD)?;
        if b.len() >= THUMB_MAGIC_NUMBERS.len() && b[..7] == THUMB_MAGIC_NUMBERS {
            Some(b[7..].to_vec())
        } else {
            Some(b)
        }
    }

    /// Upstream writeThumbBytes.
    pub fn write_thumb_bytes(&mut self, image: &[u8]) { self.archive.write_entry(ENTRY_THUMB, image); }

    /// Upstream getInvisiblePlayers: visibility.json, else the old entry;
    /// shape {"hidden":["<uuid>",..]}.
    pub fn get_invisible_players(&mut self) -> Option<Vec<String>> {
        let b = self.archive.read_entry(ENTRY_VISIBILITY)
            .or_else(|| self.archive.read_entry(ENTRY_VISIBILITY_OLD))?;
        let v = crate::serialize::json_parse(&String::from_utf8_lossy(&b)).ok()?;
        let hidden = match (&v, crate::serialize::json_parse(&String::from_utf8_lossy(&b)).ok()) {
            _ => v,
        };
        if let JVal::Obj(pairs) = hidden {
            for (k, val) in pairs {
                if k == "hidden" {
                    if let JVal::Arr(a) = val {
                        return Some(a.iter().filter_map(|e| if let JVal::Str(s) = e { Some(s.clone()) } else { None }).collect());
                    }
                }
            }
        }
        Some(Vec::new())
    }

    /// Upstream writeInvisiblePlayers.
    pub fn write_invisible_players(&mut self, uuids: &[String]) {
        let inner: Vec<String> = uuids.iter().map(|u| format!("\"{}\"", esc(u))).collect();
        let json = format!("{{\"hidden\":[{}]}}", inner.join(","));
        self.archive.write_entry(ENTRY_VISIBILITY, json.as_bytes());
    }

    /// Upstream getMarkers: array of {realTimestamp, value:{name?, position:{x,y,z,yaw,pitch,roll}}}.
    pub fn get_markers(&mut self) -> Option<Vec<Marker>> {
        let b = self.archive.read_entry(ENTRY_MARKERS)?;
        let v = crate::serialize::json_parse(&String::from_utf8_lossy(&b)).ok()?;
        let arr = match v { JVal::Arr(a) => a, _ => return Some(Vec::new()) };
        let f = |o: &JVal, k: &str| -> f64 {
            if let JVal::Obj(p) = o {
                for (kk, vv) in p { if kk == k { if let JVal::Num(n) = vv { return *n; } } }
            }
            0.0
        };
        let fs = |n: f64| n as f32;
        let mut out = Vec::new();
        for e in &arr {
            let mut name = None;
            let mut time = 0i32;
            let (mut x, mut y, mut z) = (0.0f64, 0.0f64, 0.0f64);
            let (mut yaw, mut pitch, mut roll) = (0.0f32, 0.0f32, 0.0f32);
            if let JVal::Obj(p) = e {
                for (k, val) in p {
                    match k.as_str() {
                        "realTimestamp" => { if let JVal::Num(n) = val { time = *n as i32; } }
                        "value" => {
                            if let JVal::Obj(vp) = val {
                                for (vk, vv) in vp {
                                    match vk.as_str() {
                                        "name" => { if let JVal::Str(s) = vv { name = Some(s.clone()); } }
                                        "position" => {
                                            x = f(vv, "x"); y = f(vv, "y"); z = f(vv, "z");
                                            yaw = fs(f(vv, "yaw")); pitch = fs(f(vv, "pitch")); roll = fs(f(vv, "roll"));
                                        }
                                        _ => {}
                                    }
                                }
                            }
                        }
                        _ => {}
                    }
                }
            }
            out.push(Marker { time, x, y, z, yaw, pitch, roll, name });
        }
        Some(out)
    }

    /// Upstream writeMarkers (Gson skips the null name, matching upstream's
    /// default toJson behavior).
    pub fn write_markers(&mut self, markers: &[Marker]) {
        let mut json = String::from("[");
        for (i, m) in markers.iter().enumerate() {
            if i > 0 { json.push(','); }
            let name = match &m.name { Some(n) => format!("\"{}\"", esc(n)), None => String::new() };
            json.push_str(&format!(
                "{{\"realTimestamp\":{},\"value\":{{{}\"position\":{{\"x\":{},\"y\":{},\"z\":{},\"yaw\":{},\"pitch\":{},\"roll\":{}}}}}}}",
                m.time,
                if m.name.is_some() { format!("\"name\":{},", name) } else { String::new() },
                fmt_f64(m.x), fmt_f64(m.y), fmt_f64(m.z),
                fmt_f32(m.yaw), fmt_f32(m.pitch), fmt_f32(m.roll),
            ));
        }
        json.push(']');
        self.archive.write_entry(ENTRY_MARKERS, json.as_bytes());
    }

    /// Upstream getAssets: entries under asset/, name split on first '_',
    /// extension = from the last '.' (with dot).
    pub fn get_assets(&mut self) -> Vec<ReplayAssetEntry> {
        self.archive.list_entries()
            .into_iter()
            .filter(|k| k.starts_with("asset/") && k.len() > 6)
            .filter_map(|k| {
                let base = &k[6..];
                let delim = base.find('_')?;
                let uuid = base[..delim].to_string();
                let dot = base.rfind('.')?;
                Some(ReplayAssetEntry { uuid, extension: base[dot..].to_string(), name: base[delim + 1..dot].to_string() })
            })
            .collect()
    }

    /// Upstream getAsset(uuid): first matching entry's bytes.
    pub fn get_asset(&mut self, uuid: &str) -> Option<Vec<u8>> {
        let prefix = format!("asset/{}_", uuid);
        let name = self.archive.list_entries().into_iter().find(|k| k.starts_with(&prefix))?;
        self.archive.read_entry(&name)
    }

    /// Upstream writeAsset.
    pub fn write_asset(&mut self, asset: &ReplayAssetEntry, bytes: &[u8]) {
        // Rebuilds the original entry key (uuid_name + extension-with-dot).
        self.archive.write_entry(&format!("asset/{}_{}{}", asset.uuid, asset.name, asset.extension), bytes);
    }

    /// Upstream removeAsset: every entry with that uuid.
    pub fn remove_asset(&mut self, uuid: &str) {
        let prefix = format!("asset/{}_", uuid);
        let names: Vec<String> = self.archive.list_entries().into_iter().filter(|k| k.starts_with(&prefix)).collect();
        for n in names { self.archive.remove_entry(&n); }
    }

    /// Upstream getModInfo: mods.json {"requiredMods":[{modID,modName,modVersion},..]};
    /// empty when the entry is absent.
    pub fn get_mod_info(&mut self) -> Vec<ModInfo> {
        let b = match self.archive.read_entry(ENTRY_MODS) { Some(b) => b, None => return Vec::new() };
        let v = match crate::serialize::json_parse(&String::from_utf8_lossy(&b)) { Ok(v) => v, Err(_) => return Vec::new() };
        let mut out = Vec::new();
        if let JVal::Obj(p) = &v {
            for (k, val) in p {
                if k == "requiredMods" {
                    if let JVal::Arr(a) = val {
                        for e in a {
                            if let JVal::Obj(m) = e {
                                let g = |want: &str| m.iter().find(|(k, _)| k == want)
                                    .and_then(|(_, v)| if let JVal::Str(s) = v { Some(s.clone()) } else { None })
                                    .unwrap_or_default();
                                out.push(ModInfo { id: g("modID"), name: g("modName"), version: g("modVersion") });
                            }
                        }
                    }
                }
            }
        }
        out
    }

    /// Upstream getTimelines -> TimelineSerialization.load(): the
    /// "timelines.json" entry through the phase-3 serializer; empty map when
    /// the entry is absent (upstream returns an empty map too).
    pub fn get_timelines(&mut self) -> Result<std::collections::BTreeMap<String, crate::pathing::Timeline>, String> {
        match self.archive.read_entry(ENTRY_TIMELINES) {
            None => Ok(std::collections::BTreeMap::new()),
            Some(b) => crate::serialize::deserialize(&String::from_utf8_lossy(&b)),
        }
    }

    /// Upstream writeTimelines -> TimelineSerialization.save().
    pub fn write_timelines(&mut self, timelines: &[(String, crate::pathing::Timeline)]) {
        let refs: Vec<(&str, &crate::pathing::Timeline)> = timelines.iter().map(|(n, t)| (n.as_str(), t)).collect();
        let json = crate::serialize::serialize(&refs);
        self.archive.write_entry(ENTRY_TIMELINES, json.as_bytes());
    }

    /// Upstream writeModInfo.
    pub fn write_mod_info(&mut self, mods: &[ModInfo]) {
        let inner: Vec<String> = mods.iter()
            .map(|m| format!("{{\"modID\":\"{}\",\"modName\":\"{}\",\"modVersion\":\"{}\"}}", esc(&m.id), esc(&m.name), esc(&m.version)))
            .collect();
        let json = format!("{{\"requiredMods\":[{}]}}", inner.join(","));
        self.archive.write_entry(ENTRY_MODS, json.as_bytes());
    }
}

/// Java's Double.toString / Float.toString outputs, as far as the format
/// matters for roundtripping (shortest representation).
fn fmt_f64(v: f64) -> String { format!("{}", v) }
fn fmt_f32(v: f32) -> String { format!("{}", v) }
