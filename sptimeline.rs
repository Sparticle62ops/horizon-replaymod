//! SPTimeline: 1:1 Rust port of ReplayMod simplepathing (SPTimeline,
//! InterpolatorType, ExplicitInterpolationProperty) plus the Change classes
//! it drives (changes.rs). Upstream ReplayMod, LGPL-3.0-or-later; this
//! derived port: copyright (c) 2026 sparticle, LGPL-3.0-or-later.
//!
//! Shape notes: upstream SPTimeline holds a Timeline plus two Path
//! references (TIME=0, POSITION=1); Rust uses path indices. Upstream
//! Preconditions/Validate -> Result. Interpolator serialization here is a
//! minimal hand-rolled JSON matching upstream's JsonWriter output for the
//! three known types (equality + clone use; full .pttr save format is a
//! later phase). EntityPositionTracker is a trait hook here; the tracker
//! itself is ported with the replay sender phase.

use crate::pathing::*;
use crate::changes::*;
use std::collections::BTreeMap;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum SPPath { Time, Position }

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum InterpolatorType { Default, CatmullRom, Cubic, Linear }

impl InterpolatorType {
    /// Upstream newInstance (DEFAULT throws).
    pub fn new_instance(self) -> Result<Box<dyn Interpolator>, String> {
        match self {
            InterpolatorType::Default => Err("DEFAULT has no interpolator".into()),
            InterpolatorType::CatmullRom => Ok(Box::new(CatmullRomSplineInterpolator::new(0.5))),
            InterpolatorType::Cubic => Ok(Box::new(PolynomialSplineInterpolator::cubic())),
            InterpolatorType::Linear => Ok(Box::new(LinearInterpolator::new())),
        }
    }
    /// Upstream getInterpolatorClass, as the kind string.
    pub fn class_kind(self) -> Option<&'static str> {
        match self {
            InterpolatorType::Default => None,
            InterpolatorType::CatmullRom => Some("catmull-rom-spline"),
            InterpolatorType::Cubic => Some("cubic-spline"),
            InterpolatorType::Linear => Some("linear"),
        }
    }
    /// Upstream fromClass (DEFAULT on miss).
    pub fn from_class(kind: &str) -> Self {
        match kind {
            "catmull-rom-spline" => InterpolatorType::CatmullRom,
            "cubic-spline" => InterpolatorType::Cubic,
            "linear" => InterpolatorType::Linear,
            _ => InterpolatorType::Default,
        }
    }
    /// Upstream localizationKey.
    pub fn localization_key(self) -> &'static str {
        match self {
            InterpolatorType::Default => "default",
            InterpolatorType::CatmullRom => "catmullrom",
            InterpolatorType::Cubic => "cubic",
            InterpolatorType::Linear => "linear",
        }
    }
    /// Upstream fromString: matches the i18n name; falls back to CATMULL_ROM.
    pub fn from_string(s: &str) -> Self {
        let full = |k: &str| format!("replaymod.gui.editkeyframe.interpolator.{}.name", k);
        if s == full("default") { InterpolatorType::Default }
        else if s == full("catmullrom") { InterpolatorType::CatmullRom }
        else if s == full("cubic") { InterpolatorType::Cubic }
        else if s == full("linear") { InterpolatorType::Linear }
        else { InterpolatorType::CatmullRom }
    }
}

/// Upstream replaystudio.util.Location (exact float equality, as upstream).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Location { pub x: f64, pub y: f64, pub z: f64, pub yaw: f32, pub pitch: f32 }

impl Location {
    pub fn new(x: f64, y: f64, z: f64, yaw: f32, pitch: f32) -> Self {
        Self { x, y, z, yaw, pitch }
    }
}

/// Upstream EntityPositionTracker's read the timeline layer needs; the
/// concrete tracker (replaystudio.util.EntityPositionTracker) is ported with
/// the replay sender phase.
pub trait EntityPositionTracker {
    fn get_entity_position_at_timestamp(&self, entity_id: i32, timestamp: i32) -> Option<Location>;
}

/// Upstream simplepathing.properties.ExplicitInterpolationProperty: a dummy
/// property whose presence marks a segment's interpolator as explicitly set.
/// The value is an opaque null marker (upstream ObjectUtils.NULL); only
/// presence is ever read.
pub fn explicit_interpolation_property() -> Property {
    Property {
        id: "interpolationFixed".into(),
        group: None,
        localized_name: "<internal>".into(),
        // ObjectUtils.NULL marker; never read or interpolated (no parts).
        initial: PropertyValue::Spectator(0),
        parts: vec![],
    }
}

pub struct SPTimeline {
    pub timeline: Timeline,
    pub time_path: usize,
    pub position_path: usize,
    pub entity_tracker: Option<Box<dyn EntityPositionTracker>>,
    pub default_interpolator_type: Option<InterpolatorType>,
    pub camera_position_id: PropertyId,
    pub camera_rotation_id: PropertyId,
    pub timestamp_id: PropertyId,
    pub spectator_id: PropertyId,
    pub explicit_id: PropertyId,
}

impl SPTimeline {
    /// Upstream SPTimeline() -> createInitialTimeline.
    pub fn new() -> Self {
        let mut timeline = Timeline::new();
        let timestamp_id = timeline.define_property(timestamp_property());
        let camera_position_id = timeline.define_property(camera_position());
        let camera_rotation_id = timeline.define_property(camera_rotation());
        let spectator_id = timeline.define_property(spectator_property());
        let explicit_id = timeline.define_property(explicit_interpolation_property());
        timeline.create_path();
        timeline.create_path();
        Self {
            timeline,
            time_path: 0,
            position_path: 1,
            entity_tracker: None,
            default_interpolator_type: None,
            camera_position_id,
            camera_rotation_id,
            timestamp_id,
            spectator_id,
            explicit_id,
        }
    }

    /// Upstream SPTimeline(Timeline): requires the registry properties and
    /// at least the two SPPath paths to exist (upstream indexes by ordinal).
    pub fn from_timeline(mut timeline: Timeline) -> Result<Self, String> {
        if timeline.paths.len() < 2 { return Err("timeline needs the TIME and POSITION paths".into()); }
        let get = |t: &Timeline, id: &str| t.get_property_by_string(id).ok_or_else(|| format!("property {} not registered", id));
        Ok(Self {
            camera_position_id: get(&timeline, "camera:position")?,
            camera_rotation_id: get(&timeline, "camera:rotation")?,
            timestamp_id: get(&timeline, "timestamp")?,
            spectator_id: get(&timeline, "spectate")?,
            explicit_id: get(&timeline, "interpolationFixed")?,
            timeline,
            time_path: 0,
            position_path: 1,
            entity_tracker: None,
            default_interpolator_type: None,
        })
    }

    pub fn get_path(&self, p: SPPath) -> usize {
        match p { SPPath::Time => self.time_path, SPPath::Position => self.position_path }
    }

    pub fn get_keyframe(&self, p: SPPath, time: i64) -> Option<&Keyframe> {
        self.timeline.path_get_keyframe(self.get_path(p), time)
    }

    /// Upstream setEntityTracker (checkState: only settable once).
    pub fn set_entity_tracker(&mut self, tracker: Box<dyn EntityPositionTracker>) -> Result<(), String> {
        if self.entity_tracker.is_some() { return Err("Entity tracker already set".into()); }
        self.entity_tracker = Some(tracker);
        Ok(())
    }

    /// Upstream setDefaultInterpolatorType.
    pub fn set_default_interpolator_type(&mut self, ty: InterpolatorType) -> Result<(), String> {
        if ty == InterpolatorType::Default { return Err("Must not be DEFAULT".into()); }
        let prev = self.default_interpolator_type;
        self.default_interpolator_type = Some(ty);
        if let Some(p) = prev {
            if p != ty {
                let change = self.update_interpolators()?;
                self.timeline.push_change(change);
            }
        }
        Ok(())
    }

    /// Upstream setDefaultInterpolator(Interpolator): takes an already
    /// created interpolator id; validates it against the default type.
    pub fn set_default_interpolator(&mut self, interp: InterpId) -> Result<Box<dyn Change>, String> {
        let ty = self.default_interpolator_type.ok_or("Default interpolator type not set.")?;
        let kind = self.timeline.interps[interp].kind().0;
        if Some(kind) != ty.class_kind() {
            return Err("interpolator is not of the default interpolator type".into());
        }
        self.register_position_interpolator_properties(interp);

        let mut changes: Vec<Box<dyn Change>> = Vec::new();
        let p = self.position_path;
        for i in 0..self.timeline.paths[p].segments.len() {
            let start = self.timeline.paths[p].segments[i].start;
            let explicit = self.timeline.paths[p].keyframes.get(&start)
                .map(|k| k.values.contains_key(&self.explicit_id)).unwrap_or(false);
            if explicit { continue; }
            if self.is_spectator_segment(p, i) { continue; }
            changes.push(Box::new(SetInterpolator::new(p, i, interp)));
        }
        let mut combined = CombinedChange::create(changes);
        combined.apply(&mut self.timeline)?;
        let upd = self.update_interpolators()?;
        Ok(Box::new(CombinedChange::create_from_applied(vec![Box::new(combined), upd])))
    }

    pub fn is_time_keyframe(&self, time: i64) -> bool {
        self.timeline.path_get_keyframe(self.time_path, time).is_some()
    }
    pub fn is_position_keyframe(&self, time: i64) -> bool {
        self.timeline.path_get_keyframe(self.position_path, time).is_some()
    }
    pub fn is_spectator_keyframe(&self, time: i64) -> bool {
        self.timeline.path_get_keyframe(self.position_path, time)
            .map(|k| k.values.contains_key(&self.spectator_id)).unwrap_or(false)
    }

    /// Upstream addPositionKeyframe.
    pub fn add_position_keyframe(&mut self, time: i64, pos: (f64, f64, f64), rot: (f32, f32, f32), spectated: i32) -> Result<(), String> {
        if self.timeline.paths[self.position_path].keyframes.contains_key(&time) {
            return Err("Keyframe already exists".into());
        }
        let mut parts: Vec<Box<dyn Change>> = Vec::new();
        let mut add = AddKeyframe::new(self.position_path, time);
        add.apply(&mut self.timeline)?;
        parts.push(Box::new(add));
        let index = kf_time_to_index(&self.timeline.paths[self.position_path], time);
        let update = UpdateKeyframeProperties::create(self.position_path, index)
            .set_value(self.camera_position_id, PropertyValue::Position(pos.0, pos.1, pos.2))
            .set_value(self.camera_rotation_id, PropertyValue::Rotation(rot.0, rot.1, rot.2));
        let update = if spectated != -1 { update.set_value(self.spectator_id, PropertyValue::Spectator(spectated)) } else { update };
        let mut update = update.done();
        update.apply(&mut self.timeline)?;
        parts.push(Box::new(update));

        // If this new keyframe formed the first segment of the path, create
        // an initial interpolator of default type.
        if self.timeline.paths[self.position_path].segments.len() == 1 {
            let interp = self.create_default_interpolator()?;
            let mut set = SetInterpolator::new(self.position_path, 0, interp);
            set.apply(&mut self.timeline)?;
            parts.push(Box::new(set));
        }

        // Update interpolators for spectator keyframes (upstream: overkill,
        // but far simpler than covering every case).
        let upd = self.update_interpolators()?;
        parts.push(upd);
        let mut spec = self.update_spectator_positions()?;
        spec.apply(&mut self.timeline)?;
        parts.push(spec);
        self.timeline.push_change(Box::new(CombinedChange::create_from_applied(parts)));
        Ok(())
    }

    /// Upstream updatePositionKeyframe.
    pub fn update_position_keyframe(&mut self, time: i64, pos: (f64, f64, f64), rot: (f32, f32, f32)) -> Result<Box<dyn Change>, String> {
        if !self.timeline.paths[self.position_path].keyframes.contains_key(&time) {
            return Err("Keyframe does not exists".into());
        }
        let kf = &self.timeline.paths[self.position_path].keyframes[&time];
        if kf.values.contains_key(&self.spectator_id) {
            return Err("Cannot update spectator keyframe".into());
        }
        let index = kf_time_to_index(&self.timeline.paths[self.position_path], time);
        let mut change = UpdateKeyframeProperties::create(self.position_path, index)
            .set_value(self.camera_position_id, PropertyValue::Position(pos.0, pos.1, pos.2))
            .set_value(self.camera_rotation_id, PropertyValue::Rotation(rot.0, rot.1, rot.2))
            .done();
        change.apply(&mut self.timeline)?;
        Ok(Box::new(change))
    }

    /// Upstream removePositionKeyframe.
    pub fn remove_position_keyframe(&mut self, time: i64) -> Result<(), String> {
        if !self.timeline.paths[self.position_path].keyframes.contains_key(&time) {
            return Err("No keyframe at that time".into());
        }
        let index = kf_time_to_index(&self.timeline.paths[self.position_path], time);
        let mut parts: Vec<Box<dyn Change>> = Vec::new();
        let mut remove = RemoveKeyframe::new_with_index(self.position_path, index);
        remove.apply(&mut self.timeline)?;
        parts.push(Box::new(remove));
        let upd = self.update_interpolators()?;
        parts.push(upd);
        let mut spec = self.update_spectator_positions()?;
        spec.apply(&mut self.timeline)?;
        parts.push(spec);
        self.timeline.push_change(Box::new(CombinedChange::create_from_applied(parts)));
        Ok(())
    }

    /// Upstream addTimeKeyframe.
    pub fn add_time_keyframe(&mut self, time: i64, replay_time: i32) -> Result<(), String> {
        if self.timeline.paths[self.time_path].keyframes.contains_key(&time) {
            return Err("Keyframe already exists".into());
        }
        let mut parts: Vec<Box<dyn Change>> = Vec::new();
        let mut add = AddKeyframe::new(self.time_path, time);
        add.apply(&mut self.timeline)?;
        parts.push(Box::new(add));
        let index = kf_time_to_index(&self.timeline.paths[self.time_path], time);
        let mut update = UpdateKeyframeProperties::create(self.time_path, index)
            .set_value(self.timestamp_id, PropertyValue::Timestamp(replay_time))
            .done();
        update.apply(&mut self.timeline)?;
        parts.push(Box::new(update));

        if self.timeline.paths[self.time_path].segments.len() == 1 {
            let id = self.timeline.create_interpolator(Box::new(LinearInterpolator::new()));
            self.timeline.interps[id].register_property(self.timestamp_id);
            let mut set = SetInterpolator::new(self.time_path, 0, id);
            set.apply(&mut self.timeline)?;
            parts.push(Box::new(set));
        }

        let mut spec = self.update_spectator_positions()?;
        spec.apply(&mut self.timeline)?;
        parts.push(spec);
        self.timeline.push_change(Box::new(CombinedChange::create_from_applied(parts)));
        Ok(())
    }

    /// Upstream updateTimeKeyframe.
    pub fn update_time_keyframe(&mut self, time: i64, replay_time: i32) -> Result<Box<dyn Change>, String> {
        if !self.timeline.paths[self.time_path].keyframes.contains_key(&time) {
            return Err("Keyframe does not exists".into());
        }
        let index = kf_time_to_index(&self.timeline.paths[self.time_path], time);
        let mut change = UpdateKeyframeProperties::create(self.time_path, index)
            .set_value(self.timestamp_id, PropertyValue::Timestamp(replay_time))
            .done();
        change.apply(&mut self.timeline)?;
        Ok(Box::new(change))
    }

    /// Upstream removeTimeKeyframe.
    pub fn remove_time_keyframe(&mut self, time: i64) -> Result<(), String> {
        if !self.timeline.paths[self.time_path].keyframes.contains_key(&time) {
            return Err("No keyframe at that time".into());
        }
        let index = kf_time_to_index(&self.timeline.paths[self.time_path], time);
        let mut parts: Vec<Box<dyn Change>> = Vec::new();
        let mut remove = RemoveKeyframe::new_with_index(self.time_path, index);
        remove.apply(&mut self.timeline)?;
        parts.push(Box::new(remove));
        let mut spec = self.update_spectator_positions()?;
        spec.apply(&mut self.timeline)?;
        parts.push(spec);
        self.timeline.push_change(Box::new(CombinedChange::create_from_applied(parts)));
        Ok(())
    }
}

impl SPTimeline {
    /// Upstream setInterpolatorToDefault.
    pub fn set_interpolator_to_default(&mut self, time: i64) -> Result<(), String> {
        if !self.timeline.paths[self.position_path].keyframes.contains_key(&time) {
            return Err("Keyframe does not exists".into());
        }
        let index = kf_time_to_index(&self.timeline.paths[self.position_path], time);
        let mut change = UpdateKeyframeProperties::create(self.position_path, index)
            .remove_property(self.explicit_id)
            .done();
        change.apply(&mut self.timeline)?;
        let upd = self.update_interpolators()?;
        self.timeline.push_change(Box::new(CombinedChange::create_from_applied(vec![Box::new(change), upd])));
        Ok(())
    }

    /// Upstream setInterpolator(time, interpolator): marks the keyframe
    /// explicit and sets the following segment's interpolator.
    pub fn set_interpolator(&mut self, time: i64, interp: InterpId) -> Result<(), String> {
        let p = self.position_path;
        if !self.timeline.paths[p].keyframes.contains_key(&time) {
            return Err("Keyframe does not exists".into());
        }
        let seg = self.timeline.paths[p].segments.iter().position(|s| s.start == time)
            .ok_or("Keyframe has no following segment.")?;
        self.register_position_interpolator_properties(interp);
        let index = kf_time_to_index(&self.timeline.paths[p], time);
        let a = UpdateKeyframeProperties::create(p, index)
            .set_value(self.explicit_id, PropertyValue::Spectator(0)) // ObjectUtils.NULL
            .done();
        let b = SetInterpolator::new(p, seg, interp);
        let mut combined = CombinedChange::create(vec![Box::new(a), Box::new(b)]);
        combined.apply(&mut self.timeline)?;
        let upd = self.update_interpolators()?;
        self.timeline.push_change(Box::new(CombinedChange::create_from_applied(vec![Box::new(combined), upd])));
        Ok(())
    }

    /// Upstream moveKeyframe.
    pub fn move_keyframe(&mut self, p: SPPath, old_time: i64, new_time: i64) -> Result<(), String> {
        let pi = self.get_path(p);
        if !self.timeline.paths[pi].keyframes.contains_key(&old_time) {
            return Err("No keyframe at specified time".into());
        }
        let old_index = kf_time_to_index(&self.timeline.paths[pi], old_time);
        let old_values = self.timeline.paths[pi].keyframes[&old_time].values.clone();
        let is_last_old = *self.timeline.paths[pi].keyframes.keys().last().unwrap() == old_time;
        // Interpolator of the first segment, needed if it remains the only one.
        let first_interp = self.timeline.paths[pi].segments.first().and_then(|s| s.interpolator);
        // The interpolator lost when the old keyframe is removed, restored after.
        let lost_interp = if is_last_old {
            self.timeline.paths[pi].segments.iter().find(|s| s.end == old_time).and_then(|s| s.interpolator)
        } else {
            self.timeline.paths[pi].segments.iter().find(|s| s.start == old_time).and_then(|s| s.interpolator)
        };

        let mut parts: Vec<Box<dyn Change>> = Vec::new();
        let mut remove = RemoveKeyframe::new_with_index(pi, old_index);
        remove.apply(&mut self.timeline)?;
        parts.push(Box::new(remove));
        let mut add = AddKeyframe::new(pi, new_time);
        add.apply(&mut self.timeline)?;
        parts.push(Box::new(add));

        // Copy over all properties.
        let new_index = kf_time_to_index(&self.timeline.paths[pi], new_time);
        let mut builder = UpdateKeyframeProperties::create(pi, new_index);
        for (prop, v) in &old_values {
            builder = builder.set_value(*prop, v.clone());
        }
        let mut property_change = builder.done();
        property_change.apply(&mut self.timeline)?;
        parts.push(Box::new(property_change));

        // Restore the lost interpolator.
        let new_is_last = *self.timeline.paths[pi].keyframes.keys().last().unwrap() == new_time;
        let explicit_id = self.explicit_id;
        let mut restore: Box<dyn Change> = if !new_is_last {
            match (self.timeline.paths[pi].segments.iter().position(|s| s.start == new_time), lost_interp) {
                (Some(i), Some(ip)) => Box::new(SetInterpolator::new(pi, i, ip)),
                _ => Box::new(CombinedChange::create(vec![])),
            }
        } else {
            let has_explicit = self.timeline.paths[pi].keyframes.get(&new_time)
                .map(|k| k.values.contains_key(&explicit_id)).unwrap_or(false);
            match (self.timeline.paths[pi].segments.iter().position(|s| s.end == new_time), lost_interp) {
                (Some(i), Some(ip)) => {
                    if has_explicit {
                        // Transfer the explicit marker to the new segment's start keyframe.
                        let start_time = self.timeline.paths[pi].segments[i].start;
                        let start_index = kf_time_to_index(&self.timeline.paths[pi], start_time);
                        Box::new(CombinedChange::create(vec![
                            Box::new(SetInterpolator::new(pi, i, ip)),
                            Box::new(UpdateKeyframeProperties::create(pi, start_index)
                                .set_value(explicit_id, PropertyValue::Spectator(0)).done()),
                        ]))
                    } else {
                        Box::new(SetInterpolator::new(pi, i, ip))
                    }
                }
                _ => Box::new(CombinedChange::create(vec![])),
            }
        };
        restore.apply(&mut self.timeline)?;
        parts.push(restore);

        // Finally update the interpolators.
        let interp_update: Box<dyn Change> = if p == SPPath::Position {
            // Position / spectator keyframes need special handling;
            // updateInterpolators applies itself internally (upstream bug
            // avoided by brace structure: no second apply on this branch).
            self.update_interpolators()?
        } else {
            // Time keyframes only need updating when only one segment exists.
            if self.timeline.paths[pi].segments.len() == 1 {
                let fi = first_interp.ok_or("One segment should have existed before as well")?;
                let mut c = SetInterpolator::new(pi, 0, fi);
                c.apply(&mut self.timeline)?;
                Box::new(c)
            } else {
                let mut c = CombinedChange::create(vec![]);
                c.apply(&mut self.timeline)?;
                Box::new(c)
            }
        };
        parts.push(interp_update);

        // And update spectator positions.
        let mut spectator = self.update_spectator_positions()?;
        spectator.apply(&mut self.timeline)?;
        parts.push(spectator);

        self.timeline.push_change(Box::new(CombinedChange::create_from_applied(parts)));
        Ok(())
    }

    /// Upstream isSpectatorSegment.
    pub fn is_spectator_segment(&self, p: usize, i: usize) -> bool {
        let seg = &self.timeline.paths[p].segments[i];
        let has = |t: i64| -> bool {
            self.timeline.paths[p].keyframes.get(&t)
                .map(|k| k.values.contains_key(&self.spectator_id)).unwrap_or(false)
        };
        has(seg.start) && has(seg.end)
    }

    /// Upstream updateInterpolators. Applies internally, returns the applied
    /// change (callers wrap it with createFromApplied).
    fn update_interpolators(&mut self) -> Result<Box<dyn Change>, String> {
        let ty = self.default_interpolator_type.ok_or("default interpolator type not set")?;
        let p = self.position_path;
        let seg_count = self.timeline.paths[p].segments.len();
        let spectator_id = self.spectator_id;
        let explicit_id = self.explicit_id;

        let mut updates: BTreeMap<usize, InterpId> = BTreeMap::new();

        // First, setup spectator interpolators. `spectator_interp` mirrors
        // upstream's outer `interpolator` variable, which the split loop
        // below (buggily) also reads.
        let mut spectator_interp: Option<InterpId> = None;
        for i in 0..seg_count {
            if self.is_spectator_segment(p, i) {
                if spectator_interp.is_none() {
                    let id = self.timeline.create_interpolator(Box::new(LinearInterpolator::new()));
                    self.timeline.interps[id].register_property(spectator_id);
                    spectator_interp = Some(id);
                }
                updates.insert(i, spectator_interp.unwrap());
            } else {
                spectator_interp = None;
            }
        }

        // Then, replace stale default interpolators with new ones.
        for i in 0..seg_count {
            let start = self.timeline.paths[p].segments[i].start;
            let explicit = self.timeline.paths[p].keyframes.get(&start)
                .map(|k| k.values.contains_key(&explicit_id)).unwrap_or(false);
            if explicit { continue; }
            if self.is_spectator_segment(p, i) { continue; }
            let cur: Option<InterpId> = match updates.get(&i) {
                Some(id) => Some(*id),
                None => self.timeline.paths[p].segments[i].interpolator,
            };
            // Upstream NPEs when a segment has no interpolator; error instead.
            let cur_id = cur.ok_or("segment has no interpolator")?;
            let cur_kind = self.timeline.interps[cur_id].kind().0;
            if Some(cur_kind) != ty.class_kind() {
                let id = self.create_default_interpolator()?;
                updates.insert(i, id);
            }
        }

        // Split: interpolators that are not continuous get cloned. Upstream
        // quirk preserved verbatim: `used.add(interpolator)` tests the outer
        // spectator-loop variable, not the current interpolator, so with no
        // spectator segments every new interpolator from the second onward
        // is cloned.
        let mut last: Option<InterpId> = None; // Java lastInterpolator (may be null)
        let mut used: Option<InterpId> = None; // the used-set holds at most one non-null member here
        let mut used_had_null = false;
        for i in 0..seg_count {
            if self.is_spectator_segment(p, i) { last = None; continue; }
            let current: Option<InterpId> = match updates.get(&i) {
                Some(id) => Some(*id),
                None => self.timeline.paths[p].segments[i].interpolator,
            };
            if last == current { continue; } // Java ==, including null == null
            let used_add_ok = match spectator_interp {
                None => { if used_had_null { false } else { used_had_null = true; true } }
                Some(id) => { if used == Some(id) { false } else { used = Some(id); true } }
            };
            if !used_add_ok {
                let cur = current.ok_or("cannot clone null interpolator")?;
                let cloned = self.clone_interpolator(cur)?;
                updates.insert(i, cloned);
                last = Some(cloned);
            } else {
                last = current;
            }
        }

        // Merge: neighbouring interpolators that are equal get merged.
        let mut last2: Option<InterpId> = None;
        let mut last2_ser: Option<String> = None; // Java null
        for i in 0..seg_count {
            if self.is_spectator_segment(p, i) { last2 = None; last2_ser = None; continue; }
            let current: Option<InterpId> = match updates.get(&i) {
                Some(id) => Some(*id),
                None => self.timeline.paths[p].segments[i].interpolator,
            };
            let cur = current.ok_or("segment has no interpolator")?;
            let ser = self.serialize_interpolator(cur)?;
            if last2 != current && Some(&ser) == last2_ser.as_ref() {
                updates.insert(i, last2.unwrap());
                continue;
            }
            last2 = current;
            last2_ser = Some(ser);
        }

        let mut changes: Vec<Box<dyn Change>> = Vec::new();
        for (i, id) in &updates {
            changes.push(Box::new(SetInterpolator::new(p, *i, *id)));
        }
        let mut change = CombinedChange::create(changes);
        change.apply(&mut self.timeline)?;
        Ok(Box::new(change))
    }

    /// Upstream updateSpectatorPositions. Returns an UNapplied change.
    fn update_spectator_positions(&mut self) -> Result<Box<dyn Change>, String> {
        if self.entity_tracker.is_none() {
            return Ok(Box::new(CombinedChange::create(vec![])));
        }
        self.timeline.path_update(self.time_path, false)?;
        let kf_times: Vec<i64> = self.timeline.paths[self.position_path].keyframes.keys().copied().collect();
        let mut changes: Vec<Box<dyn Change>> = Vec::new();
        for time in kf_times {
            let spectator = match self.timeline.paths[self.position_path].keyframes[&time]
                .values.get(&self.spectator_id) {
                Some(PropertyValue::Spectator(s)) => *s,
                _ => continue,
            };
            let replay_time = match self.timeline.get_value(self.timestamp_id, time)? {
                Some(PropertyValue::Timestamp(t)) => t,
                _ => continue, // no time keyframe here, cannot determine replay time
            };
            let expected = self.entity_tracker.as_ref().unwrap()
                .get_entity_position_at_timestamp(spectator, replay_time);
            let expected = match expected { Some(l) => l, None => continue };
            let (pos, rot) = {
                let kf = &self.timeline.paths[self.position_path].keyframes[&time];
                (kf.values.get(&self.camera_position_id).cloned()
                    .unwrap_or(PropertyValue::Position(0.0, 0.0, 0.0)),
                 kf.values.get(&self.camera_rotation_id).cloned()
                    .unwrap_or(PropertyValue::Rotation(0.0, 0.0, 0.0)))
            };
            let actual = Location::new(
                match pos { PropertyValue::Position(x, _, _) => x, _ => 0.0 },
                match pos { PropertyValue::Position(_, y, _) => y, _ => 0.0 },
                match pos { PropertyValue::Position(_, _, z) => z, _ => 0.0 },
                match rot { PropertyValue::Rotation(y, _, _) => y, _ => 0.0 },
                match rot { PropertyValue::Rotation(_, p, _) => p, _ => 0.0 },
            );
            if expected != actual {
                let index = kf_time_to_index(&self.timeline.paths[self.position_path], time);
                changes.push(Box::new(UpdateKeyframeProperties::create(self.position_path, index)
                    .set_value(self.camera_position_id, PropertyValue::Position(expected.x, expected.y, expected.z))
                    .set_value(self.camera_rotation_id, PropertyValue::Rotation(expected.yaw, expected.pitch, 0.0))
                    .done()));
            }
        }
        Ok(Box::new(CombinedChange::create(changes)))
    }

    /// Upstream createDefaultInterpolator.
    fn create_default_interpolator(&mut self) -> Result<InterpId, String> {
        let ty = self.default_interpolator_type.ok_or("default interpolator type not set")?;
        let id = self.timeline.create_interpolator(ty.new_instance()?);
        self.register_position_interpolator_properties(id);
        Ok(id)
    }

    /// Upstream registerPositionInterpolatorProperties.
    fn register_position_interpolator_properties(&mut self, interp: InterpId) {
        self.timeline.interps[interp].register_property(self.camera_position_id);
        self.timeline.interps[interp].register_property(self.camera_rotation_id);
    }

    /// Upstream serializeInterpolator (as a JSON string, minimal writer).
    pub fn serialize_interpolator(&self, interp: InterpId) -> Result<String, String> {
        let (kind, alpha) = self.timeline.interps[interp].kind();
        match kind {
            "linear" => Ok("[\"linear\"]".into()),
            "cubic-spline" => Ok("[\"cubic-spline\"]".into()),
            "catmull-rom-spline" => Ok(format!(
                "[{{\"type\":\"catmull-rom-spline\",\"alpha\":{}}}]", alpha.unwrap_or(0.0))),
            other => Err(format!("Unknown interpolator type: {}", other)),
        }
    }

    /// Upstream deserializeInterpolator (minimal reader for the same shapes).
    pub fn deserialize_interpolator_json(&self, json: &str) -> Result<Box<dyn Interpolator>, String> {
        let inner = json.trim()
            .strip_prefix('[').ok_or("Unexpected token")?
            .strip_suffix(']').ok_or("Unexpected token")?
            .trim();
        if inner.starts_with('"') {
            let t = inner.trim_matches('"');
            match t {
                "linear" => Ok(Box::new(LinearInterpolator::new())),
                "cubic-spline" => Ok(Box::new(PolynomialSplineInterpolator::cubic())),
                _ => Err(format!("Unknown interpolation type: {}", t)),
            }
        } else {
            // Object form: {"type":"catmull-rom-spline","alpha":0.5}
            let ty_pos = inner.find("\"type\":\"").ok_or("missing type")? + 8;
            let rest = &inner[ty_pos..];
            let t = &rest[..rest.find('"').ok_or("missing type")?];
            if t != "catmull-rom-spline" { return Err(format!("Unknown interpolation type: {}", t)); }
            let a_pos = inner.find("\"alpha\":").ok_or("Missing alpha value for catmull-rom-spline.")? + 8;
            let rest = &inner[a_pos..];
            let end = rest.find(|c| c == ',' || c == '}').unwrap_or(rest.len());
            let alpha: f64 = rest[..end].trim().parse().map_err(|_| "bad alpha")?;
            Ok(Box::new(CatmullRomSplineInterpolator::new(alpha)))
        }
    }

    /// Upstream cloneInterpolator: deserialize(serialize(x)), re-registering
    /// the original's keyframe properties.
    fn clone_interpolator(&mut self, interp: InterpId) -> Result<InterpId, String> {
        let json = self.serialize_interpolator(interp)?;
        let cloned = self.deserialize_interpolator_json(&json)?;
        let props: Vec<PropertyId> = self.timeline.interps[interp]
            .keyframe_properties().iter().copied().collect();
        let new_id = self.timeline.create_interpolator(cloned);
        for p in props {
            self.timeline.interps[new_id].register_property(p);
        }
        Ok(new_id)
    }
}
