//! Timeline save format: 1:1 Rust port of ReplayStudio
//! pathing/serialize/TimelineSerialization (the "timelines.json" entry of a
//! replay file, i.e. what backs the .pttr pathing save).
//! Upstream: com.replaymod.replaystudio.pathing.serialize.TimelineSerialization
//! (LGPL-3.0-or-later). This file: Copyright (c) 2026 sparticle, LGPL-3.0-or-later.
//!
//! The upstream registry's interpolator serialization lives in ReplayMod
//! (not ReplayStudio); we mirror it with the same three-type JSON shape the
//! SPTimeline port uses: ["linear"], ["cubic-spline"],
//! [{"type":"catmull-rom-spline","alpha":<a>}], written as a JSON string.
//! Property values: position [x,y,z], rotation [yaw,pitch,roll],
//! timestamp/spectate plain integers.
use crate::pathing::*;
use crate::sptimeline::explicit_interpolation_property;
use std::collections::BTreeMap;

// ---------- minimal JSON reader (upstream uses Gson's JsonReader) ----------

#[derive(Debug, Clone)]
pub enum JVal {
    Null,
    Bool(bool),
    Num(f64),
    Str(String),
    Arr(Vec<JVal>),
    Obj(Vec<(String, JVal)>),
}

struct Parser<'a> { s: &'a [u8], i: usize }

impl<'a> Parser<'a> {
    fn new(s: &'a str) -> Self { Self { s: s.as_bytes(), i: 0 } }
    fn ws(&mut self) { while self.i < self.s.len() && (self.s[self.i] as char).is_ascii_whitespace() { self.i += 1; } }
    fn peek(&mut self) -> u8 { self.ws(); self.s[self.i] }
    fn val(&mut self) -> Result<JVal, String> {
        match self.peek() {
            b'{' => {
                self.i += 1;
                let mut out: Vec<(String, JVal)> = Vec::new();
                loop {
                    if self.peek() == b'}' { self.i += 1; return Ok(JVal::Obj(out)); }
                    let k = match self.val()? { JVal::Str(s) => s, _ => return Err("object key must be a string".into()) };
                    if self.peek() != b':' { return Err("expected ':'".into()); }
                    self.i += 1;
                    let v = self.val()?;
                    out.push((k, v));
                    if self.peek() == b',' { self.i += 1; }
                }
            }
            b'[' => {
                self.i += 1;
                let mut out = Vec::new();
                loop {
                    if self.peek() == b']' { self.i += 1; return Ok(JVal::Arr(out)); }
                    out.push(self.val()?);
                    if self.peek() == b',' { self.i += 1; }
                }
            }
            b'"' => {
                self.i += 1;
                let mut out = String::new();
                while self.s[self.i] != b'"' {
                    if self.s[self.i] == b'\\' {
                        self.i += 1;
                        match self.s[self.i] {
                            b'n' => out.push('\n'),
                            b't' => out.push('\t'),
                            b'r' => out.push('\r'),
                            c => out.push(c as char),
                        }
                    } else {
                        out.push(self.s[self.i] as char);
                    }
                    self.i += 1;
                }
                self.i += 1;
                Ok(JVal::Str(out))
            }
            b't' => { self.i += 4; Ok(JVal::Bool(true)) }
            b'f' => { self.i += 5; Ok(JVal::Bool(false)) }
            b'n' => { self.i += 4; Ok(JVal::Null) }
            _ => {
                let start = self.i;
                while self.i < self.s.len() && matches!(self.s[self.i], b'-' | b'.' | b'0'..=b'9' | b'e' | b'E' | b'+') { self.i += 1; }
                let txt = std::str::from_utf8(&self.s[start..self.i]).map_err(|_| "bad number")?;
                let n: f64 = txt.parse().map_err(|_| format!("bad number: {}", txt))?;
                Ok(JVal::Num(n))
            }
        }
    }
    fn has_next(&mut self) -> bool { self.ws(); self.s.get(self.i).map(|c| *c != b'}' && *c != b']').unwrap_or(false) }
}

fn jstr<'v>(v: &'v JVal, key: &str) -> Option<&'v JVal> {
    match v { JVal::Obj(m) => m.iter().find(|(k, _)| k == key).map(|(_, v)| v), _ => None }
}
fn jnum(v: &JVal) -> Option<f64> { match v { JVal::Num(n) => Some(*n), _ => None } }
fn jstr_v(v: &JVal) -> Option<&str> { match v { JVal::Str(s) => Some(s), _ => None } }

// ---------- writer ----------

pub fn esc(s: &str) -> String {
    let mut out = String::new();
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\t' => out.push_str("\\t"),
            _ => out.push(c),
        }
    }
    out
}

fn prop_key(t: &Timeline, id: PropertyId) -> String {
    let p = &t.properties[id];
    match &p.group { Some(g) => format!("{}:{}", g, p.id), None => p.id.clone() }
}

fn write_prop_value(out: &mut String, v: &PropertyValue) {
    match v {
        PropertyValue::Position(x, y, z) => { out.push_str(&format!("[{},{},{}]", x, y, z)); }
        PropertyValue::Rotation(y, p, r) => { out.push_str(&format!("[{},{},{}]", y, p, r)); }
        PropertyValue::Timestamp(t) => out.push_str(&t.to_string()),
        PropertyValue::Spectator(s) => out.push_str(&s.to_string()),
    }
}

/// Upstream TimelineSerialization.serialize: {"<name>": [ {keyframes: [...],
/// segments: [...], interpolators: [...]}, ... ]} per named timeline.
pub fn serialize(timelines: &[(&str, &Timeline)]) -> String {
    let mut out = String::from("{");
    for (name, t) in timelines {
        if out.len() > 1 { out.push(','); }
        out.push_str(&format!("\"{}\":", esc(name)));
        out.push('[');
        for path in &t.paths {
            out.push_str("{\"keyframes\":[");
            for (time, kf) in &path.keyframes {
                if out.ends_with('}') { out.push(','); }
                out.push_str(&format!("{{\"time\":{},\"properties\":{{", time));
                let mut first = true;
                for (&pid, v) in &kf.values {
                    if !first { out.push(','); }
                    first = false;
                    out.push_str(&format!("\"{}\":", esc(&prop_key(t, pid))));
                    write_prop_value(&mut out, v);
                }
                out.push_str("}}");
            }
            out.push_str("],\"segments\":[");
            let mut map: Vec<InterpId> = Vec::new();
            for (si, seg) in path.segments.iter().enumerate() {
                if si > 0 { out.push(','); }
                match seg.interpolator {
                    None => out.push_str("null"),
                    Some(id) => {
                        if let Some(idx) = map.iter().position(|&m| m == id) {
                            out.push_str(&idx.to_string());
                        } else {
                            out.push_str(&map.len().to_string());
                            map.push(id);
                        }
                    }
                }
            }
            out.push_str("],\"interpolators\":[");
            for (ii, &id) in map.iter().enumerate() {
                if ii > 0 { out.push(','); }
                out.push_str(&format!("{{\"type\":\"{}\",\"properties\":[",
                    esc(&json_string_for_interpolator(t, id))));
                let props: Vec<PropertyId> =
                    t.interps[id].keyframe_properties().iter().copied().collect();
                for (pi, &pid) in props.iter().enumerate() {
                    if pi > 0 { out.push(','); }
                    out.push_str(&format!("\"{}\"", esc(&prop_key(t, pid))));
                }
                out.push_str("]}");
            }
            out.push_str("]}");
        }
        out.push(']');
    }
    out.push('}');
    out
}

/// Interpolator "type" payload: same shape as SPTimeline::serialize_interpolator.
fn json_string_for_interpolator(t: &Timeline, id: InterpId) -> String {
    let (kind, alpha) = t.interps[id].kind();
    match kind {
        "linear" => "[\"linear\"]".into(),
        "cubic-spline" => "[\"cubic-spline\"]".into(),
        "catmull-rom-spline" => format!("[{{\"type\":\"catmull-rom-spline\",\"alpha\":{}}}]", alpha.unwrap_or(0.0)),
        other => format!("[\"{}\"]", other),
    }
}

// ---------- deserialize ----------

/// Upstream PathingRegistry.createTimeline(): SPTimeline's timeline with the
/// four camera properties + the explicit-interpolation marker registered.
pub fn create_timeline() -> Timeline {
    let mut t = Timeline::new();
    // Same registration order as SPTimeline::new so PropertyId indices match.
    t.define_property(timestamp_property());
    t.define_property(camera_position());
    t.define_property(camera_rotation());
    t.define_property(spectator_property());
    t.define_property(explicit_interpolation_property());
    t
}

fn parse_prop_value(t: &Timeline, pid: PropertyId, v: &JVal) -> Result<PropertyValue, String> {
    let id = t.properties[pid].id.as_str();
    match id {
        "position" => match v {
            JVal::Arr(a) if a.len() == 3 => Ok(PropertyValue::Position(
                jnum(&a[0]).ok_or("bad position")?, jnum(&a[1]).ok_or("bad position")?, jnum(&a[2]).ok_or("bad position")?)),
            _ => Err("bad position".into()),
        },
        "rotation" => match v {
            JVal::Arr(a) if a.len() == 3 => Ok(PropertyValue::Rotation(
                jnum(&a[0]).ok_or("bad rotation")? as f32, jnum(&a[1]).ok_or("bad rotation")? as f32, jnum(&a[2]).ok_or("bad rotation")? as f32)),
            _ => Err("bad rotation".into()),
        },
        "timestamp" => Ok(PropertyValue::Timestamp(jnum(v).ok_or("bad timestamp")? as i32)),
        "spectate" => Ok(PropertyValue::Spectator(jnum(v).ok_or("bad spectate")? as i32)),
        // ObjectUtils.NULL marker, written as a plain integer 0.
        "interpolationFixed" => Ok(PropertyValue::Spectator(jnum(v).ok_or("bad marker")? as i32)),
        other => Err(format!("Unknown property: {}", other)),
    }
}

/// Upstream TimelineSerialization.deserialize. Only the current
/// timelines.json format is supported (legacy converters are not ported).
pub fn deserialize(json: &str) -> Result<BTreeMap<String, Timeline>, String> {
    let root = Parser::new(json).val()?;
    let obj = match root { JVal::Obj(m) => m, _ => return Err("expected object".into()) };
    let mut timelines = BTreeMap::new();
    for (name, arr) in obj {
        let mut t = create_timeline();
        let paths = match arr { JVal::Arr(a) => a, _ => return Err("expected path array".into()) };
        for p in &paths {
            let p = p.clone();
            let keyframes = jstr(&p, "keyframes").ok_or("missing keyframes")?;
            let segments = jstr(&p, "segments").ok_or("missing segments")?;
            let interpolators = jstr(&p, "interpolators").ok_or("missing interpolators")?;
            let path = t.create_path();
            let kfs = match keyframes { JVal::Arr(a) => a, _ => return Err("expected keyframes array".into()) };
            for kf in kfs.iter() {
                let time = jnum(jstr(kf, "time").ok_or("missing time")?).ok_or("bad time")? as i64;
                let mut values: BTreeMap<PropertyId, PropertyValue> = BTreeMap::new();
                let props = jstr(kf, "properties").ok_or("missing properties")?;
                if let JVal::Obj(m) = props {
                    for (id, v) in m {
                        let pid = t.get_property_by_string(id).ok_or_else(|| format!("Unknown property: {}", id))?;
                        values.insert(pid, parse_prop_value(&t, pid, v)?);
                    }
                }
                t.path_insert(path, time, values)?;
            }
            let interps = match interpolators { JVal::Arr(a) => a, _ => return Err("expected interpolators array".into()) };
            let mut created: Vec<InterpId> = Vec::new();
            for iv in interps.iter() {
                let ty = jstr_v(jstr(iv, "type").ok_or("Missing interpolator type")?)
                    .ok_or("Missing interpolator type")?;
                let id = {
                    let interp = match ty {
                        "[\"linear\"]" => Box::new(LinearInterpolator::new()) as Box<dyn Interpolator>,
                        "[\"cubic-spline\"]" => Box::new(PolynomialSplineInterpolator::cubic()) as Box<dyn Interpolator>,
                        _ => {
                            // catmull-rom: [{"type":...,"alpha":...}]
                            let p = Parser::new(ty).val()?;
                            let inner = match p { JVal::Arr(a) if !a.is_empty() => a[0].clone(), _ => return Err("bad interpolator".into()) };
                            let kind = jstr(&inner, "type").and_then(|v| jstr_v(v)).ok_or("missing type")?;
                            if kind != "catmull-rom-spline" { return Err(format!("Unknown interpolation type: {}", kind)); }
                            let alpha = jnum(jstr(&inner, "alpha").ok_or("Missing alpha value for catmull-rom-spline.")?)
                                .ok_or("bad alpha")?;
                            Box::new(CatmullRomSplineInterpolator::new(alpha)) as Box<dyn Interpolator>
                        }
                    };
                    t.create_interpolator(interp)
                };
                if let Some(JVal::Arr(props)) = jstr(iv, "properties") {
                    for pv in props {
                        let name = jstr_v(pv).ok_or("bad property name")?;
                        let pid = t.get_property_by_string(name)
                            .ok_or_else(|| format!("Timeline does not know property '{}'", name))?;
                        t.interps[id].register_property(pid);
                    }
                }
                created.push(id);
            }
            let segs = match segments { JVal::Arr(a) => a, _ => return Err("expected segments array".into()) };
            for (i, sv) in segs.into_iter().enumerate() {
                if let JVal::Num(n) = sv {
                    let idx = *n as usize;
                    let id = *created.get(idx).ok_or("bad interpolator index")?;
                    t.interps[id].add_segment(t.paths[path].segments[i].id);
                    t.paths[path].segments[i].interpolator = Some(id);
                }
            }
        }
        timelines.insert(name, t);
    }
    Ok(timelines)
}

/// Public entry to the JSON reader (used by the replay-file layer).
pub fn json_parse(json: &str) -> Result<JVal, String> { Parser::new(json).val() }
