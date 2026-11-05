//! Path preview sampling: 1:1 simplepathing.preview.PathPreviewRenderer
//! renderCameraPath — the camera-path sampling, coloring and camera-marker
//! logic. Upstream draws the sampled points with GL (drawConnection,
//! drawPoint, drawCamera are Horizon's render seam); this port produces the
//! same point/color plan headless.
//! This file: Copyright (c) 2026 sparticle, LGPL-3.0-or-later.
use crate::sptimeline::{SPTimeline, SPPath};
use crate::pathing::PropertyValue;

pub const SLOW_PATH_COLOR: u32 = 0xffcccc;
pub const FAST_PATH_COLOR: u32 = 0x660000;
pub const FASTEST_PATH_SPEED: f64 = 0.01;
pub const SPECTATOR_LINE_COLOR: u32 = 0x0000ffff;

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Vec3f { pub x: f32, pub y: f32, pub z: f32 }
impl Vec3f { pub fn new(x: f32, y: f32, z: f32) -> Self { Self { x, y, z } } }

pub fn distance_squared(a: Vec3f, b: Vec3f) -> f64 {
    let dx = (a.x - b.x) as f64; let dy = (a.y - b.y) as f64; let dz = (a.z - b.z) as f64;
    dx * dx + dy * dy + dz * dz
}

fn loc2vec(loc: &crate::sptimeline::Location) -> Vec3f {
    Vec3f::new(loc.x as f32, loc.y as f32, loc.z as f32)
}
fn triple_d2vec(x: f64, y: f64, z: f64) -> Vec3f { Vec3f::new(x as f32, y as f32, z as f32) }
fn triple_f2vec(y: f32, p: f32, r: f32) -> Vec3f { Vec3f::new(y, p, r) }

/// 1:1 interpolateColorComponent: exponential ease between the components.
fn interpolate_color_component(c1: u32, c2: u32, weight: f64) -> u32 {
    let v = c1 as f64 + (1.0 - (-4.0 * weight).exp()) * (c2 as f64 - c1 as f64);
    (v as i32 & 0xff) as u32
}

pub fn interpolate_color(c1: u32, c2: u32, weight: f64) -> u32 {
    (interpolate_color_component((c1 >> 16) & 0xff, (c2 >> 16) & 0xff, weight) << 16)
        | (interpolate_color_component((c1 >> 8) & 0xff, (c2 >> 8) & 0xff, weight) << 8)
        | interpolate_color_component(c1 & 0xff, c2 & 0xff, weight)
}

/// One sampled connection: a colored line between two sampled points.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PathPoint { pub from: Vec3f, pub to: Vec3f, pub color: u32 }
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct KeyframePoint { pub time: i64, pub pos: Vec3f }
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct CameraMarker { pub pos: Vec3f, pub rot: Vec3f }

#[derive(Clone, Debug, Default)]
pub struct PathPreviewPlan {
    pub segments: Vec<Vec<PathPoint>>,
    pub keyframes: Vec<KeyframePoint>,
    pub camera: Option<CameraMarker>,
}

/// 1:1 renderCameraPath minus the GL: sample every path segment into colored
/// connections, collect keyframe points in view range (furthest first), and
/// resolve the camera marker at the cursor. Returns None for the upstream
/// early-out guards (not async mode, HUD hidden, empty path).
pub fn render_camera_path(
    sp: &mut SPTimeline,
    cursor_time: i64,
    view_pos: Vec3f,
    render_distance: i32,
    hud_hidden: bool,
    async_mode: bool,
) -> Result<Option<PathPreviewPlan>, String> {
    if !async_mode || hud_hidden { return Ok(None); }
    let p = sp.position_path;
    let time_path = sp.time_path;
    if sp.timeline.paths[p].keyframes.is_empty() { return Ok(None); }
    sp.timeline.path_update(p, false)?;
    let render_distance_squared = (render_distance * render_distance) as f64;

    let mut plan = PathPreviewPlan::default();
    for (i, seg) in sp.timeline.paths[p].segments.clone().iter().enumerate() {
        // Spectator segments have 20 lines per second (at least 10); normal
        // segments have a fixed 100.
        let spectator = match seg.interpolator {
            Some(id) => sp.timeline.interps[id].keyframe_properties().contains(&sp.spectator_id),
            None => false,
        };
        if spectator && sp.entity_tracker.is_none() { continue; }
        let diff = (seg.end - seg.start) as i32 as i64;
        let steps: i64 = if spectator { std::cmp::max(diff / 50, 10) } else { 100 };
        let mut prev: Option<Vec3f> = None;
        let mut pts: Vec<PathPoint> = Vec::new();
        for s in 0..=steps {
            let time = seg.start + diff * s / steps;
            if spectator {
                let ent = sp.timeline.path_get_value(p, sp.spectator_id, time)?;
                let rt = sp.timeline.path_get_value(time_path, sp.timestamp_id, time)?;
                if let (Some(PropertyValue::Spectator(eid)), Some(PropertyValue::Timestamp(rt))) = (ent, rt) {
                    let loc = sp.entity_tracker.as_ref().unwrap()
                        .get_entity_position_at_timestamp(eid, rt);
                    if let Some(loc) = loc {
                        let pos = loc2vec(&loc);
                        if let Some(from) = prev {
                            pts.push(PathPoint { from, to: pos, color: SPECTATOR_LINE_COLOR });
                        }
                        prev = Some(pos);
                        continue;
                    }
                }
            } else if let Some(PropertyValue::Position(x, y, z)) =
                sp.timeline.path_get_value(p, sp.camera_position_id, time)? {
                let pos = triple_d2vec(x, y, z);
                if let Some(from) = prev {
                    let distance = distance_squared(from, pos).sqrt();
                    let speed = (distance / (diff / steps) as f64).min(FASTEST_PATH_SPEED);
                    let fraction = speed / FASTEST_PATH_SPEED;
                    let color = (interpolate_color(SLOW_PATH_COLOR, FAST_PATH_COLOR, fraction) << 8) | 0xff;
                    pts.push(PathPoint { from, to: pos, color });
                }
                prev = Some(pos);
                continue;
            }
            prev = None;
        }
        plan.segments.push(pts);
        let _ = i;
    }

    // Keyframe points in view range, furthest first.
    let mut kfs: Vec<KeyframePoint> = Vec::new();
    for (time, kf) in sp.timeline.paths[p].keyframes.iter() {
        if let Some(PropertyValue::Position(x, y, z)) = kf.values.get(&sp.camera_position_id) {
            let pos = triple_d2vec(*x, *y, *z);
            if distance_squared(pos, view_pos) < render_distance_squared {
                kfs.push(KeyframePoint { time: *time, pos });
            }
        }
    }
    kfs.sort_by(|a, b| {
        distance_squared(b.pos, view_pos).partial_cmp(&distance_squared(a.pos, view_pos)).unwrap()
    });
    plan.keyframes = kfs;

    // Camera marker at the cursor: spectated entity position, else the path.
    let ent = sp.timeline.path_get_value(p, sp.spectator_id, cursor_time)?;
    if let Some(PropertyValue::Spectator(eid)) = ent {
        if let Some(tracker) = sp.entity_tracker.as_ref() {
            let rt = sp.timeline.path_get_value(time_path, sp.timestamp_id, cursor_time)?;
            if let Some(PropertyValue::Timestamp(rt)) = rt {
                if let Some(loc) = tracker.get_entity_position_at_timestamp(eid, rt) {
                    plan.camera = Some(CameraMarker {
                        pos: loc2vec(&loc),
                        rot: Vec3f::new(loc.yaw, loc.pitch, 0.0),
                    });
                }
            }
        }
    } else {
        let pos = sp.timeline.path_get_value(p, sp.camera_position_id, cursor_time)?;
        let rot = sp.timeline.path_get_value(p, sp.camera_rotation_id, cursor_time)?;
        if let (Some(PropertyValue::Position(x, y, z)), Some(PropertyValue::Rotation(ya, pi, ro))) = (pos, rot) {
            plan.camera = Some(CameraMarker {
                pos: triple_d2vec(x, y, z),
                rot: triple_f2vec(ya, pi, ro),
            });
        }
    }
    Ok(Some(plan))
}
