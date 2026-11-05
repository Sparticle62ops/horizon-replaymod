//! Change classes: 1:1 Rust port of ReplayStudio pathing/change
//! (AddKeyframe, RemoveKeyframe, UpdateKeyframeProperties, SetInterpolator,
//! CombinedChange). Upstream Preconditions.checkState -> Result.
//! Copyright (c) 2026 sparticle, LGPL-3.0-or-later.
use crate::pathing::*;
use std::collections::BTreeMap;

/// Nth keyframe time (sorted), upstream Iterables.get(path.getKeyframes(), i).
pub fn kf_index_to_time(p: &Path, index: usize) -> Option<i64> {
    p.keyframes.keys().copied().nth(index)
}

pub fn kf_time_to_index(p: &Path, time: i64) -> usize {
    p.keyframes.keys().filter(|k| **k < time).count()
}

/// Upstream AddKeyframe.
pub struct AddKeyframe { path: usize, time: i64, index: Option<usize>, applied: bool }

impl AddKeyframe {
    pub fn new(path: usize, time: i64) -> Self {
        Self { path, time, index: None, applied: false }
    }
}

impl Change for AddKeyframe {
    fn apply(&mut self, t: &mut Timeline) -> Result<(), String> {
        if self.applied { return Err("Already applied!".into()); }
        t.path_insert(self.path, self.time, BTreeMap::new())?;
        self.index = Some(kf_time_to_index(&t.paths[self.path], self.time));
        self.applied = true;
        Ok(())
    }
    fn undo(&mut self, t: &mut Timeline) -> Result<(), String> {
        if !self.applied { return Err("Not yet applied!".into()); }
        let index = self.index.ok_or("not applied")?;
        let time = kf_index_to_time(&t.paths[self.path], index).ok_or("keyframe missing")?;
        t.path_remove(self.path, time, true)?;
        self.applied = false;
        Ok(())
    }
}

/// Upstream RemoveKeyframe. Stores the removed keyframe and the interpolator
/// that would be lost, for the undo.
pub struct RemoveKeyframe {
    path: usize, index: usize,
    removed: Option<(i64, BTreeMap<PropertyId, PropertyValue>)>,
    removed_interpolator: Option<InterpId>,
    applied: bool,
}

impl RemoveKeyframe {
    /// Upstream create(Path, Keyframe): the index is the keyframe's position
    /// in the path's sorted keyframe list; callers pass it (SPTimeline
    /// computes it from the keyframe time).
    pub fn new_with_index(path: usize, index: usize) -> Self {
        Self { path, index, removed: None, removed_interpolator: None, applied: false }
    }
}

impl Change for RemoveKeyframe {
    fn apply(&mut self, t: &mut Timeline) -> Result<(), String> {
        if self.applied { return Err("Already applied!".into()); }
        let path = &t.paths[self.path];
        if !path.segments.is_empty() {
            // Last keyframe keeps the left-hand interpolator, others the right-hand one.
            if self.index == path.segments.len() {
                self.removed_interpolator = path.segments[self.index - 1].interpolator;
            } else {
                self.removed_interpolator = path.segments[self.index].interpolator;
            }
        }
        let time = kf_index_to_time(path, self.index).ok_or("no such keyframe")?;
        self.removed = Some((time, path.keyframes[&time].values.clone()));
        t.path_remove(self.path, time, true)?;
        self.applied = true;
        Ok(())
    }
    fn undo(&mut self, t: &mut Timeline) -> Result<(), String> {
        if !self.applied { return Err("Not yet applied!".into()); }
        let (time, values) = self.removed.clone().ok_or("not applied")?;
        t.path_insert(self.path, time, values)?;
        if let Some(ip) = self.removed_interpolator {
            if self.index == t.paths[self.path].segments.len() {
                t.seg_set_interpolator(self.path, self.index - 1, Some(ip));
            } else {
                t.seg_set_interpolator(self.path, self.index, Some(ip));
            }
        }
        self.applied = false;
        Ok(())
    }
}

/// Upstream UpdateKeyframeProperties (builder + change).
pub struct UpdateKeyframeProperties {
    path: usize, index: usize,
    new_values: BTreeMap<PropertyId, Option<PropertyValue>>,
    old_values: BTreeMap<PropertyId, Option<PropertyValue>>,
    applied: bool,
}

pub struct UpdateKeyframePropertiesBuilder {
    path: usize, index: usize,
    updates: BTreeMap<PropertyId, Option<PropertyValue>>,
}

impl UpdateKeyframePropertiesBuilder {
    pub fn set_value(mut self, prop: PropertyId, v: PropertyValue) -> Self {
        self.updates.insert(prop, Some(v)); self
    }
    pub fn remove_property(mut self, prop: PropertyId) -> Self {
        self.updates.insert(prop, None); self
    }
    pub fn done(self) -> UpdateKeyframeProperties {
        UpdateKeyframeProperties { path: self.path, index: self.index, new_values: self.updates, old_values: BTreeMap::new(), applied: false }
    }
}

impl UpdateKeyframeProperties {
    /// Upstream create(Path, Keyframe): builder positioned at that keyframe.
    pub fn create(path: usize, index: usize) -> UpdateKeyframePropertiesBuilder {
        UpdateKeyframePropertiesBuilder { path, index, updates: BTreeMap::new() }
    }
}

impl Change for UpdateKeyframeProperties {
    fn apply(&mut self, t: &mut Timeline) -> Result<(), String> {
        if self.applied { return Err("Already applied!".into()); }
        let time = kf_index_to_time(&t.paths[self.path], self.index).ok_or("no such keyframe")?;
        let props: Vec<(PropertyId, Option<PropertyValue>)> = self.new_values.iter().map(|(k, v)| (*k, v.clone())).collect();
        {
            let kf = &mut t.paths[self.path].keyframes.get_mut(&time).ok_or("no such keyframe")?;
            for (prop, nv) in props {
                self.old_values.insert(prop, kf.values.get(&prop).cloned());
                match nv {
                    Some(v) => { kf.values.insert(prop, v); }
                    None => { kf.values.remove(&prop); }
                }
            }
        }
        self.applied = true;
        Ok(())
    }
    fn undo(&mut self, t: &mut Timeline) -> Result<(), String> {
        if !self.applied { return Err("Not yet applied!".into()); }
        let time = kf_index_to_time(&t.paths[self.path], self.index).ok_or("no such keyframe")?;
        let old: Vec<(PropertyId, Option<PropertyValue>)> = self.old_values.iter().map(|(k, v)| (*k, v.clone())).collect();
        {
            let kf = &mut t.paths[self.path].keyframes.get_mut(&time).ok_or("no such keyframe")?;
            for (prop, ov) in old {
                self.new_values.insert(prop, kf.values.get(&prop).cloned());
                match ov {
                    Some(v) => { kf.values.insert(prop, v); }
                    None => { kf.values.remove(&prop); }
                }
            }
        }
        self.applied = false;
        Ok(())
    }
}

/// Upstream SetInterpolator.
pub struct SetInterpolator { path: usize, index: usize, interp: InterpId, old_interp: Option<InterpId>, applied: bool }

impl SetInterpolator {
    pub fn new(path: usize, seg_index: usize, interp: InterpId) -> Self {
        Self { path, index: seg_index, interp, old_interp: None, applied: false }
    }
}

impl Change for SetInterpolator {
    fn apply(&mut self, t: &mut Timeline) -> Result<(), String> {
        if self.applied { return Err("Already applied!".into()); }
        if self.index >= t.paths[self.path].segments.len() { return Err("no such segment".into()); }
        self.old_interp = t.paths[self.path].segments[self.index].interpolator;
        t.seg_set_interpolator(self.path, self.index, Some(self.interp));
        self.applied = true;
        Ok(())
    }
    fn undo(&mut self, t: &mut Timeline) -> Result<(), String> {
        if !self.applied { return Err("Not yet applied!".into()); }
        t.seg_set_interpolator(self.path, self.index, self.old_interp);
        self.applied = false;
        Ok(())
    }
}

/// Upstream CombinedChange.
pub struct CombinedChange { changes: Vec<Box<dyn Change>>, applied: bool }

impl CombinedChange {
    pub fn create(changes: Vec<Box<dyn Change>>) -> Self { Self { changes, applied: false } }
    pub fn create_from_applied(changes: Vec<Box<dyn Change>>) -> Self { Self { changes, applied: true } }
}

impl Change for CombinedChange {
    fn apply(&mut self, t: &mut Timeline) -> Result<(), String> {
        if self.applied { return Err("Already applied!".into()); }
        for c in &mut self.changes { c.apply(t)?; }
        self.applied = true;
        Ok(())
    }
    fn undo(&mut self, t: &mut Timeline) -> Result<(), String> {
        if !self.applied { return Err("Not yet applied!".into()); }
        for c in self.changes.iter_mut().rev() { c.undo(t)?; }
        self.applied = false;
        Ok(())
    }
}
