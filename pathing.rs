//! Pathing core: 1:1 Rust port of ReplayStudio's pathing package.
//! Upstream: ReplayStudio (github.com/ReplayMod/ReplayStudio),
//! com/replaymod/replaystudio/pathing/{path,impl,property,interpolation},
//! LGPL-3.0-or-later. This derived port: copyright (c) 2026 sparticle,
//! licensed LGPL-3.0-or-later.
//!
//! Shape notes: upstream Path back-references its Timeline; Rust ownership
//! makes Timeline the single owner and path handles are indices. Keyframes
//! live in a BTreeMap keyed by time (upstream TreeMap<Long,Keyframe>), so
//! keyframe identity == keyframe time; segments reference start/end times.
//! Interpolators are shared objects (upstream identity) kept in Timeline
//! .interps and referenced by id from segments.

use std::collections::{BTreeMap, HashMap, HashSet};

pub type PropertyId = usize;
pub type InterpId = usize;
pub type PartKey = (PropertyId, usize);

#[derive(Clone, Debug, PartialEq)]
pub enum PropertyValue {
    Position(f64, f64, f64),
    Rotation(f32, f32, f32),
    Timestamp(i32),
    Spectator(i32),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PartKind { PosX, PosY, PosZ, Yaw, Pitch, Roll, Timestamp, Spectator }

#[derive(Clone, Debug)]
pub struct PartDef {
    pub kind: PartKind,
    pub interpolatable: bool,
    pub upper_bound: f64,
}

impl PartDef {
    pub fn new(kind: PartKind, interpolatable: bool, upper_bound: f64) -> Self {
        Self { kind, interpolatable, upper_bound }
    }

    /// Upstream PropertyPart.toDouble. The interface javadoc claims an
    /// UnsupportedOperationException for non-interpolatable parts, but the
    /// concrete parts (ForInteger, ForDoubleTriple, ForFloatTriple) never
    /// check; only get_value's isInterpolatable guard does. Mirrored here.
    pub fn to_double(&self, v: &PropertyValue) -> Result<f64, String> {
        match (self.kind, v) {
            (PartKind::PosX, PropertyValue::Position(x, _, _)) => Ok(*x),
            (PartKind::PosY, PropertyValue::Position(_, y, _)) => Ok(*y),
            (PartKind::PosZ, PropertyValue::Position(_, _, z)) => Ok(*z),
            (PartKind::Yaw, PropertyValue::Rotation(y, _, _)) => Ok(*y as f64),
            (PartKind::Pitch, PropertyValue::Rotation(_, p, _)) => Ok(*p as f64),
            (PartKind::Roll, PropertyValue::Rotation(_, _, r)) => Ok(*r as f64),
            (PartKind::Timestamp, PropertyValue::Timestamp(t)) => Ok(*t as f64),
            (PartKind::Spectator, PropertyValue::Spectator(s)) => Ok(*s as f64),
            _ => Err("part kind does not match value".into()),
        }
    }

    /// Upstream PropertyPart.fromDouble: replace this part's component.
    pub fn from_double(&self, v: &PropertyValue, d: f64) -> Result<PropertyValue, String> {
        match (self.kind, v) {
            (PartKind::PosX, PropertyValue::Position(_, y, z)) => Ok(PropertyValue::Position(d, *y, *z)),
            (PartKind::PosY, PropertyValue::Position(x, _, z)) => Ok(PropertyValue::Position(*x, d, *z)),
            (PartKind::PosZ, PropertyValue::Position(x, y, _)) => Ok(PropertyValue::Position(*x, *y, d)),
            (PartKind::Yaw, PropertyValue::Rotation(_, p, r)) => Ok(PropertyValue::Rotation(d as f32, *p, *r)),
            (PartKind::Pitch, PropertyValue::Rotation(y, _, r)) => Ok(PropertyValue::Rotation(*y, d as f32, *r)),
            (PartKind::Roll, PropertyValue::Rotation(y, p, _)) => Ok(PropertyValue::Rotation(*y, *p, d as f32)),
            (PartKind::Timestamp, PropertyValue::Timestamp(_)) => Ok(PropertyValue::Timestamp(round_i32(d))),
            (PartKind::Spectator, PropertyValue::Spectator(_)) => Ok(PropertyValue::Spectator(round_i32(d))),
            _ => Err("part kind does not match value".into()),
        }
    }
}

/// Upstream PropertyParts.ForInteger.fromDouble: (int) Math.round(d).
fn round_i32(d: f64) -> i32 { d.round() as i32 }

/// Upstream Property: id, group, localized name (upstream resolves via I18n;
/// the key is stored and the GUI layer resolves), initial value, parts.
#[derive(Clone, Debug)]
pub struct Property {
    pub id: String,
    pub group: Option<String>,
    pub localized_name: String,
    pub initial: PropertyValue,
    pub parts: Vec<PartDef>,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Keyframe {
    pub values: BTreeMap<PropertyId, PropertyValue>,
}

#[derive(Clone, Copy, Debug)]
pub struct Segment {
    pub id: u32,
    pub start: i64,
    pub end: i64,
    pub interpolator: Option<InterpId>,
}

#[derive(Default)]
pub struct Path {
    pub keyframes: BTreeMap<i64, Keyframe>,
    pub segments: Vec<Segment>,
    pub active: bool,
}

impl Path {
    pub fn new() -> Self { Self { keyframes: BTreeMap::new(), segments: Vec::new(), active: true } }
}

#[derive(Clone, Copy, Debug)]
pub struct InterpParams { pub value: f64, pub velocity: f64, pub acceleration: f64 }

/// Upstream com.replaymod.replaystudio.pathing.change.Change.
pub trait Change {
    fn apply(&mut self, t: &mut Timeline) -> Result<(), String>;
    fn undo(&mut self, t: &mut Timeline) -> Result<(), String>;
}

/// Upstream com.replaymod.replaystudio.pathing.interpolation.Interpolator.
/// Object-safe trait; shared instances live in Timeline.interps.
pub trait Interpolator {
    /// Upstream interpolator class identity + constructor argument, for
    /// SPTimeline's class checks and serialization: ("linear", None),
    /// ("cubic-spline", None), ("catmull-rom-spline", Some(alpha)).
    fn kind(&self) -> (&'static str, Option<f64>) { ("unknown", None) }
    fn register_property(&mut self, prop: PropertyId);
    fn unregister_property(&mut self, prop: PropertyId);
    fn keyframe_properties(&self) -> &HashSet<PropertyId>;
    fn add_segment(&mut self, seg: u32);
    fn remove_segment(&mut self, seg: u32);
    fn segments(&self) -> &[u32];
    fn is_dirty(&self) -> bool;
    /// Upstream bake(Map): sorts this interpolator's segments by start time,
    /// checks continuity, then bakeInterpolation. `segs` is the (id, start,
    /// end) list for the segment ids this interpolator holds.
    fn bake(
        &mut self,
        segs: &[(u32, i64, i64)],
        keyframes: &BTreeMap<i64, Keyframe>,
        properties: &[Property],
        params: BTreeMap<PartKey, InterpParams>,
    ) -> Result<BTreeMap<PartKey, InterpParams>, String>;
    /// Upstream getValue: keyframes is the owning path's map, properties the
    /// timeline registry.
    fn get_value(
        &self,
        prop: PropertyId,
        time: i64,
        keyframes: &BTreeMap<i64, Keyframe>,
        properties: &[Property],
    ) -> Result<Option<PropertyValue>, String>;
}

/// State shared by upstream AbstractInterpolator subclasses.
#[derive(Default)]
pub struct AbstractInterpolatorState {
    pub segments: Vec<u32>,
    pub dirty: bool,
    pub properties: HashSet<PropertyId>,
}

/// Upstream AbstractInterpolator.bake (sort + continuity check).
pub fn abstract_bake(
    st: &mut AbstractInterpolatorState,
    segs: &[(u32, i64, i64)],
) -> Result<Vec<(u32, i64, i64)>, String> {
    if st.segments.is_empty() { return Err("No segments have been added yet.".into()); }
    let by_id: HashMap<u32, (i64, i64)> =
        segs.iter().map(|&(id, s, e)| (id, (s, e))).collect();
    // Sort this interpolator's own segment list by start keyframe time.
    st.segments.sort_by_key(|&id| by_id.get(&id).map(|p| p.0).unwrap_or(i64::MAX));
    let sorted: Vec<(u32, i64, i64)> = st
        .segments
        .iter()
        .map(|&id| {
            let (s, e) = by_id.get(&id).copied().unwrap_or((i64::MAX, i64::MAX));
            (id, s, e)
        })
        .collect();
    // Check for continuity: last.end keyframe == next.start keyframe.
    for w in sorted.windows(2) {
        if w[0].2 != w[1].1 {
            return Err("Segments are not continuous.".into());
        }
    }
    Ok(sorted)
}

/// Upstream LinearInterpolator: linear interpolation with cyclic wrapping on
/// bounded parts. framesToProperty keeps keyframe times in bake (insertion)
/// order, mirroring upstream's LinkedHashSet.
#[derive(Default)]
pub struct LinearInterpolator {
    pub base: AbstractInterpolatorState,
    frames_to_property: BTreeMap<PropertyId, Vec<i64>>,
}

impl LinearInterpolator {
    pub fn new() -> Self { Self::default() }

    fn add_to_map(&mut self, prop: PropertyId, time: i64) {
        let v = self.frames_to_property.entry(prop).or_default();
        if !v.contains(&time) { v.push(time); }
    }
}

impl Interpolator for LinearInterpolator {
    fn kind(&self) -> (&'static str, Option<f64>) { ("linear", None) }
    fn register_property(&mut self, prop: PropertyId) {
        if self.base.properties.insert(prop) { self.base.dirty = true; }
    }
    fn unregister_property(&mut self, prop: PropertyId) {
        if self.base.properties.remove(&prop) { self.base.dirty = true; }
    }
    fn keyframe_properties(&self) -> &HashSet<PropertyId> { &self.base.properties }
    fn add_segment(&mut self, seg: u32) { self.base.segments.push(seg); self.base.dirty = true; }
    fn remove_segment(&mut self, seg: u32) {
        self.base.segments.retain(|&s| s != seg);
        self.base.dirty = true;
    }
    fn segments(&self) -> &[u32] { &self.base.segments }
    fn is_dirty(&self) -> bool { self.base.dirty }

    fn bake(
        &mut self,
        segs: &[(u32, i64, i64)],
        keyframes: &BTreeMap<i64, Keyframe>,
        _properties: &[Property],
        params: BTreeMap<PartKey, InterpParams>,
    ) -> Result<BTreeMap<PartKey, InterpParams>, String> {
        let sorted = abstract_bake(&mut self.base, segs)?;

        self.frames_to_property.clear();
        let props: Vec<PropertyId> = self.base.properties.iter().copied().collect();
        for &(_, start, end) in &sorted {
            for &prop in &props {
                if keyframes.get(&start).map_or(false, |k| k.values.contains_key(&prop)) {
                    self.add_to_map(prop, start);
                }
                if keyframes.get(&end).map_or(false, |k| k.values.contains_key(&prop)) {
                    self.add_to_map(prop, end);
                }
            }
        }

        // lastParameters from the final segment's end keyframe.
        let last = sorted.last().map(|&(_, _, e)| e).unwrap();
        let mut out = params;
        for &prop in &props {
            if let Some(v) = keyframes.get(&last).and_then(|k| k.values.get(&prop)) {
                for (i, part) in _properties[prop].parts.iter().enumerate() {
                    out.insert(
                        (prop, i),
                        InterpParams { value: part.to_double(v)?, velocity: 1.0, acceleration: 0.0 },
                    );
                }
            }
        }
        Ok(out)
    }

    fn get_value(
        &self,
        prop: PropertyId,
        time: i64,
        keyframes: &BTreeMap<i64, Keyframe>,
        properties: &[Property],
    ) -> Result<Option<PropertyValue>, String> {
        let Some(times) = self.frames_to_property.get(&prop) else { return Ok(None) };
        let mut before: Option<i64> = None;
        let mut after: Option<i64> = None;
        for &t in times {
            if t == time {
                return Ok(keyframes.get(&t).and_then(|k| k.values.get(&prop)).cloned());
            } else if t < time {
                before = Some(t);
            } else {
                after = Some(t);
                break;
            }
        }
        let (Some(bt), Some(at)) = (before, after) else { return Ok(None) };
        let value_before = keyframes[&bt].values[&prop].clone();
        let value_after = keyframes[&at].values[&prop].clone();
        let fraction = (time - bt) as f64 / (at - bt) as f64;

        let mut interpolated = value_before.clone();
        for part in &properties[prop].parts {
            if part.interpolatable {
                let mut b = part.to_double(&value_before)?;
                let mut a = part.to_double(&value_after)?;
                let bound = part.upper_bound;
                if !bound.is_nan() {
                    b = fmod_floor(b, bound);
                    a = fmod_floor(a, bound);
                    if (a - b).abs() > bound / 2.0 {
                        if b < bound / 2.0 { a -= bound; } else { a += bound; }
                    }
                }
                let mut v = (a - b) * fraction + b;
                if !bound.is_nan() { v = fmod_floor(v, bound); }
                interpolated = part.from_double(&interpolated, v)?;
            }
        }
        Ok(Some(interpolated))
    }
}

/// Upstream LinearInterpolator.mod: floor-based modulo.
fn fmod_floor(val: f64, m: f64) -> f64 {
    let off = (val / m).floor();
    val - off * m
}

/// Upstream TimelineImpl + PathImpl, with Timeline as the single owner.
pub struct Timeline {
    pub paths: Vec<Path>,
    /// Property registry: PropertyId = index; each define_property appends,
    /// matching upstream object identity (two registrations of the same id
    /// string are distinct objects unless replaced in prop_by_string).
    pub properties: Vec<Property>,
    /// Upstream TimelineImpl.properties map, keyed "group:id" or "id".
    prop_by_string: HashMap<String, PropertyId>,
    pub interps: Vec<Box<dyn Interpolator>>,
    next_seg_id: u32,
    undo: Vec<Box<dyn Change>>,
    redo: Vec<Box<dyn Change>>,
}

impl Default for Timeline { fn default() -> Self { Self::new() } }

impl Timeline {
    pub fn new() -> Self {
        Self {
            paths: Vec::new(),
            properties: Vec::new(),
            prop_by_string: HashMap::new(),
            interps: Vec::new(),
            next_seg_id: 0,
            undo: Vec::new(),
            redo: Vec::new(),
        }
    }

    /// Upstream Timeline.registerProperty (which also creates the property).
    pub fn define_property(&mut self, p: Property) -> PropertyId {
        let id = self.properties.len();
        let key = match &p.group {
            Some(g) => format!("{}:{}", g, p.id),
            None => p.id.clone(),
        };
        self.prop_by_string.insert(key, id);
        self.properties.push(p);
        id
    }

    /// Upstream Timeline.getProperty(id).
    pub fn get_property_by_string(&self, id: &str) -> Option<PropertyId> {
        self.prop_by_string.get(id).copied()
    }

    /// Upstream Timeline.createPath.
    pub fn create_path(&mut self) -> usize {
        self.paths.push(Path::new());
        self.paths.len() - 1
    }

    pub fn create_interpolator(&mut self, i: Box<dyn Interpolator>) -> InterpId {
        self.interps.push(i);
        self.interps.len() - 1
    }

    fn insert_segment_at(&mut self, p: usize, idx: usize, start: i64, end: i64, interp: Option<InterpId>) {
        let id = self.next_seg_id;
        self.next_seg_id += 1;
        self.paths[p].segments.insert(idx, Segment { id, start, end, interpolator: None });
        if let Some(ip) = interp {
            self.interps[ip].add_segment(id);
            self.paths[p].segments[idx].interpolator = Some(ip);
        }
    }

    fn push_segment(&mut self, p: usize, start: i64, end: i64, interp: Option<InterpId>) {
        let id = self.next_seg_id;
        self.next_seg_id += 1;
        self.paths[p].segments.push(Segment { id, start, end, interpolator: None });
        if let Some(ip) = interp {
            self.interps[ip].add_segment(id);
            let n = self.paths[p].segments.len();
            self.paths[p].segments[n - 1].interpolator = Some(ip);
        }
    }

    /// Upstream PathSegmentImpl.setInterpolator: detach old, attach new.
    pub fn seg_set_interpolator(&mut self, p: usize, seg_idx: usize, interp: Option<InterpId>) {
        let seg = self.paths[p].segments[seg_idx];
        if let Some(old) = seg.interpolator {
            self.interps[old].remove_segment(seg.id);
        }
        self.paths[p].segments[seg_idx].interpolator = interp;
        if let Some(new) = interp {
            self.interps[new].add_segment(seg.id);
        }
    }

    /// Upstream Path.insert(Keyframe) / insert(long); `values` are the new
    /// keyframe's property values (empty for insert(long)).
    pub fn path_insert(
        &mut self,
        p: usize,
        time: i64,
        values: BTreeMap<PropertyId, PropertyValue>,
    ) -> Result<(), String> {
        if self.paths[p].keyframes.contains_key(&time) {
            return Err(format!("A keyframe at {} already exists.", time));
        }
        self.paths[p].keyframes.insert(time, Keyframe { values });

        if self.paths[p].segments.is_empty() {
            if self.paths[p].keyframes.len() >= 2 {
                let mut it = self.paths[p].keyframes.keys().copied();
                let t1 = it.next().unwrap();
                let t2 = it.next().unwrap();
                self.push_segment(p, t1, t2, None);
            }
            return Ok(());
        }

        let first = self.paths[p].segments[0];
        if time < first.start {
            let interp = first.interpolator;
            self.insert_segment_at(p, 0, time, first.start, interp);
            return Ok(());
        }

        let mut i = 0usize;
        loop {
            let seg = self.paths[p].segments[i];
            if seg.start <= time && seg.end >= time {
                let interp = seg.interpolator;
                self.paths[p].segments.remove(i);
                if let Some(ip) = interp {
                    self.interps[ip].remove_segment(seg.id);
                }
                self.insert_segment_at(p, i, seg.start, time, interp);
                self.insert_segment_at(p, i + 1, time, seg.end, interp);
                return Ok(());
            }
            if i + 1 < self.paths[p].segments.len() {
                i += 1;
            } else {
                let interp = seg.interpolator;
                self.push_segment(p, seg.end, time, interp);
                return Ok(());
            }
        }
    }

    /// Upstream Path.getKeyframe(time).
    pub fn path_get_keyframe(&self, p: usize, time: i64) -> Option<&Keyframe> {
        self.paths[p].keyframes.get(&time)
    }

    /// Upstream Keyframe.setValue/get/removeProperty helpers.
    pub fn kf_set_value(&mut self, p: usize, time: i64, prop: PropertyId, v: PropertyValue) -> Result<(), String> {
        let kf = self.paths[p].keyframes.get_mut(&time).ok_or("no such keyframe")?;
        kf.values.insert(prop, v);
        Ok(())
    }
    pub fn kf_remove_property(&mut self, p: usize, time: i64, prop: PropertyId) -> Result<(), String> {
        let kf = self.paths[p].keyframes.get_mut(&time).ok_or("no such keyframe")?;
        kf.values.remove(&prop);
        Ok(())
    }
    pub fn kf_properties(&self, p: usize, time: i64) -> Vec<PropertyId> {
        self.paths[p].keyframes.get(&time).map(|k| k.values.keys().copied().collect()).unwrap_or_default()
    }

    /// Upstream Path.remove(keyframe, useFirstInterpolator).
    pub fn path_remove(&mut self, p: usize, time: i64, use_first: bool) -> Result<(), String> {
        if !self.paths[p].keyframes.contains_key(&time) {
            return Err(format!("The keyframe at {} is not part of this path.", time));
        }
        self.paths[p].keyframes.remove(&time);

        if self.paths[p].segments.len() < 2 {
            for s in &mut self.paths[p].segments {
                if let Some(ip) = s.interpolator {
                    self.interps[ip].remove_segment(s.id);
                    s.interpolator = None;
                }
            }
            self.paths[p].segments.clear();
            return Ok(());
        }

        let mut i = 0usize;
        while i < self.paths[p].segments.len() {
            let seg = self.paths[p].segments[i];
            if seg.end == time {
                self.paths[p].segments.remove(i);
                if let Some(ip) = seg.interpolator {
                    self.interps[ip].remove_segment(seg.id);
                }
                if i < self.paths[p].segments.len() {
                    let next2 = self.paths[p].segments[i];
                    let chosen = if use_first { seg.interpolator } else { next2.interpolator };
                    self.paths[p].segments.remove(i);
                    if let Some(ip) = next2.interpolator {
                        self.interps[ip].remove_segment(next2.id);
                    }
                    self.insert_segment_at(p, i, seg.start, next2.end, chosen);
                }
                return Ok(());
            }
            if seg.start == time {
                if let Some(ip) = seg.interpolator {
                    self.interps[ip].remove_segment(seg.id);
                }
                self.paths[p].segments.remove(i);
                return Ok(());
            }
            i += 1;
        }
        Err("No segment for keyframe found!".into())
    }

    /// Upstream Path.update/updateAll: both call update(false) upstream; the
    /// force flag is kept for phase 2's direct bakes.
    pub fn path_update(&mut self, p: usize, force: bool) -> Result<(), String> {
        let mut params: BTreeMap<PartKey, InterpParams> = BTreeMap::new();
        let mut current: Option<InterpId> = None;
        let segs_snapshot: Vec<(u32, i64, i64)> =
            self.paths[p].segments.iter().map(|s| (s.id, s.start, s.end)).collect();
        for seg in &self.paths[p].segments {
            if seg.interpolator != current {
                current = seg.interpolator;
                if let Some(id) = seg.interpolator {
                    if force || self.interps[id].is_dirty() {
                        let ids = self.interps[id].segments().to_vec();
                        let ctx: Vec<(u32, i64, i64)> = ids
                            .iter()
                            .filter_map(|sid| segs_snapshot.iter().copied().find(|(x, _, _)| x == sid))
                            .collect();
                        let kfs = self.paths[p].keyframes.clone();
                        let taken = std::mem::take(&mut params);
                        params = self.interps[id].bake(&ctx, &kfs, &self.properties, taken)?;
                    }
                }
            }
        }
        Ok(())
    }

    /// Upstream Path.getValue(property, time).
    pub fn path_get_value(&self, p: usize, prop: PropertyId, time: i64) -> Result<Option<PropertyValue>, String> {
        if let Some(seg) = self.paths[p].segments.iter().find(|s| s.start <= time && s.end >= time) {
            if let Some(id) = seg.interpolator {
                if self.interps[id].keyframe_properties().contains(&prop) {
                    return self.interps[id].get_value(prop, time, &self.paths[p].keyframes, &self.properties);
                }
            }
        }
        Ok(None)
    }

    /// Upstream Timeline.getValue(property, time): first active path wins.
    pub fn get_value(&self, prop: PropertyId, time: i64) -> Result<Option<PropertyValue>, String> {
        for p in 0..self.paths.len() {
            if self.paths[p].active {
                if let Some(v) = self.path_get_value(p, prop, time)? {
                    return Ok(Some(v));
                }
            }
        }
        Ok(None)
    }

    /// Upstream TimelineImpl.applyToGame: applies every registered property
    /// via the game glue (upstream Property.applyToGame(replayHandler)).
    pub fn apply_to_game(&mut self, time: i64, applier: &mut dyn FnMut(PropertyId, &PropertyValue)) -> Result<(), String> {
        for pid in 0..self.properties.len() {
            if let Some(v) = self.get_value(pid, time)? {
                applier(pid, &v);
            }
        }
        Ok(())
    }

    /// Upstream Timeline.applyChange: apply, push, clear redo.
    pub fn apply_change(&mut self, mut c: Box<dyn Change>) -> Result<(), String> {
        c.apply(self)?;
        self.undo.push(c);
        self.redo.clear();
        Ok(())
    }
    /// Upstream Timeline.pushChange.
    pub fn push_change(&mut self, c: Box<dyn Change>) {
        self.undo.push(c);
        self.redo.clear();
    }
    /// Upstream undoLastChange.
    pub fn undo_last_change(&mut self) -> Result<(), String> {
        let mut c = self.undo.pop().ok_or("undo stack empty")?;
        c.undo(self)?;
        self.redo.push(c);
        Ok(())
    }
    /// Upstream redoLastChange.
    pub fn redo_last_change(&mut self) -> Result<(), String> {
        let mut c = self.redo.pop().ok_or("redo stack empty")?;
        c.apply(self)?;
        self.undo.push(c);
        Ok(())
    }
    /// Upstream peekUndoStack / peekRedoStack (presence only).
    pub fn has_undo(&self) -> bool { !self.undo.is_empty() }
    pub fn has_redo(&self) -> bool { !self.redo.is_empty() }
}

/// Upstream replaymod pathing/properties: the four concrete properties the
/// timeline layer uses. Localization keys are stored verbatim (upstream
/// resolves them via I18n at display time).
pub fn camera_position() -> Property {
    Property {
        id: "position".into(),
        group: Some("camera".into()),
        localized_name: "replaymod.gui.position".into(),
        initial: PropertyValue::Position(0.0, 0.0, 0.0),
        parts: vec![
            PartDef::new(PartKind::PosX, true, f64::NAN),
            PartDef::new(PartKind::PosY, true, f64::NAN),
            PartDef::new(PartKind::PosZ, true, f64::NAN),
        ],
    }
}

pub fn camera_rotation() -> Property {
    Property {
        id: "rotation".into(),
        group: Some("camera".into()),
        localized_name: "replaymod.gui.rotation".into(),
        initial: PropertyValue::Rotation(0.0, 0.0, 0.0),
        parts: vec![
            PartDef::new(PartKind::Yaw, true, 360.0),
            PartDef::new(PartKind::Pitch, true, 360.0),
            PartDef::new(PartKind::Roll, true, 360.0),
        ],
    }
}

pub fn timestamp_property() -> Property {
    Property {
        id: "timestamp".into(),
        group: None,
        localized_name: "replaymod.gui.editkeyframe.timestamp".into(),
        initial: PropertyValue::Timestamp(0),
        parts: vec![PartDef::new(PartKind::Timestamp, true, f64::NAN)],
    }
}

pub fn spectator_property() -> Property {
    Property {
        id: "spectate".into(),
        group: None,
        localized_name: "replaymod.gui.playeroverview.spectate".into(),
        initial: PropertyValue::Spectator(-1),
        parts: vec![PartDef::new(PartKind::Spectator, false, f64::NAN)],
    }
}

/// Upstream PolynomialSplineInterpolator.Polynomial. Coefficients ascending:
/// coefficients[0] multiplies the HIGHEST power (eval is Horner from the
/// front, matching upstream).
#[derive(Clone, Debug)]
pub struct Polynomial { pub coefficients: Vec<f64> }

impl Polynomial {
    pub fn eval(&self, at: f64) -> f64 {
        let mut val = 0.0;
        for &c in &self.coefficients { val = val * at + c; }
        val
    }
    pub fn derivative(&self) -> Polynomial {
        if self.coefficients.is_empty() { return self.clone(); }
        let n = self.coefficients.len();
        let mut derived = vec![0.0; n - 1];
        for i in 0..n - 1 {
            derived[i] = self.coefficients[i] * (n - 1 - i) as f64;
        }
        Polynomial { coefficients: derived }
    }
}

/// Upstream PolynomialSplineInterpolator.Polynomials.
#[derive(Clone, Debug)]
pub struct Polynomials { pub y_offset: f64, pub polynomials: Vec<Polynomial> }

impl Polynomials {
    pub fn eval(&self, time_ms: f64, index: usize) -> f64 {
        self.polynomials[index].eval(time_ms / 1000.0) + self.y_offset
    }
}

/// Upstream solveMatrix (Gauss elimination, forward then back substitution),
/// ported line for line including its exact scaling behavior.
pub fn solve_matrix(matrix: &mut [Vec<f64>]) {
    let n = matrix.len();
    for i in 0..n {
        if matrix[i][i] == 0.0 {
            for j in i + 1..n {
                if matrix[j][i] != 0.0 {
                    matrix.swap(i, j);
                    break;
                }
            }
        }
        let factor = matrix[i][i];
        if factor != 1.0 {
            matrix[i][i] = 1.0;
            for j in i + 1..matrix[i].len() {
                matrix[i][j] /= factor;
            }
        }
        for j in i + 1..n {
            let factor = matrix[j][i];
            if factor != 0.0 {
                matrix[j][i] = 0.0;
                for k in i + 1..matrix[j].len() {
                    matrix[j][k] -= matrix[i][k] * factor;
                }
            }
        }
    }
    for i in (0..n).rev() {
        for j in (0..i).rev() {
            if matrix[j][i] != 0.0 {
                let k = matrix[j].len() - 1;
                matrix[j][k] -= matrix[j][i] / matrix[i][i] * matrix[i][k];
                matrix[j][i] = 0.0;
            }
        }
    }
}

/// Which concrete spline fills the matrix. Upstream: abstract
/// PolynomialSplineInterpolator with CubicSplineInterpolator the concrete
/// subclass (degree 3).
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum SplineKind { Cubic }

/// Upstream PolynomialSplineInterpolator (degree from kind) + its Cubic
/// subclass. framesToProperty keeps keyframe times in bake order.
pub struct PolynomialSplineInterpolator {
    pub base: AbstractInterpolatorState,
    pub kind: SplineKind,
    frames_to_property: BTreeMap<PropertyId, Vec<i64>>,
    polynomials: BTreeMap<PartKey, Polynomials>,
}

impl PolynomialSplineInterpolator {
    pub fn cubic() -> Self {
        Self { base: Default::default(), kind: SplineKind::Cubic,
               frames_to_property: BTreeMap::new(), polynomials: BTreeMap::new() }
    }

    fn add_to_map(&mut self, prop: PropertyId, time: i64) {
        let v = self.frames_to_property.entry(prop).or_default();
        if !v.contains(&time) { v.push(time); }
    }

    /// Upstream calcPolynomials (shared by all splines of this family).
    fn calc_polynomials(
        &mut self,
        part: &PartDef,
        mut xs: Vec<f64>,
        mut ys: Vec<f64>,
        params: Option<InterpParams>,
    ) -> Polynomials {
        let degree = match self.kind { SplineKind::Cubic => 3 };
        let unknowns = degree + 1;
        let num = xs.len() - 1;
        if num == 0 {
            return Polynomials { y_offset: 0.0, polynomials: vec![Polynomial { coefficients: vec![ys[0]] }] };
        }
        for x in &mut xs { *x /= 1000.0; }

        let y_offset: f64;
        if part.upper_bound.is_nan() {
            let total: f64 = ys.iter().sum();
            y_offset = total / ys.len() as f64;
            for y in &mut ys { *y -= y_offset; }
            let params = params.map(|p| InterpParams { value: p.value - y_offset, ..p });
            return self.finish(degree, unknowns, num, xs, ys, params, y_offset);
        } else {
            let bound = part.upper_bound;
            let half = bound / 2.0;
            let first_value = params.map(|p| p.value).unwrap_or(ys[0]);
            let mut offset = (first_value / bound).floor() as i64;
            let mut last_value = fmod_floor(first_value, bound);
            for i in 1..ys.len() {
                let value = fmod_floor(ys[i], bound);
                if (value - last_value).abs() > half {
                    if last_value < half { offset -= 1; } else { offset += 1; }
                }
                ys[i] = value + (offset as f64) * bound;
                last_value = value;
            }
            y_offset = 0.0;
        }
        self.finish(degree, unknowns, num, xs, ys, params, 0.0)
    }

    fn finish(
        &mut self,
        degree: usize,
        unknowns: usize,
        num: usize,
        xs: Vec<f64>,
        ys: Vec<f64>,
        params: Option<InterpParams>,
        y_off: f64,
    ) -> Polynomials {
        let n = num * unknowns;
        let mut matrix = vec![vec![0.0; n + 1]; n];
        match self.kind {
            SplineKind::Cubic => fill_matrix_cubic(&mut matrix, &xs, &ys, num, unknowns, params),
        }
        solve_matrix(&mut matrix);
        let mut polynomials = Vec::with_capacity(num);
        for i in 0..num {
            let mut coefficients = vec![0.0; degree + 1];
            for j in 0..=degree {
                coefficients[j] = matrix[i * unknowns + j][n];
            }
            polynomials.push(Polynomial { coefficients });
        }
        Polynomials { y_offset: y_off, polynomials }
    }

    /// Upstream prepareProperty: per interpolatable part, fit the spline and
    /// record endpoint value/velocity/acceleration into the params map.
    fn prepare_property(
        &mut self,
        prop: PropertyId,
        times: &[i64],
        keyframes: &BTreeMap<i64, Keyframe>,
        properties: &[Property],
        params: &mut BTreeMap<PartKey, InterpParams>,
    ) -> Result<(), String> {
        for pi in 0..properties[prop].parts.len() {
            let part = &properties[prop].parts[pi];
            if !part.interpolatable { continue; }
            let mut xs = Vec::with_capacity(times.len());
            let mut ys = Vec::with_capacity(times.len());
            for &t in times {
                xs.push(t as f64);
                let v = &keyframes[&t].values[&prop];
                ys.push(part.to_double(v)?);
            }
            let incoming = params.get(&(prop, pi)).copied();
            let polys = self.calc_polynomials(part, xs, ys, incoming);
            let last_time = times[times.len() - 1] as f64;
            let last_poly = polys.polynomials.last().unwrap().clone();
            let last_value = last_poly.eval(last_time) + polys.y_offset;
            let last_velocity = last_poly.derivative().eval(last_time);
            let last_acceleration = last_poly.derivative().derivative().eval(last_time);
            params.insert((prop, pi), InterpParams { value: last_value, velocity: last_velocity, acceleration: last_acceleration });
            self.polynomials.insert((prop, pi), polys);
        }
        Ok(())
    }
}

/// Upstream CubicSplineInterpolator.fillMatrix, ported line for line
/// (matrix columns: per cubic a=x^3, b=x^2, c=x, d=1; last column = y).
fn fill_matrix_cubic(
    matrix: &mut [Vec<f64>],
    xs: &[f64],
    ys: &[f64],
    num: usize,
    _unknowns: usize,
    params: Option<InterpParams>,
) {
    let mut row = 0usize;
    let n = num * 4;
    if let Some(p) = params {
        // Apply previous values
        let mut ysv = ys.to_vec();
        ysv[0] = p.value;
        let x = xs[0];
        matrix[row][0] = 3.0 * x * x;
        matrix[row][1] = 2.0 * x;
        matrix[row][2] = 1.0;
        matrix[row][n] = p.velocity;
        row += 1;
        matrix[row][0] = 6.0 * x;
        matrix[row][1] = 2.0;
        matrix[row][n] = p.acceleration;
        row += 1;
        // ys[0] was overwritten locally; matrix equations below use ysv.
        for i in 0..num {
            let x = xs[i];
            matrix[row][i * 4] = x * x * x;
            matrix[row][i * 4 + 1] = x * x;
            matrix[row][i * 4 + 2] = x;
            matrix[row][i * 4 + 3] = 1.0;
            matrix[row][n] = ysv[i];
            row += 1;
            let x = xs[i + 1];
            matrix[row][i * 4] = x * x * x;
            matrix[row][i * 4 + 1] = x * x;
            matrix[row][i * 4 + 2] = x;
            matrix[row][i * 4 + 3] = 1.0;
            matrix[row][n] = ysv[i + 1];
            row += 1;
            if i < num - 1 {
                let x = xs[i + 1];
                matrix[row][i * 4 + 4] = 3.0 * x * x;
                matrix[row][i * 4] = -matrix[row][i * 4 + 4];
                matrix[row][i * 4 + 5] = 2.0 * x;
                matrix[row][i * 4 + 1] = -matrix[row][i * 4 + 5];
                matrix[row][i * 4 + 6] = 1.0;
                matrix[row][i * 4 + 2] = -matrix[row][i * 4 + 6];
                row += 1;
                matrix[row][i * 4 + 4] = 6.0 * x;
                matrix[row][i * 4] = -matrix[row][i * 4 + 4];
                matrix[row][i * 4 + 5] = 2.0;
                matrix[row][i * 4 + 1] = -matrix[row][i * 4 + 5];
                row += 1;
            }
        }
        return;
    }
    // Set second derivative at the first and the last knot to 0
    matrix[row][0] = 6.0 * xs[0];
    matrix[row][1] = 2.0;
    row += 1;
    matrix[row][(num - 1) * 4] = 6.0 * xs[xs.len() - 1];
    matrix[row][(num - 1) * 4 + 1] = 2.0;
    row += 1;
    for i in 0..num {
        let x = xs[i];
        matrix[row][i * 4] = x * x * x;
        matrix[row][i * 4 + 1] = x * x;
        matrix[row][i * 4 + 2] = x;
        matrix[row][i * 4 + 3] = 1.0;
        matrix[row][n] = ys[i];
        row += 1;
        let x = xs[i + 1];
        matrix[row][i * 4] = x * x * x;
        matrix[row][i * 4 + 1] = x * x;
        matrix[row][i * 4 + 2] = x;
        matrix[row][i * 4 + 3] = 1.0;
        matrix[row][n] = ys[i + 1];
        row += 1;
        if i < num - 1 {
            let x = xs[i + 1];
            matrix[row][i * 4 + 4] = 3.0 * x * x;
            matrix[row][i * 4] = -matrix[row][i * 4 + 4];
            matrix[row][i * 4 + 5] = 2.0 * x;
            matrix[row][i * 4 + 1] = -matrix[row][i * 4 + 5];
            matrix[row][i * 4 + 6] = 1.0;
            matrix[row][i * 4 + 2] = -matrix[row][i * 4 + 6];
            row += 1;
            matrix[row][i * 4 + 4] = 6.0 * x;
            matrix[row][i * 4] = -matrix[row][i * 4 + 4];
            matrix[row][i * 4 + 5] = 2.0;
            matrix[row][i * 4 + 1] = -matrix[row][i * 4 + 5];
            row += 1;
        }
    }
}

impl Interpolator for PolynomialSplineInterpolator {
    fn kind(&self) -> (&'static str, Option<f64>) { ("cubic-spline", None) }
    fn register_property(&mut self, prop: PropertyId) {
        if self.base.properties.insert(prop) { self.base.dirty = true; }
    }
    fn unregister_property(&mut self, prop: PropertyId) {
        if self.base.properties.remove(&prop) { self.base.dirty = true; }
    }
    fn keyframe_properties(&self) -> &HashSet<PropertyId> { &self.base.properties }
    fn add_segment(&mut self, seg: u32) { self.base.segments.push(seg); self.base.dirty = true; }
    fn remove_segment(&mut self, seg: u32) {
        self.base.segments.retain(|&s| s != seg);
        self.base.dirty = true;
    }
    fn segments(&self) -> &[u32] { &self.base.segments }
    fn is_dirty(&self) -> bool { self.base.dirty }

    fn bake(
        &mut self,
        segs: &[(u32, i64, i64)],
        keyframes: &BTreeMap<i64, Keyframe>,
        properties: &[Property],
        params: BTreeMap<PartKey, InterpParams>,
    ) -> Result<BTreeMap<PartKey, InterpParams>, String> {
        let sorted = abstract_bake(&mut self.base, segs)?;
        self.frames_to_property.clear();
        let props: Vec<PropertyId> = self.base.properties.iter().copied().collect();
        for &(_, start, end) in &sorted {
            for &prop in &props {
                if keyframes.get(&start).map_or(false, |k| k.values.contains_key(&prop)) {
                    self.add_to_map(prop, start);
                }
                if keyframes.get(&end).map_or(false, |k| k.values.contains_key(&prop)) {
                    self.add_to_map(prop, end);
                }
            }
        }
        // Upstream copies the incoming map, then mutates it per property.
        let mut out = params;
        for (prop, times) in self.frames_to_property.clone() {
            self.prepare_property(prop, &times, keyframes, properties, &mut out)?;
        }
        Ok(out)
    }

    fn get_value(
        &self,
        prop: PropertyId,
        time: i64,
        keyframes: &BTreeMap<i64, Keyframe>,
        properties: &[Property],
    ) -> Result<Option<PropertyValue>, String> {
        let Some(times) = self.frames_to_property.get(&prop) else { return Ok(None) };
        // Upstream scan: index ends at the count of frames strictly before
        // the first frame after `time` (the segment index to evaluate).
        let mut before: Option<i64> = None;
        let mut after: Option<i64> = None;
        let mut index: usize = 0;
        for &t in times {
            if t == time {
                return Ok(keyframes.get(&t).and_then(|k| k.values.get(&prop)).cloned());
            } else if t < time {
                before = Some(t);
            } else {
                after = Some(t);
                index -= 1; // matches upstream index-- before break
                break;
            }
            index += 1;
        }
        let (Some(_bt), Some(_at)) = (before, after) else { return Ok(None) };
        let value_before = keyframes[&before.unwrap()].values[&prop].clone();
        let mut interpolated = value_before.clone();
        for (pi, part) in properties[prop].parts.iter().enumerate() {
            if part.interpolatable {
                let mut value = self.polynomials[&(prop, pi)].eval(time as f64, index);
                if !part.upper_bound.is_nan() {
                    value = fmod_floor(value, part.upper_bound);
                }
                interpolated = part.from_double(&interpolated, value)?;
            }
        }
        Ok(Some(interpolated))
    }
}

/// Upstream CatmullRomSplineInterpolator: Catmull-Rom splines parameterized
/// 0..1 per segment, with configurable alpha for tangent scaling.
pub struct CatmullRomSplineInterpolator {
    pub base: AbstractInterpolatorState,
    pub alpha: f64,
    frames_to_property: BTreeMap<PropertyId, Vec<i64>>,
    cubic_polynomials: BTreeMap<PartKey, Vec<Polynomial>>,
}

impl CatmullRomSplineInterpolator {
    pub fn new(alpha: f64) -> Self {
        Self { base: Default::default(), alpha,
               frames_to_property: BTreeMap::new(), cubic_polynomials: BTreeMap::new() }
    }

    fn add_to_map(&mut self, prop: PropertyId, time: i64) {
        let v = self.frames_to_property.entry(prop).or_default();
        if !v.contains(&time) { v.push(time); }
    }

    /// Upstream calcPolynomials (steve.hollasch.net catmull-rom reference).
    fn calc_polynomials(
        &mut self,
        keyframes: &BTreeMap<i64, Keyframe>,
        properties: &[Property],
    ) -> Result<(), String> {
        let props: Vec<PropertyId> = self.base.properties.iter().copied().collect();
        for &prop in &props {
            for pi in 0..properties[prop].parts.len() {
                let part = &properties[prop].parts[pi];
                if !part.interpolatable { continue; }
                let mut values: Vec<f64> = Vec::new();
                if part.upper_bound.is_nan() {
                    for &t in &self.frames_to_property[&prop] {
                        let v = &keyframes[&t].values[&prop];
                        values.push(part.to_double(v)?);
                    }
                } else {
                    let bound = part.upper_bound;
                    let half = bound / 2.0;
                    let mut last_value: Option<f64> = None;
                    let mut offset: i64 = 0;
                    for &t in &self.frames_to_property[&prop] {
                        let v = &keyframes[&t].values[&prop];
                        let value = fmod_floor(part.to_double(v)?, bound);
                        if last_value.is_none() {
                            last_value = Some(value);
                            offset = (value / bound).floor() as i64;
                        }
                        let lv = last_value.unwrap();
                        if (value - lv).abs() > half {
                            if lv < half { offset -= 1; } else { offset += 1; }
                        }
                        values.push(value + (offset as f64) * bound);
                        last_value = Some(value);
                    }
                }
                let n = self.frames_to_property[&prop].len();
                let mut polys = Vec::with_capacity(values.len().saturating_sub(1));
                for i in 0..values.len().saturating_sub(1) {
                    let p1 = values[i];
                    let p2 = values[i + 1];
                    let p0 = if i > 0 { values[i - 1] } else { p1 };
                    let p3 = if i < n - 2 { values[i + 2] } else { p2 };
                    let t0 = self.alpha * (p2 - p0);
                    let t1 = self.alpha * (p3 - p1);
                    polys.push(Polynomial { coefficients: vec![
                        2.0 * p1 - 2.0 * p2 + t0 + t1,
                        -3.0 * p1 + 3.0 * p2 - 2.0 * t0 - t1,
                        t0,
                        p1,
                    ] });
                }
                self.cubic_polynomials.insert((prop, pi), polys);
            }
        }
        Ok(())
    }
}

impl Interpolator for CatmullRomSplineInterpolator {
    fn kind(&self) -> (&'static str, Option<f64>) { ("catmull-rom-spline", Some(self.alpha)) }
    fn register_property(&mut self, prop: PropertyId) {
        if self.base.properties.insert(prop) { self.base.dirty = true; }
    }
    fn unregister_property(&mut self, prop: PropertyId) {
        if self.base.properties.remove(&prop) { self.base.dirty = true; }
    }
    fn keyframe_properties(&self) -> &HashSet<PropertyId> { &self.base.properties }
    fn add_segment(&mut self, seg: u32) { self.base.segments.push(seg); self.base.dirty = true; }
    fn remove_segment(&mut self, seg: u32) {
        self.base.segments.retain(|&s| s != seg);
        self.base.dirty = true;
    }
    fn segments(&self) -> &[u32] { &self.base.segments }
    fn is_dirty(&self) -> bool { self.base.dirty }

    fn bake(
        &mut self,
        segs: &[(u32, i64, i64)],
        keyframes: &BTreeMap<i64, Keyframe>,
        properties: &[Property],
        _params: BTreeMap<PartKey, InterpParams>,
    ) -> Result<BTreeMap<PartKey, InterpParams>, String> {
        let sorted = abstract_bake(&mut self.base, segs)?;
        self.frames_to_property.clear();
        let props: Vec<PropertyId> = self.base.properties.iter().copied().collect();
        for &(_, start, end) in &sorted {
            for &prop in &props {
                if keyframes.get(&start).map_or(false, |k| k.values.contains_key(&prop)) {
                    self.add_to_map(prop, start);
                }
                if keyframes.get(&end).map_or(false, |k| k.values.contains_key(&prop)) {
                    self.add_to_map(prop, end);
                }
            }
        }
        self.calc_polynomials(keyframes, properties)?;
        // Upstream builds lastParameters from every registered property's
        // parts; a missing polynomial (non-interpolatable part or < 2 frames)
        // NPEs upstream, mirrored as Err here.
        let mut out: BTreeMap<PartKey, InterpParams> = BTreeMap::new();
        for &prop in &props {
            for pi in 0..properties[prop].parts.len() {
                let polys = self.cubic_polynomials.get(&(prop, pi))
                    .ok_or_else(|| format!("catmullrom: no polynomials for part ({}, {})", prop, pi))?;
                let last = polys.last().ok_or("catmullrom: empty polynomial set")?.clone();
                let value = last.eval(1.0);
                let velocity = last.derivative().eval(1.0);
                let acceleration = last.derivative().derivative().eval(1.0);
                out.insert((prop, pi), InterpParams { value, velocity, acceleration });
            }
        }
        Ok(out)
    }

    fn get_value(
        &self,
        prop: PropertyId,
        time: i64,
        keyframes: &BTreeMap<i64, Keyframe>,
        properties: &[Property],
    ) -> Result<Option<PropertyValue>, String> {
        let Some(times) = self.frames_to_property.get(&prop) else { return Ok(None) };
        let mut time_before: Option<i64> = None;
        let mut time_after: Option<i64> = None;
        let mut value_before: Option<PropertyValue> = None;
        let mut index: usize = 0;
        for (i, &t) in times.iter().enumerate() {
            if t == time {
                return Ok(keyframes.get(&t).and_then(|k| k.values.get(&prop)).cloned());
            } else if t < time {
                index = i;
                time_before = Some(t);
                value_before = Some(keyframes[&t].values[&prop].clone());
            } else {
                time_after = Some(t);
                break;
            }
        }
        let (Some(tb), Some(ta)) = (time_before, time_after) else { return Ok(None) };
        let fraction = (time - tb) as f64 / (ta - tb) as f64;
        let mut interpolated = value_before.unwrap();
        for (pi, part) in properties[prop].parts.iter().enumerate() {
            if part.interpolatable {
                let polys = self.cubic_polynomials.get(&(prop, pi))
                    .ok_or_else(|| format!("catmullrom: no polynomials for part ({}, {})", prop, pi))?;
                // No upper-bound mod here: upstream omits it (values were
                // unwrapped at bake time).
                interpolated = part.from_double(&interpolated, polys[index].eval(fraction))?;
            }
        }
        Ok(Some(interpolated))
    }
}
