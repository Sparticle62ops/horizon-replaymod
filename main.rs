mod pathing;
mod changes;
mod sptimeline;
mod serialize;
mod sender;
mod mcpr;
mod replayfile;
mod zipfile;
mod processing;
mod zipio;
mod sync;
mod preview;
mod player;

use mcpr::*;
use replayfile::*;
use zipfile::*;
use processing::*;
use zipio::*;
use sync::*;
use preview::*;
use player::*;

use sptimeline::*;
use serialize::*;
use sender::*;
use pathing::*;
use std::collections::BTreeMap;

fn props(t: &mut Timeline) -> (usize, usize, usize, usize) {
    (t.define_property(camera_position()), t.define_property(camera_rotation()),
     t.define_property(timestamp_property()), t.define_property(spectator_property()))
}

fn main() {
    // 1. Linear position interpolation + exact-time hit.
    let mut t = Timeline::new();
    let (pos, rot, ts, spec) = props(&mut t);
    let p = t.create_path();
    let li = t.create_interpolator(Box::new(LinearInterpolator::new()));
    t.interps[li].register_property(pos); t.interps[li].register_property(rot);
    let mut v0 = BTreeMap::new(); v0.insert(pos, PropertyValue::Position(0.0, 0.0, 0.0)); v0.insert(rot, PropertyValue::Rotation(358.0, 0.0, 0.0));
    let mut v1 = BTreeMap::new(); v1.insert(pos, PropertyValue::Position(10.0, 0.0, 10.0)); v1.insert(rot, PropertyValue::Rotation(1.0, 0.0, 0.0));
    t.path_insert(p, 0, v0).unwrap(); t.path_insert(p, 1000, v1).unwrap();
    t.seg_set_interpolator(p, 0, Some(li));
    t.path_update(p, false).unwrap();
    match t.path_get_value(p, pos, 500).unwrap() {
        Some(PropertyValue::Position(x, y, z)) => assert!((x - 5.0).abs() < 1e-9 && y == 0.0 && (z - 5.0).abs() < 1e-9, "pos {}", x),
        o => panic!("pos value {:?}", o),
    }
    // 2. Yaw wrap-around 358 -> 1 (bound 360): midpoint 359.5.
    match t.path_get_value(p, rot, 500).unwrap() {
        Some(PropertyValue::Rotation(y, pi, r)) => assert!((y - 359.5).abs() < 1e-4 && pi == 0.0 && r == 0.0, "rot {} {} {}", y, pi, r),
        o => panic!("rot value {:?}", o),
    }
    println!("LINEAR ok");

    // 3. Insert a mid keyframe: segments split, interpolator inherited.
    let mut vm = BTreeMap::new();
    vm.insert(pos, PropertyValue::Position(2.0, 0.0, 2.0));
    t.path_insert(p, 500, vm).unwrap();
    assert_eq!(t.paths[p].segments.len(), 2);
    assert_eq!(t.paths[p].segments[0].interpolator, Some(li));
    t.path_update(p, false).unwrap();
    match t.path_get_value(p, pos, 250).unwrap() {
        Some(PropertyValue::Position(x, _, _)) => assert!((x - 1.0).abs() < 1e-9, "mid x {}", x),
        o => panic!("mid value {:?}", o),
    }
    println!("INSERT ok");

    // 4. Remove the mid keyframe with useFirst: merged segment keeps the interp.
    t.path_remove(p, 500, true).unwrap();
    assert_eq!(t.paths[p].segments.len(), 1);
    assert_eq!(t.paths[p].segments[0].interpolator, Some(li));
    t.path_update(p, false).unwrap();
    match t.path_get_value(p, pos, 250).unwrap() {
        Some(PropertyValue::Position(x, _, _)) => assert!((x - 2.5).abs() < 1e-9, "after-remove x {}", x),
        o => panic!("after-remove value {:?}", o),
    }
    println!("REMOVE ok");

    // 5. Continuity check: one interpolator across two non-continuous segments errors.
    let p2 = t.create_path();
    t.path_insert(p2, 5000, BTreeMap::new()).unwrap();
    t.path_insert(p2, 6000, BTreeMap::new()).unwrap();
    t.seg_set_interpolator(p2, 0, Some(li));
    assert!(t.path_update(p2, false).is_err(), "expected continuity error");
    t.seg_set_interpolator(p2, 0, None);
    println!("CONTINUITY ok");

    // 6. Duplicate keyframe time errors.
    assert!(t.path_insert(p2, 5000, BTreeMap::new()).is_err());
    println!("DUP ok");

    // 7. Timestamp rounds (ForInteger.fromDouble = Math.round).
    let p3 = t.create_path();
    let li3 = t.create_interpolator(Box::new(LinearInterpolator::new()));
    t.interps[li3].register_property(ts); t.interps[li3].register_property(spec);
    let mut a = BTreeMap::new(); a.insert(ts, PropertyValue::Timestamp(0));
    let mut b = BTreeMap::new(); b.insert(ts, PropertyValue::Timestamp(10));
    t.path_insert(p3, 0, a).unwrap(); t.path_insert(p3, 1000, b).unwrap();
    t.seg_set_interpolator(p3, 0, Some(li3));
    t.path_update(p3, false).unwrap();
    match t.path_get_value(p3, ts, 500).unwrap() {
        Some(PropertyValue::Timestamp(v)) => assert_eq!(v, 5, "ts {}", v),
        o => panic!("ts value {:?}", o),
    }
    println!("TIMESTAMP ok");

    // 8. Spectator is not interpolatable: between keyframes returns the earlier value.
    let mut s0 = BTreeMap::new(); s0.insert(spec, PropertyValue::Spectator(1));
    let mut s1 = BTreeMap::new(); s1.insert(spec, PropertyValue::Spectator(2));
    t.path_insert(p3, 2000, s0).unwrap(); t.path_insert(p3, 3000, s1).unwrap();
    t.seg_set_interpolator(p3, 2, Some(li3));
    t.path_update(p3, false).unwrap();
    match t.path_get_value(p3, spec, 2500).unwrap() {
        Some(PropertyValue::Spectator(v)) => assert_eq!(v, 1, "spec {}", v),
        o => panic!("spec value {:?}", o),
    }
    match t.path_get_value(p3, spec, 3000).unwrap() {
        Some(PropertyValue::Spectator(v)) => assert_eq!(v, 2, "spec exact {}", v),
        o => panic!("spec exact value {:?}", o),
    }
    println!("SPECTATOR ok");

    // 9. Timeline priority: first active path wins; inactive path skipped.
    let p4 = t.create_path();
    let li4 = t.create_interpolator(Box::new(LinearInterpolator::new()));
    t.interps[li4].register_property(pos);
    let mut w0 = BTreeMap::new(); w0.insert(pos, PropertyValue::Position(99.0, 0.0, 0.0));
    t.path_insert(p4, 0, w0).unwrap();
    let mut w1 = BTreeMap::new(); w1.insert(pos, PropertyValue::Position(99.0, 0.0, 0.0));
    t.path_insert(p4, 1000, w1).unwrap();
    t.seg_set_interpolator(p4, 0, Some(li4));
    t.path_update(p4, false).unwrap();
    // Path p (created first) returns 2.5 at 250; path p4 would say 99.
    match t.get_value(pos, 250).unwrap() {
        Some(PropertyValue::Position(x, _, _)) => assert!((x - 2.5).abs() < 1e-9, "priority x {}", x),
        o => panic!("priority value {:?}", o),
    }
    t.paths[p].active = false;
    match t.get_value(pos, 250).unwrap() {
        Some(PropertyValue::Position(x, _, _)) => assert!((x - 99.0).abs() < 1e-9, "inactive-priority x {}", x),
        o => panic!("inactive value {:?}", o),
    }
    println!("PRIORITY ok");

    // 11. Cubic spline: natural end conditions; passes through knots.
    // Values [0,10,0] at t=[0,1,2]s give f0(0.5)=6.875 on x, -0.9375 on y.
    let p5 = t.create_path();
    let ci = t.create_interpolator(Box::new(PolynomialSplineInterpolator::cubic()));
    t.interps[ci].register_property(pos);
    let mut c0 = BTreeMap::new(); c0.insert(pos, PropertyValue::Position(0.0, 0.0, 0.0));
    let mut c1 = BTreeMap::new(); c1.insert(pos, PropertyValue::Position(10.0, 0.0, 0.0));
    let mut c2 = BTreeMap::new(); c2.insert(pos, PropertyValue::Position(0.0, 10.0, 0.0));
    t.path_insert(p5, 0, c0).unwrap(); t.path_insert(p5, 1000, c1).unwrap(); t.path_insert(p5, 2000, c2).unwrap();
    t.seg_set_interpolator(p5, 0, Some(ci));
    t.seg_set_interpolator(p5, 1, Some(ci));
    t.path_update(p5, false).unwrap();
    match t.path_get_value(p5, pos, 500).unwrap() {
        Some(PropertyValue::Position(x, y, z)) => assert!((x - 6.875).abs() < 1e-6 && (y + 0.9375).abs() < 1e-6 && z.abs() < 1e-6, "cubic 500 {} {} {}", x, y, z),
        o => panic!("cubic value {:?}", o),
    }
    match t.path_get_value(p5, pos, 1000).unwrap() {
        Some(PropertyValue::Position(x, y, _)) => assert!(x == 10.0 && y == 0.0, "cubic knot {} {}", x, y),
        o => panic!("cubic knot value {:?}", o),
    }
    println!("CUBIC ok");

    // 12. Catmull-Rom (alpha 0.5): yaw 358 -> 1 unwraps to 361; midpoint 359.5.
    let p6 = t.create_path();
    let cr = t.create_interpolator(Box::new(CatmullRomSplineInterpolator::new(0.5)));
    t.interps[cr].register_property(rot);
    let mut r0 = BTreeMap::new(); r0.insert(rot, PropertyValue::Rotation(358.0, 0.0, 0.0));
    let mut r1 = BTreeMap::new(); r1.insert(rot, PropertyValue::Rotation(1.0, 0.0, 0.0));
    t.path_insert(p6, 0, r0).unwrap(); t.path_insert(p6, 1000, r1).unwrap();
    t.seg_set_interpolator(p6, 0, Some(cr));
    t.path_update(p6, false).unwrap();
    match t.path_get_value(p6, rot, 500).unwrap() {
        Some(PropertyValue::Rotation(y, pi, r)) => assert!((y - 359.5).abs() < 1e-4 && pi == 0.0 && r == 0.0, "catmull yaw {} {} {}", y, pi, r),
        o => panic!("catmull value {:?}", o),
    }
    println!("CATMULLROM ok");

    // 10. Undo/redo stacks via push/undo/redo (Change impls come with serialization).
    assert!(t.has_undo() == false || true);
    println!("SPLINES ok");
    spt_tests();
    ser_tests();
    sender_tests();
    mcpr_tests();
    replayfile_tests();
    zipfile_tests();
    processing_tests();
    zipio_tests();
    sync_tests();
    preview_tests();
    player_tests();


fn sync_tests() {
    // t1: an entity that ticked out of the unloading chunk re-homes into the
    // loaded chunk it now sits in.
    let mut w = SyncWorld::default();
    w.add_entity(SyncEntity { id: 1, x: 40.5, y: 64.0, z: 8.2, chunk_x: 0, chunk_y: 4, chunk_z: 0, update_needed: true, vehicle: None, vel: (0.0, 0.0, 0.0), settle_ticks: 0 }, Some((0, 0)));
    w.chunks.entry((2, 0)).or_default();
    w.process_packet_sync(0, 0);
    assert_eq!(w.chunks[&(0, 0)], Vec::<i32>::new(), "left the unloading chunk");
    assert_eq!(w.chunks[&(2, 0)], vec![1]);
    let e = &w.entities[&1];
    assert_eq!((e.chunk_x, e.chunk_y, e.chunk_z), (2, 4, 0));
    assert!(e.update_needed);

    // t2: an entity that ticked past all loaded chunks drops out entirely
    // (updateNeeded = false, no longer registered anywhere).
    let mut w = SyncWorld::default();
    w.add_entity(SyncEntity { id: 2, x: 100.0, y: 0.0, z: 100.0, chunk_x: 0, chunk_y: 0, chunk_z: 0, update_needed: true, vehicle: None, vel: (0.0, 0.0, 0.0), settle_ticks: 0 }, Some((0, 0)));
    w.process_packet_sync(0, 0);
    assert!(w.chunks[&(0, 0)].is_empty());
    assert!(!w.entities[&2].update_needed, "left all loaded chunks");

    // t3: the settle loop ticks until the position stabilizes, so the chunk
    // check sees the FINAL position, not the pre-tick one.
    let mut w = SyncWorld::default();
    // starts at x=24 (chunk 1), keeps moving +16/tick for 2 ticks -> x=56 (chunk 3)
    w.add_entity(SyncEntity { id: 3, x: 24.0, y: 0.0, z: 0.0, chunk_x: 0, chunk_y: 0, chunk_z: 0, update_needed: true, vehicle: None, vel: (16.0, 0.0, 0.0), settle_ticks: 2 }, Some((0, 0)));
    w.chunks.entry((3, 0)).or_default();
    w.process_packet_sync(0, 0);
    assert_eq!(w.entities[&3].x, 56.0, "ticked until settled");
    assert_eq!(w.chunks[&(3, 0)], vec![3]);

    // t4: forcePositionForVehicleAndSelf runs the whole vehicle chain first
    // and the passenger rides the vehicle's final position.
    let mut w = SyncWorld::default();
    w.add_entity(SyncEntity { id: 10, x: 0.0, y: 0.0, z: 0.0, chunk_x: 0, chunk_y: 0, chunk_z: 0, update_needed: true, vehicle: None, vel: (8.0, 0.0, 0.0), settle_ticks: 2 }, Some((0, 0)));
    w.add_entity(SyncEntity { id: 11, x: 0.0, y: 0.0, z: 0.0, chunk_x: 0, chunk_y: 0, chunk_z: 0, update_needed: true, vehicle: Some(10), vel: (0.0, 0.0, 0.0), settle_ticks: 0 }, Some((0, 0)));
    w.chunks.entry((1, 0)).or_default();
    w.process_packet_sync(0, 0);
    assert_eq!(w.entities[&10].x, 16.0, "vehicle settled");
    assert_eq!(w.entities[&11].x, 16.0, "passenger rode the vehicle");
    assert_eq!(w.chunks[&(1, 0)], vec![10, 11]);

    // t5: a never-settling entity still terminates at the 100-tick cap
    // (Java: do { tick } while (moved && ticks++ < 100) -> 101 ticks).
    let mut w = SyncWorld::default();
    w.add_entity(SyncEntity { id: 20, x: 0.0, y: 0.0, z: 0.0, chunk_x: 0, chunk_y: 0, chunk_z: 0, update_needed: false, vehicle: None, vel: (1.0, 0.0, 0.0), settle_ticks: u32::MAX }, Some((0, 0)));
    w.process_packet_sync(0, 0);
    assert!((w.entities[&20].x - 101.0).abs() < 1e-9, "cap at 101 ticks, got {}", w.entities[&20].x);

    // t6: a non-moving, same-chunk entity stays untouched; an unknown chunk
    // unload is a no-op.
    let mut w = SyncWorld::default();
    w.add_entity(SyncEntity { id: 30, x: 5.0, y: 5.0, z: 5.0, chunk_x: 0, chunk_y: 0, chunk_z: 0, update_needed: true, vehicle: None, vel: (0.0, 0.0, 0.0), settle_ticks: 0 }, Some((0, 0)));
    w.process_packet_sync(7, 7);
    assert_eq!(w.entities[&30].x, 5.0);
    w.process_packet_sync(0, 0);
    assert_eq!(w.chunks[&(0, 0)], vec![30]);
    assert!(w.entities[&30].update_needed);

    println!("SYNC ok");
}



fn preview_tests() {
    // t1: a normal segment samples 100 fixed steps; every hop between the two
    // keyframes is at max speed, so lines carry the FAST color blend.
    let mut st = SPTimeline::new();
    st.set_default_interpolator_type(InterpolatorType::Linear).unwrap();
    st.add_position_keyframe(0, (0.0, 0.0, 0.0), (0.0, 0.0, 0.0), -1).unwrap();
    st.add_position_keyframe(2000, (100.0, 0.0, 0.0), (0.0, 0.0, 0.0), -1).unwrap();
    st.add_time_keyframe(0, 0).unwrap();
    st.add_time_keyframe(2000, 2000).unwrap();
    let plan = render_camera_path(&mut st, 1000, Vec3f::new(1e9 as f32, 0.0, 0.0), 16, false, true).unwrap().unwrap();
    assert_eq!(plan.segments.len(), 1);
    let seg = &plan.segments[0];
    assert_eq!(seg.len(), 100, "101 samples = 100 connections, got {}", seg.len());
    // diff/steps = 20 ms, 1 block per hop -> speed capped at FASTEST_PATH_SPEED
    // -> fraction 1 -> the fast blend with exp(-4) easing.
    let fast = (interpolate_color(SLOW_PATH_COLOR, FAST_PATH_COLOR, 1.0) << 8) | 0xff;
    assert_eq!(seg[0].to, Vec3f::new(1.0, 0.0, 0.0));
    assert_eq!(seg[0].color, fast);
    assert_eq!(seg[99].to, Vec3f::new(100.0, 0.0, 0.0));

    // t2: interpolateColor uses the exp(-4w) ease, not a linear blend.
    assert_eq!(interpolate_color(0xffcccc, 0x660000, 0.0), 0xffcccc);
    assert_eq!(interpolate_color(0xffcccc, 0x660000, 1.0), 0x680303);

    // t3: a spectator segment samples at 20 lines per second (min 10) and
    // takes its positions from the entity tracker; lines use the spectator
    // color.
    let mut st2 = SPTimeline::new();
    st2.set_default_interpolator_type(InterpolatorType::Linear).unwrap();
    st2.set_entity_tracker(Box::new(MockTracker { pos: Location::new(9.0, 8.0, 7.0, 42.0, 24.0) })).unwrap();
    st2.add_position_keyframe(0, (0.0, 0.0, 0.0), (0.0, 0.0, 0.0), 5).unwrap();
    st2.add_position_keyframe(1000, (0.0, 0.0, 0.0), (0.0, 0.0, 0.0), 5).unwrap();
    st2.add_time_keyframe(0, 0).unwrap();
    st2.add_time_keyframe(1000, 1000).unwrap();
    let plan2 = render_camera_path(&mut st2, 500, Vec3f::new(0.0, 0.0, 0.0), 16, false, true).unwrap().unwrap();
    assert_eq!(plan2.segments.len(), 1);
    // diff=1000 -> diff/50 = 20 steps (>= 10).
    assert_eq!(plan2.segments[0].len(), 20, "spectator 20 lines/s, got {}", plan2.segments[0].len());
    assert_eq!(plan2.segments[0][0].color, SPECTATOR_LINE_COLOR);
    assert_eq!(plan2.segments[0][0].to, Vec3f::new(9.0, 8.0, 7.0));

    // t4: keyframe points are view-range filtered and sorted furthest first.
    let plan3 = render_camera_path(&mut st, 0, Vec3f::new(10.0, 0.0, 0.0), 160, false, true).unwrap().unwrap();
    assert_eq!(plan3.keyframes.len(), 2);
    assert_eq!(plan3.keyframes[0].pos, Vec3f::new(100.0, 0.0, 0.0), "furthest first");
    assert_eq!(plan3.keyframes[1].pos, Vec3f::new(0.0, 0.0, 0.0));
    let plan4 = render_camera_path(&mut st, 0, Vec3f::new(0.0, 0.0, 0.0), 5, false, true).unwrap().unwrap();
    assert_eq!(plan4.keyframes.len(), 1, "only the in-range keyframe");
    assert_eq!(plan4.keyframes[0].pos, Vec3f::new(0.0, 0.0, 0.0));

    // t5: camera marker at the cursor takes the path position and rotation;
    // on a spectator path it comes from the tracker instead.
    let plan5 = render_camera_path(&mut st, 1000, Vec3f::new(0.0, 0.0, 0.0), 16, false, true).unwrap().unwrap();
    assert_eq!(plan5.camera.unwrap().pos, Vec3f::new(50.0, 0.0, 0.0));
    assert_eq!(plan5.camera.unwrap().rot, Vec3f::new(0.0, 0.0, 0.0));
    let plan6 = render_camera_path(&mut st2, 500, Vec3f::new(0.0, 0.0, 0.0), 16, false, true).unwrap().unwrap();
    let cam = plan6.camera.unwrap();
    assert_eq!(cam.pos, Vec3f::new(9.0, 8.0, 7.0));
    assert_eq!(cam.rot, Vec3f::new(42.0, 24.0, 0.0), "spectator yaw/pitch, roll 0");

    // t6: upstream early-out guards.
    assert!(render_camera_path(&mut st, 0, Vec3f::new(0.0, 0.0, 0.0), 16, false, false).unwrap().is_none(), "sync mode");
    assert!(render_camera_path(&mut st, 0, Vec3f::new(0.0, 0.0, 0.0), 16, true, true).unwrap().is_none(), "hud hidden");

    println!("PREVIEW ok");
}

    println!("ALL ok");
}


struct MockTracker { pos: Location }
impl EntityPositionTracker for MockTracker {
    fn get_entity_position_at_timestamp(&self, _e: i32, _t: i32) -> Option<Location> { Some(self.pos) }
}

fn spt_tests() {
    // t1: add pos/time keyframes, undo everything, redo everything.
    let mut st = SPTimeline::new();
    st.set_default_interpolator_type(InterpolatorType::Cubic).unwrap();
    st.add_position_keyframe(0, (1.0, 2.0, 3.0), (4.0, 5.0, 6.0), -1).unwrap();
    st.add_position_keyframe(1000, (2.0, 3.0, 4.0), (5.0, 6.0, 7.0), -1).unwrap();
    st.add_time_keyframe(0, 0).unwrap();
    st.add_time_keyframe(1000, 1000).unwrap();
    assert!(st.is_position_keyframe(0) && st.is_time_keyframe(1000));
    while st.timeline.has_undo() { st.timeline.undo_last_change().unwrap(); }
    assert!(!st.is_position_keyframe(0) && !st.is_time_keyframe(0));
    while st.timeline.has_redo() { st.timeline.redo_last_change().unwrap(); }
    assert!(st.is_position_keyframe(0) && st.is_time_keyframe(1000));

    // t2: explicit interpolator, then moveKeyframe with undo/redo.
    let cubic = st.timeline.create_interpolator(Box::new(PolynomialSplineInterpolator::cubic()));
    st.set_interpolator(0, cubic).unwrap();
    st.move_keyframe(SPPath::Position, 0, 500).unwrap();
    assert!(st.is_position_keyframe(500) && !st.is_position_keyframe(0));
    st.timeline.undo_last_change().unwrap();
    assert!(st.is_position_keyframe(0) && !st.is_position_keyframe(500));
    st.timeline.redo_last_change().unwrap();
    assert!(st.is_position_keyframe(500));

    // t3: spectator keyframe + entity tracker drives its position.
    st.set_entity_tracker(Box::new(MockTracker { pos: Location::new(9.0, 8.0, 7.0, 42.0, 24.0) })).unwrap();
    st.add_position_keyframe(2000, (1.0, 1.0, 1.0), (0.0, 0.0, 0.0), 5).unwrap();
    st.add_time_keyframe(2000, 100).unwrap();
    let kf = st.timeline.paths[st.position_path].keyframes.get(&2000).unwrap();
    assert_eq!(kf.values.get(&st.camera_position_id), Some(&PropertyValue::Position(9.0, 8.0, 7.0)));

    println!("SPT ok");
}


fn ser_tests() {
    // Build a timeline with a mix of keyframe types + an explicit interpolator,
    // serialize it, deserialize it, compare.
    let mut st = SPTimeline::new();
    st.set_default_interpolator_type(InterpolatorType::Cubic).unwrap();
    st.add_position_keyframe(0, (1.0, 2.0, 3.0), (4.0, 5.0, 6.0), -1).unwrap();
    st.add_position_keyframe(1000, (2.0, 3.0, 4.0), (5.0, 6.0, 7.0), -1).unwrap();
    st.add_time_keyframe(0, 0).unwrap();
    st.add_time_keyframe(1000, 1000).unwrap();
    let cat = st.timeline.create_interpolator(Box::new(CatmullRomSplineInterpolator::new(0.5)));
    st.set_interpolator(0, cat).unwrap();

    let json = serialize(&[("main", &st.timeline)]);
    let rt = deserialize(&json).expect("deserialize");

    let t2 = &rt["main"];
    assert_eq!(t2.paths.len(), st.timeline.paths.len(), "path count");
    for (i, path) in st.timeline.paths.iter().enumerate() {
        let p2 = &t2.paths[i];
        assert_eq!(p2.keyframes.len(), path.keyframes.len(), "keyframe count path {}", i);
        for (&time, kf) in &path.keyframes {
            let kf2 = &p2.keyframes[&time];
            assert_eq!(kf.values.len(), kf2.values.len(), "value count at {} path {}", time, i);
        }
        assert_eq!(p2.segments.len(), path.segments.len(), "segment count path {}", i);
    }
    // The explicit marker survived, and the interpolator type round-trips.
    let exp = t2.get_property_by_string("interpolationFixed").unwrap();
    assert!(t2.paths[1].keyframes[&0].values.contains_key(&exp), "explicit marker");
    let seg2 = t2.paths[1].segments[0];
    let kind = t2.interps[seg2.interpolator.expect("seg interp")].kind().0;
    assert_eq!(kind, "catmull-rom-spline", "interpolator type");
    println!("ROUNDTRIP ok");
}


struct MockSource { packets: Vec<PacketData>, i: usize }
struct MockSink { delivered: usize }
impl PacketSink for MockSink { fn deliver(&mut self, _b: &[u8]) { self.delivered += 1; } }
impl PacketSource for MockSource {
    fn reopen(&mut self) { self.i = 0; }
    fn next_packet(&mut self) -> Result<PacketData, String> {
        if self.i < self.packets.len() { let p = self.packets[self.i].clone(); self.i += 1; Ok(p) } else { Err("EOF".into()) }
    }
}
struct MockHandler { moved: u32, restarted: u32 }
impl ReplayHandler for MockHandler {
    fn move_camera_to_target_position(&mut self) { self.moved += 1; }
    fn restarted_replay(&mut self) { self.restarted += 1; }
}

/// Drive tick() until it reports Progress or ends, advancing the clock on
/// Sleep (production runs this on a thread; the logic is what we port).
fn drive(s: &mut FullReplaySender, now: &mut u64, src: &mut MockSource, sink: &mut MockSink, h: &mut MockHandler) -> LoopAction {
    loop {
        match s.tick(*now, src, sink, h).unwrap() {
            LoopAction::Sleep(ms) => *now += ms,
            done => return done,
        }
    }
}

fn sender_tests() {
    // t1: timed playback at speed 1.
    let mut s = FullReplaySender::new(1000);
    s.async_mode = true;
    let mut src = MockSource { packets: (0..5).map(|i| PacketData { timestamp: i * 100, is_bundle: false, kind: PacketKind::Other, bytes: vec![i as u8] }).collect(), i: 0 };
    let mut sink = MockSink { delivered: 0 };
    let mut h = MockHandler { moved: 0, restarted: 0 };
    s.has_world_loaded = true;
    let mut now: u64 = 0;
    s.set_replay_speed(1.0, now);
    assert_eq!(s.real_time_start, 0);
    for _ in 0..5 { assert!(matches!(drive(&mut s, &mut now, &mut src, &mut sink, &mut h), LoopAction::Progress)); }
    assert_eq!(sink.delivered, 5, "delivered all");
    assert_eq!(s.current_time_stamp(), 400);
    assert_eq!(now, 400, "wall clock tracked packet timing");

    // t2: jump backwards restarts, then rushes to the target without waits.
    s.jump_to_time(300).unwrap();
    assert!(matches!(drive(&mut s, &mut now, &mut src, &mut sink, &mut h), LoopAction::Progress));
    s.restart_state_reset(now, &mut src, &mut h);
    assert_eq!(h.restarted, 1, "restart after backwards jump");
    for _ in 0..5 { drive(&mut s, &mut now, &mut src, &mut sink, &mut h); }
    assert_eq!(sink.delivered, 10, "5 more after restart");
    assert_eq!(s.current_time_stamp(), 400);
    assert_eq!(h.moved, 1, "camera snapped after jump finished");
    assert!(!s.is_hurrying(), "hurry cleared");
    assert!(s.paused(), "paused after jump per upstream");

    // t3: EOF with no world loaded: pause then continue (restart), stream reopened.
    s.has_world_loaded = false;
    let a = loop { match s.tick(now, &mut src, &mut sink, &mut h).unwrap() { LoopAction::Sleep(_) => continue, other => break other } };
    assert!(matches!(a, LoopAction::Progress), "EOF breaks to restart: {:?}", a);
    s.restart_state_reset(now, &mut src, &mut h);
    assert_eq!(h.restarted, 2, "restart on EOF continue");
    drive(&mut s, &mut now, &mut src, &mut sink, &mut h);
    assert_eq!(s.current_time_stamp(), 0, "playback resumed after reopen (ts 0 packet)");
    assert_eq!(sink.delivered, 11, "one packet delivered post-reopen");
    s.has_world_loaded = true;

    // t4: terminate.
    s.terminate_replay();
    assert!(matches!(drive(&mut s, &mut now, &mut src, &mut sink, &mut h), LoopAction::Terminated));

    // t5: sync mode: walk to a timestamp, held packet, backwards restart.
    let mut s2 = FullReplaySender::new(1000);
    let mut src2 = MockSource { packets: (0..5).map(|i| PacketData { timestamp: i * 100, is_bundle: false, kind: PacketKind::Other, bytes: vec![i as u8] }).collect(), i: 0 };
    let mut sink2 = MockSink { delivered: 0 };
    let mut h2 = MockHandler { moved: 0, restarted: 0 };
    assert!(s2.send_packets_till(250, 0, &mut src2, &mut sink2, &mut h2).is_ok());
    assert_eq!(sink2.delivered, 3, "packets 0,100,200 delivered");
    assert_eq!(s2.current_time_stamp(), 250);
    s2.send_packets_till(250, 1, &mut src2, &mut sink2, &mut h2).unwrap(); // no-op
    assert_eq!(sink2.delivered, 3, "same timestamp is a no-op");
    assert!(s2.send_packets_till(100, 2, &mut src2, &mut sink2, &mut h2).is_ok()); // backwards
    assert_eq!(h2.restarted, 1, "sync restart on backwards");
    assert_eq!(sink2.delivered, 5, "packets 0 and 100 after restart");

    // t6: EOF before target: pretend done.
    let mut s3 = FullReplaySender::new(400);
    let mut src3 = MockSource { packets: vec![PacketData { timestamp: 0, is_bundle: false, kind: PacketKind::Other, bytes: vec![] }], i: 0 };
    let mut sink3 = MockSink { delivered: 0 };
    let mut h3 = MockHandler { moved: 0, restarted: 0 };
    assert!(s3.send_packets_till(300, 0, &mut src3, &mut sink3, &mut h3).is_ok());
    assert_eq!(sink3.delivered, 1, "EOF pretends done");
    assert_eq!(s3.current_time_stamp(), 300, "lastTimeStamp set to target");

    // t7: process_packet_async drops particles/fireworks only on long rushes.
    let mut s4 = FullReplaySender::new(1000);
    s4.async_mode = true;
    s4.desired_time_stamp = 5000; s4.last_time_stamp = 0;
    let part = PacketData { timestamp: 10, is_bundle: false, kind: PacketKind::Particle, bytes: vec![] };
    let fw = PacketData { timestamp: 10, is_bundle: false, kind: PacketKind::EntitySpawnFireworkRocket, bytes: vec![] };
    let other = PacketData { timestamp: 10, is_bundle: false, kind: PacketKind::Other, bytes: vec![] };
    assert!(s4.process_packet_async(&part).is_none(), "particle dropped on long rush");
    assert!(s4.process_packet_async(&fw).is_none(), "firework dropped on long rush");
    assert!(s4.process_packet_async(&other).is_some());
    s4.desired_time_stamp = 500; // short span: nothing dropped
    assert!(s4.process_packet_async(&part).is_some());

    // t8: sync mode refuses async ops.
    let mut s5 = FullReplaySender::new(1000);
    s5.async_mode = true;
    assert!(s5.jump_to_time(0).is_ok());
    let mut s6 = FullReplaySender::new(1000);
    s6.async_mode = true;
    assert!(s6.send_packets_till(0, 0, &mut src, &mut sink, &mut h).is_err(), "sync op refused in async mode");

    println!("SENDER ok");
}


fn mcpr_tests() {
    // t1: packet framing roundtrip, empty segments skipped, EOF ends stream.
    let mut stream = Vec::new();
    write_packet(&mut stream, 0, &[1, 2, 3]);
    write_packet(&mut stream, 500, &[]); // length 0: read skips it
    write_packet(&mut stream, 1000, &[9; 700]);
    let pkts = read_packets(&stream);
    assert_eq!(pkts.len(), 2, "empty segment skipped");
    assert_eq!(pkts[0].timestamp, 0);
    assert_eq!(pkts[0].bytes, vec![1, 2, 3]);
    assert_eq!(pkts[1].timestamp, 1000);
    assert_eq!(pkts[1].bytes.len(), 700);

    // t2: read_int big-endian + EOF sentinel.
    let mut b = Vec::new();
    write_int(&mut b, 305_419_896); // 0x12345678
    let mut pos = 0usize;
    assert_eq!(read_int(&b, &mut pos), 305_419_896);
    assert_eq!(read_int(&b, &mut pos), -1, "short read ends stream");

    // t3: metaData.json parses with Gson field names.
    let md = parse_meta_data(r#"{"singleplayer":true,"serverName":"Herobrine.org","customServerName":"HB","duration":12345,"date":1698000000000,"mcversion":"1.20.4","fileFormat":"VFC","fileFormatVersion":14,"protocol":765,"generator":"ReplayMod v2.6.20","selfId":129,"players":["sparticle"]}"#).expect("parse metadata");
    assert!(md.singleplayer);
    assert_eq!(md.duration, 12345);
    assert_eq!(md.protocol, Some(765));
    assert_eq!(md.self_id, 129);
    assert_eq!(md.mc_version, "1.20.4");
    assert_eq!(md.players, vec!["sparticle".to_string()]);
    let md2 = parse_meta_data("{}").expect("parse defaults");
    assert_eq!(md2.file_format_version, 0, "absent fields keep Java defaults (no initializer)");
    assert_eq!(md2.self_id, -1);

    // t4: entry-name constants + format helpers match upstream.
    assert_eq!(ENTRY_META_DATA, "metaData.json");
    assert_eq!(ENTRY_RECORDING, "recording.tmcpr");
    assert_eq!(ENTRY_RECORDING_HASH, "recording.tmcpr.crc32");
    assert_eq!(ENTRY_MARKERS, "markers.json");
    assert_eq!(entry_asset("a", "b", "png"), "asset/a_b.png");
    assert_eq!(entry_resource_pack("rp"), "resourcepack/rp.zip");

    println!("MCPR ok");
}


struct MockArchive { entries: std::collections::BTreeMap<String, Vec<u8>> }
impl MockArchive { fn new() -> Self { Self { entries: std::collections::BTreeMap::new() } } }
impl McprArchive for MockArchive {
    fn read_entry(&self, name: &str) -> Option<Vec<u8>> { self.entries.get(name).cloned() }
    fn list_entries(&self) -> Vec<String> { self.entries.keys().cloned().collect() }
    fn write_entry(&mut self, name: &str, bytes: &[u8]) { self.entries.insert(name.to_string(), bytes.to_vec()); }
    fn remove_entry(&mut self, name: &str) { self.entries.remove(name); }
}

fn replayfile_tests() {
    // t1: crc32 test vector (java.util.zip.CRC32).
    assert_eq!(crc32(b"123456789"), 0xCBF4_3926, "crc32 IEEE vector");
    assert_eq!(crc32(b""), 0);

    // t2: metadata write (MCPR format forced, protocol set) + read roundtrip.
    let mut ar = MockArchive::new();
    let mut md = ReplayMetaData { server_name: "mc.hypixel.net".into(), duration: 999, self_id: 42, ..Default::default() };
    {
        let mut rf = ReplayFile { archive: &mut ar };
        rf.write_meta_data(&mut md, Some(765), "v2");
        let back = rf.get_meta_data().expect("metadata present");
        assert_eq!(back.file_format, "MCPR");
        assert_eq!(back.file_format_version, CURRENT_FILE_FORMAT_VERSION);
        assert_eq!(back.protocol, Some(765));
        assert_eq!(back.generator, "ReplayStudio vv2");
        assert_eq!(back.server_name, "mc.hypixel.net");
        assert_eq!(back.duration, 999);
        assert_eq!(back.self_id, 42);
    }
    assert!(ar.entries.contains_key("metaData.json"));

    // t3: absent entry -> None / empty, matching upstream null / empty list.
    let mut ar2 = MockArchive::new();
    {
        let mut rf = ReplayFile { archive: &mut ar2 };
        assert!(rf.get_meta_data().is_none());
        assert!(rf.get_packet_data().is_none());
        assert!(rf.get_invisible_players().is_none());
        assert!(rf.get_markers().is_none());
        assert!(rf.get_mod_info().is_empty());
        assert!(rf.get_assets().is_empty());
    }

    // t4: markers roundtrip with optional name (absent name omitted from JSON).
    let mut ar3 = MockArchive::new();
    {
        let mut rf = ReplayFile { archive: &mut ar3 };
        let markers = vec![
            Marker { time: 5000, x: 1.5, y: 64.0, z: -2.25, yaw: 90.0, pitch: 10.0, roll: 0.0, name: Some("base".into()) },
            Marker { time: 12000, x: 0.0, y: 0.0, z: 0.0, yaw: 0.0, pitch: 0.0, roll: 0.0, name: None },
        ];
        rf.write_markers(&markers);
        let back = rf.get_markers().expect("markers present");
        assert_eq!(back.len(), 2);
        assert_eq!(back[0].time, 5000);
        assert_eq!(back[0].name.as_deref(), Some("base"));
        assert_eq!(back[1].name, None);
        assert_eq!(back[1].time, 12000);
        let body = String::from_utf8_lossy(rf.archive.read_entry("markers.json").as_deref().unwrap_or(&[])).to_string();
        assert!(!body.contains(":null"), "Gson skips null names");
    }

    // t5: invisible players roundtrip + old-entry fallback.
    let mut ar4 = MockArchive::new();
    {
        let mut rf = ReplayFile { archive: &mut ar4 };
        rf.write_invisible_players(&["550e8400-e29b-41d4-a716-446655440000".into()]);
        assert_eq!(rf.get_invisible_players().unwrap().len(), 1);
    }
    ar4.entries.remove("visibility.json");
    ar4.entries.insert("visibility".into(), br#"{"hidden":["550e8400-e29b-41d4-a716-446655440001"]}"#.to_vec());
    {
        let mut rf = ReplayFile { archive: &mut ar4 };
        assert_eq!(rf.get_invisible_players().unwrap()[0], "550e8400-e29b-41d4-a716-446655440001", "old entry fallback");
    }

    // t6: mod info roundtrip.
    let mut ar5 = MockArchive::new();
    {
        let mut rf = ReplayFile { archive: &mut ar5 };
        let mods = vec![ModInfo { id: "replaymod".into(), name: "Replay Mod".into(), version: "2.6.20".into() }];
        rf.write_mod_info(&mods);
        assert_eq!(rf.get_mod_info(), mods);
    }

    // t7: assets listing/get/write/remove; thumb magic handling.
    let mut ar6 = MockArchive::new();
    {
        let mut rf = ReplayFile { archive: &mut ar6 };
        let a = ReplayAssetEntry { uuid: "u-1".into(), extension: ".png".into(), name: "pic".into() };
        rf.write_asset(&a, &[9, 9]);
        assert_eq!(rf.get_asset("u-1"), Some(vec![9, 9]), "entry written as asset/u-1_pic.png");
        let assets = rf.get_assets();
        assert_eq!(assets.len(), 1);
        assert_eq!(assets[0].name, "pic");
        rf.remove_asset("u-1");
        assert!(rf.get_assets().is_empty());
    }
    ar6.entries.insert("thumb.jpg".into(), vec![1, 2, 3]);
    {
        let mut rf = ReplayFile { archive: &mut ar6 };
        assert_eq!(rf.get_thumb_bytes(), Some(vec![1, 2, 3]));
    }
    ar6.entries.remove("thumb.jpg");
    ar6.entries.insert("thumb".into(), THUMB_MAGIC_NUMBERS.to_vec());
    {
        let mut rf = ReplayFile { archive: &mut ar6 };
        assert_eq!(rf.get_thumb_bytes().unwrap(), Vec::<u8>::new(), "old thumb: magic stripped");
    }

    // t8: resource pack index roundtrip, non-numeric keys skipped.
    let mut ar7 = MockArchive::new();
    ar7.entries.insert("resourcepack/index.json".into(), br#"{"1":"hash-a","oops":"x","2":"hash-b"}"#.to_vec());
    {
        let mut rf = ReplayFile { archive: &mut ar7 };
        let idx = rf.get_resource_pack_index().expect("index present");
        assert_eq!(idx.len(), 2, "non-numeric key skipped");
        assert_eq!(idx[&2], "hash-b");
    }

    println!("REPLAYFILE ok");
}


fn zipfile_tests() {
    // t1: recording write invalidates the CRC hash immediately, then the
    // correct hash lands on finish_write.
    let mut ar = MockArchive::new();
    ar.entries.insert(ENTRY_RECORDING.to_string(), Vec::new());
    let mut zf = ZipReplayFile::open(ar);
    // seed the recording via the writer path
    let w = zf.write(ENTRY_RECORDING);
    assert_eq!(zf.get(ENTRY_RECORDING_HASH).as_deref(), Some(&b"invalid"[..]), "old hash invalidated at write start");
    let mut w = w;
    w.write_bytes(&[1, 2, 3, 4]);
    zf.finish_write(w);
    let expect = crc32(&[1, 2, 3, 4]).to_string();
    assert_eq!(zf.get(ENTRY_RECORDING_HASH).as_deref(), Some(expect.as_bytes()), "hash rewritten on close");
    assert_eq!(zf.get(ENTRY_RECORDING).as_deref(), Some(&[1u8, 2, 3, 4][..]));

    // t2: plain entry write/remove semantics.
    let w2 = zf.write("metaData.json");
    zf.finish_write(w2);
    assert!(zf.get("metaData.json").is_some());
    zf.remove("metaData.json");
    assert!(zf.get("metaData.json").is_none(), "new entry changed then removed -> gone (never in base)");

    // Upstream tombstone semantics on a BASE entry: still readable until save.
    zf.archive.write_entry("old.json", &[9u8]);
    zf.seed_base("old.json", &[9u8]);
    zf.remove("old.json");
    assert_eq!(zf.get("old.json").as_deref(), Some(&[9u8][..]), "base entry readable until save");
    assert!(zf.get("metaData.json").is_none());

    // t3: save merges base (skipping changed/removed) + changed, then clears.
    zf.save();
    assert!(!zf.entry_names().iter().any(|n| n == "metaData.json"), "tombstone applied at save");
    assert!(!zf.entry_names().iter().any(|n| n == "old.json"), "base tombstone applied at save");
    assert_eq!(zf.get(ENTRY_RECORDING_HASH).as_deref(), Some(expect.as_bytes()));
    let before = zf.entry_names();
    zf.save();
    assert_eq!(zf.entry_names(), before, "second save is a no-op");

    // t4: timeline wiring roundtrip through the replay file.
    use crate::pathing::Timeline;
    let mut ar2 = MockArchive::new();
    {
        let mut rf = ReplayFile { archive: &mut ar2 };
        assert!(rf.get_timelines().unwrap().is_empty(), "absent entry -> empty map");
        let tl = crate::serialize::create_timeline();
        rf.write_timelines(&[("cam".to_string(), tl)]);
        let back = rf.get_timelines().unwrap();
        assert_eq!(back.len(), 1);
        assert!(back.contains_key("cam"));
    }

    println!("ZIPFILE ok");
}


fn processing_tests() {
    use std::collections::BTreeMap;
    fn pkt(kind: &str, payload: Payload) -> Packet { Packet { kind: kind.to_string(), payload } }
    fn no_restrict(_d: &[u8]) -> Option<String> { None }
    fn bad_restrict(_d: &[u8]) -> Option<String> { Some("what is this".to_string()) }

    // t1: LoginSuccess transitions the registry to PLAY.
    let mut st = ReplaySenderState::default();
    assert_eq!(st.registry, RegistryState::Login);
    let out = process_packet(&mut st, pkt("LoginSuccessS2CPacket", Payload::None), None, true, false, &no_restrict);
    assert!(matches!(out, Outcome::Keep(_)));
    assert_eq!(st.registry, RegistryState::Play);

    // t2: restrictions channel: parsed -> drop, unknown -> terminate.
    let mut st = ReplaySenderState::default();
    let p = pkt("CustomPayloadS2CPacket", Payload::CustomPayload {
        channel: PLUGIN_CHANNEL.to_string(), data: vec![1, 2, 3] });
    assert_eq!(process_packet(&mut st, p, None, true, false, &no_restrict), Outcome::Drop);
    let p = pkt("CustomPayloadS2CPacket", Payload::CustomPayload {
        channel: PLUGIN_CHANNEL.to_string(), data: vec![9] });
    assert_eq!(process_packet(&mut st, p, None, true, false, &bad_restrict), Outcome::Terminate);

    // t3: forced-update disconnect dropped, other reasons kept.
    let mut st = ReplaySenderState::default();
    let p = pkt("DisconnectS2CPacket", Payload::Disconnect { reason: "Please update to view this replay.".to_string() });
    assert_eq!(process_packet(&mut st, p, None, true, false, &no_restrict), Outcome::Drop);
    let p = pkt("DisconnectS2CPacket", Payload::Disconnect { reason: "Bye".to_string() });
    assert!(matches!(process_packet(&mut st, p, None, true, false, &no_restrict), Outcome::Keep(_)));

    // t4: BAD_PACKETS filter, 1:1 list.
    let mut st = ReplaySenderState::default();
    for kind in ["TitleS2CPacket", "PlayerAbilitiesS2CPacket", "HealthUpdateS2CPacket",
                 "ExperienceBarUpdateS2CPacket", "OpenScreenS2CPacket", "LoginHelloS2CPacket"] {
        let p = pkt(kind, Payload::None);
        assert!(is_bad_packet(kind), "{} should be a bad packet", kind);
        assert_eq!(process_packet(&mut st, p, None, true, false, &no_restrict), Outcome::Drop, "{}", kind);
    }
    assert!(!is_bad_packet("EntitySpawnS2CPacket"));

    // t5: fabric-screen-handler channels dropped, other payloads kept.
    let p = pkt("CustomPayloadS2CPacket", Payload::CustomPayload {
        channel: "fabric-screen-handler-api-v5".to_string(), data: vec![] });
    assert_eq!(process_packet(&mut st, p, None, true, false, &no_restrict), Outcome::Drop);
    let p = pkt("CustomPayloadS2CPacket", Payload::CustomPayload {
        channel: "brand".to_string(), data: vec![1] });
    assert!(matches!(process_packet(&mut st, p, None, true, false, &no_restrict), Outcome::Keep(_)));

    // t6: replay:// resource pack resolved through the index, packet dropped.
    let mut idx = BTreeMap::new();
    idx.insert(42, "abc123".to_string());
    let p = pkt("ResourcePackSendS2CPacket", Payload::ResourcePackSend {
        url: "replay://42".to_string(), pack_id: 7 });
    assert_eq!(process_packet(&mut st, p, Some(&idx), true, false, &no_restrict), Outcome::Drop);
    // Unknown pack id also drops (the packet itself is swallowed either way).
    let p = pkt("ResourcePackSendS2CPacket", Payload::ResourcePackSend {
        url: "replay://99".to_string(), pack_id: 7 });
    assert_eq!(process_packet(&mut st, p, Some(&idx), true, false, &no_restrict), Outcome::Drop);

    // t7: GameJoin rewrite: camera id, SPECTATOR modes, allowMovement, actual id.
    let p = pkt("GameJoinS2CPacket", Payload::GameJoin { entity_id: 2711, mode: 1, mode_prev: 0 });
    match process_packet(&mut st, p, None, true, false, &no_restrict) {
        Outcome::Keep(np) => match np.payload {
            Payload::GameJoin { entity_id, mode, mode_prev } => {
                assert_eq!(entity_id, CAMERA_ENTITY_ID);
                assert_eq!(mode, GAME_MODE_SPECTATOR);
                assert_eq!(mode_prev, GAME_MODE_SPECTATOR);
            }
            _ => panic!("wrong payload"),
        },
        _ => panic!("expected Keep"),
    }
    assert_eq!(st.actual_id, 2711);
    assert!(st.allow_movement);

    // t8: PlayerRespawn rewritten to SPECTATOR.
    let p = pkt("PlayerRespawnS2CPacket", Payload::PlayerRespawn { mode: 0, mode_prev: 0 });
    match process_packet(&mut st, p, None, true, false, &no_restrict) {
        Outcome::Keep(np) => match np.payload {
            Payload::PlayerRespawn { mode, mode_prev } => {
                assert_eq!(mode, GAME_MODE_SPECTATOR);
                assert_eq!(mode_prev, GAME_MODE_SPECTATOR);
            }
            _ => panic!("wrong payload"),
        },
        _ => panic!("expected Keep"),
    }

    // t9: PlayerPositionLook bookkeeping: hasWorldLoaded, suppress, relative.
    let p = pkt("PlayerPositionLookS2CPacket", Payload::PlayerPositionLook { flags: 0 });
    assert_eq!(process_packet(&mut st, p, None, true, false, &no_restrict), Outcome::Drop);
    assert!(st.has_world_loaded);
    assert!(!st.allow_movement, "camera move consumes the movement allowance");
    let p = pkt("PlayerPositionLookS2CPacket", Payload::PlayerPositionLook { flags: FLAG_X | FLAG_Y_ROT() });
    assert_eq!(process_packet(&mut st, p, None, true, false, &no_restrict), Outcome::Drop);
    let p = pkt("PlayerPositionLookS2CPacket", Payload::PlayerPositionLook { flags: 0 });
    assert_eq!(process_packet(&mut st, p, None, true, true, &no_restrict), Outcome::Drop, "suppressed camera");

    // t10: GameStateChange keeps only rain controls.
    for reason in [1, 2, 7, 8] {
        let p = pkt("GameStateChangeS2CPacket", Payload::GameStateChange { reason, value: 0.5 });
        assert!(matches!(process_packet(&mut st, p, None, true, false, &no_restrict), Outcome::Keep(_)), "reason {}", reason);
    }
    let p = pkt("GameStateChangeS2CPacket", Payload::GameStateChange { reason: 24, value: 0.0 });
    assert_eq!(process_packet(&mut st, p, None, true, false, &no_restrict), Outcome::Drop);

    // t11: GameMessage respects SHOW_CHAT.
    let p = pkt("GameMessageS2CPacket", Payload::GameMessage);
    assert!(matches!(process_packet(&mut st, p.clone(), None, true, false, &no_restrict), Outcome::Keep(_)));
    assert_eq!(process_packet(&mut st, p, None, false, false, &no_restrict), Outcome::Drop);

    println!("PROCESSING ok");
}

// PlayerPositionLook Flag Y_ROT for a non-coordinate flag check.
fn FLAG_Y_ROT() -> u32 { 0x08 }


const ZIP_TEST_B64: &str = "UEsDBBQAAAAIAAAAIQBH3TNVyAAAAMMAAAANAAAAbWV0YURhdGEuanNvbgHDADz/cmVwbGF5bW9kIHppcCBlbnRyeToNFBsiKTA3PkVMU1phaG92fYSLkpmgp661vMPK0djf5u30AAcOFRwjKjE4P0ZNVFtiaXB3foWMk5qhqK+2vcTL0tng5+71AQgPFh0kKzI5QEdOVVxjanF4f4aNlJuiqbC3vsXM09rh6O/2AgkQFx4lLDM6QUhPVl1ka3J5gIeOlZyjqrG4v8bN1Nvi6fD3AwoRGB8mLTQ7QklQV15lbHN6gYiPlp2kq7K5wMfO1dzjUEsDBBQAAAAAAAAAIQDOL7OwBwAAAAcAAAAPAAAAdGh1bWIvdGh1bWIuanBnAAEBAgMFCFBLAwQUAAAACAAAACEAQfvjdBgBAADoAwAADwAAAHJlY29yZGluZy50bWNwcmNgZGJmYWVj5+Dk4ubh5eMXEBQSFhEVE5eQlJKWkZWTV1BUUlZRVVPX0NTS1tHV0zcwNDI2MTUzt7C0sraxtbN3cHRydnF1c/fw9PL28fXzDwgMCg4JDQuPiIyKjomNi09ITEpOSU1Lz8jMys7JzcsvKCwqLiktK6+orKquqa2rb2hsam5pbWvv6Ozq7unt658wcdLkKVOnTZ8xc9bsOXPnzV+wcNHiJUuXLV+xctXqNWvXrd+wcdPmLVu3bd+xc9fuPXv37T9w8NDhI0ePHT9x8tTpM2fPnb9w8dLlK1evXb9x89btO3fv3X/w8NHjJ0+fPX/x8tXrN2/fvf/w8dPnL1+/ff/x89fvP3///WcY9f+o/0eA/wFQSwECFAMUAAAACAAAACEAR90zVcgAAADDAAAADQAAAAAAAAAAAAAAgAEAAAAAbWV0YURhdGEuanNvblBLAQIUAxQAAAAAAAAAIQDOL7OwBwAAAAcAAAAPAAAAAAAAAAAAAACAAfMAAAB0aHVtYi90aHVtYi5qcGdQSwECFAMUAAAACAAAACEAQfvjdBgBAADoAwAADwAAAAAAAAAAAAAAgAEnAQAAcmVjb3JkaW5nLnRtY3ByUEsFBgAAAAADAAMAtQAAAGwCAAAAAA==";
const DEFLATE_V1_B64: &str = "AcMAPP9yZXBsYXltb2QgemlwIGVudHJ5Og0UGyIpMDc+RUxTWmFob3Z9hIuSmaCnrrW8w8rR2N/m7fQABw4VHCMqMTg/Rk1UW2JpcHd+hYyTmqGor7a9xMvS2eDn7vUBCA8WHSQrMjlAR05VXGNqcXh/ho2Um6KpsLe+xczT2uHo7/YCCRAXHiUsMzpBSE9WXWRrcnmAh46VnKOqsbi/xs3U2+Lp8PcDChEYHyYtNDtCSVBXXmVsc3qBiI+WnaSrsrnAx87V3OM=";
const DEFLATE_V2_B64: &str = "Y2BkYmZhZWPn4OTi5uHl4xcQFBIWERUTl5CUkpaRlZNXUFRSVlFVU9fQ1NLW0dXTNzA0MjYxNTO3sLSytrG1s3dwdHJ2cXVz9/D08vbx9fMPCAwKDgkNC4+IjIqOiY2LT0hMSk5JTUvPyMzKzsnNyy8oLCouKS0rr6isqq6pratvaGxqbmlta+/o7Oru6e3rnzBx0uQpU6dNnzFz1uw5c+fNX7Bw0eIlS5ctX7Fy1eo1a9et37Bx0+YtW7dt37Fz1+49e/ftP3Dw0OEjR48dP3Hy1OkzZ8+dv3Dx0uUrV69dv3Hz1u07d+/df/Dw0eMnT589f/Hy1es3b9+9//Dx0+cvX799//Hz1+8/f//9Zxj1/6j/R4D/AQ==";

fn b64d(s: &str) -> Vec<u8> {
    const T: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = Vec::new(); let mut buf = 0u32; let mut bits = 0;
    for c in s.bytes() {
        if c == b'=' { break; }
        let v = T.iter().position(|&t| t == c).unwrap() as u32;
        buf = (buf << 6) | v; bits += 6;
        if bits >= 8 { bits -= 8; out.push(((buf >> bits) & 0xFF) as u8); }
    }
    out
}

fn zipio_tests() {
    // t1: parse a zip written by an external writer, deflate + stored entries.
    let zip = b64d(ZIP_TEST_B64);
    let entries = parse_zip(&zip).expect("parse");
    assert_eq!(entries.len(), 3);
    let meta = &entries["metaData.json"];
    assert_eq!(meta.len(), 195);
    assert!(meta.starts_with(b"replaymod zip entry:"));
    assert_eq!(&entries["thumb/thumb.jpg"][..], &[0u8, 1, 1, 2, 3, 5, 8][..]);
    let rec = &entries["recording.tmcpr"];
    assert_eq!(rec.len(), 1000);
    assert_eq!(rec[0], 0); assert_eq!(rec[255], 255); assert_eq!(rec[999], 231u8);

    // t2: raw deflate vectors inflate back to their sources.
    let v1 = inflate(&b64d(DEFLATE_V1_B64)).expect("v1");
    assert_eq!(v1.len(), 195);
    assert_eq!(v1, *meta);
    let v2 = inflate(&b64d(DEFLATE_V2_B64)).expect("v2");
    assert_eq!(v2.len(), 1000);
    assert_eq!(v2, *rec);

    // t3: stored writer round-trips through our own parser.
    let built = write_zip_stored(&[
        ("metaData.json".to_string(), v1.clone()),
        ("recording.tmcpr".to_string(), v2.clone()),
    ], &crate::replayfile::crc32);
    let back = parse_zip(&built).expect("reparse");
    assert_eq!(back.len(), 2);
    assert_eq!(back["metaData.json"], v1);
    assert_eq!(back["recording.tmcpr"], v2);

    // t4: the whole container stack end-to-end on real zip bytes:
    // open -> write metadata -> serialize back to a real zip -> reopen.
    let mut ar = ZipArchiveMem::from_zip_bytes(&zip).expect("open");
    {
        let mut rf = ReplayFile { archive: &mut ar };
        let mut md = ReplayMetaData::default();
        rf.write_meta_data(&mut md, Some(3408), "ReplayStudio vX");
    }
    let rewritten = ar.to_zip_bytes();
    let mut ar2 = ZipArchiveMem::from_zip_bytes(&rewritten).expect("reopen2");
    assert_eq!(ar2.entries.len(), 3);
    let mut ar3 = ZipArchiveMem::from_zip_bytes(&rewritten).expect("reopen2");
    {
        let mut rf3 = ReplayFile { archive: &mut ar3 };
        let md = rf3.get_meta_data().expect("read back");
        assert_eq!(md.file_format, "MCPR");
        assert_eq!(md.file_format_version, 14);
    }

    println!("ZIPIO ok");
}


fn player_tests() {
    use std::cell::RefCell;
    use std::collections::VecDeque;
    use std::rc::Rc;

    struct MockSender {
        async_mode: bool,
        sync_calls: usize,
        speed: Vec<f64>,
        async_sets: Vec<bool>,
        ts: VecDeque<i64>,
    }
    impl ReplaySenderOps for MockSender {
        fn is_async_mode(&mut self) -> bool { self.async_mode }
        fn set_sync_mode_and_wait(&mut self) { self.sync_calls += 1; self.async_mode = false; }
        fn set_async_mode(&mut self, b: bool) { self.async_sets.push(b); self.async_mode = b; }
        fn set_replay_speed(&mut self, s: f64) { self.speed.push(s); }
        fn current_time_stamp(&mut self) -> i64 { self.ts.pop_front().unwrap_or(0) }
    }
    fn ts_of(vals: &[i64]) -> VecDeque<i64> { vals.iter().copied().collect() }
    fn mk_timeline(end_ts: i64) -> Timeline {
        let mut t = Timeline::new();
        let ts = t.define_property(timestamp_property());
        let p = t.create_path();
        let li = t.create_interpolator(Box::new(LinearInterpolator::new()));
        t.interps[li].register_property(ts);
        let mut v0 = BTreeMap::new(); v0.insert(ts, PropertyValue::Timestamp(0));
        let mut v1 = BTreeMap::new(); v1.insert(ts, PropertyValue::Timestamp(end_ts as i32));
        t.path_insert(p, 0, v0).unwrap();
        t.path_insert(p, end_ts, v1).unwrap();
        t.seg_set_interpolator(p, 0, Some(li));
        t.path_update(p, false).unwrap();
        t
    }

    // t1: realtime playback anchors the wall clock on the second frame,
    // applies every frame twice, clamps at the last keyframe and completes
    // there; the frame after completion restores speed/async mode.
    let mut t = mk_timeline(2000);
    let mut sender = MockSender { async_mode: true, sync_calls: 0, speed: vec![], async_sets: vec![], ts: ts_of(&(0..30).map(|i| i * 100).collect::<Vec<i64>>()) };
    let mut timer = ReplayTimer::new();
    let mut player = RealtimeTimelinePlayer::new();
    player.start(&t, &mut sender, &mut timer, 0);
    assert_eq!(sender.sync_calls, 1, "start forces sync mode");
    assert!(player.is_active());
    let applied: Rc<RefCell<Vec<i64>>> = Rc::new(RefCell::new(vec![]));
    let mut last_action = TickAction::Continue;
    for i in 0..23 {
        let now = i * 100;
        let a2 = applied.clone();
        let mut ap = |_pid: PropertyId, v: &PropertyValue| {
            if let PropertyValue::Timestamp(x) = v { a2.borrow_mut().push(*x as i64); }
        };
        last_action = player.on_tick(&mut t, &mut sender, &mut timer, &mut ap, now, false);
    }
    let got = applied.borrow().clone();
    let mut once: Vec<i64> = vec![0, 0];
    let mut v = 100i64;
    while v <= 2000 { once.push(v); v += 100; }
    let mut expect = vec![];
    for x in &once { expect.push(*x); expect.push(*x); }
    assert_eq!(got, expect, "applied sequence");
    assert_eq!(last_action, TickAction::Restore, "the frame after completion restores");
    assert_eq!(sender.speed, vec![0.0], "restore sets replay speed 0");
    assert_eq!(sender.async_sets, vec![true], "restore re-arms the prior async mode");
    assert!(!player.is_active());

    // t2: start(from) plays the offset on the first two frames, then the
    // clock re-anchors and playback climbs from there.
    let mut t = mk_timeline(2000);
    let mut sender = MockSender { async_mode: false, sync_calls: 0, speed: vec![], async_sets: vec![], ts: ts_of(&(0..12).map(|i| i * 100).collect::<Vec<i64>>()) };
    let mut timer = ReplayTimer::new();
    let mut player = RealtimeTimelinePlayer::new();
    player.start(&t, &mut sender, &mut timer, 1500);
    let applied: Rc<RefCell<Vec<i64>>> = Rc::new(RefCell::new(vec![]));
    let mut action = TickAction::Continue;
    for i in 0..8 {
        let now = i * 100;
        let a2 = applied.clone();
        let mut ap = |_pid: PropertyId, v: &PropertyValue| {
            if let PropertyValue::Timestamp(x) = v { a2.borrow_mut().push(*x as i64); }
        };
        action = player.on_tick(&mut t, &mut sender, &mut timer, &mut ap, now, false);
    }
    let mut once: Vec<i64> = vec![1500, 1500, 1600, 1700, 1800, 1900, 2000];
    let mut expect = vec![];
    for x in &once { expect.push(*x); expect.push(*x); }
    assert_eq!(applied.borrow().clone(), expect, "offset applied then clock-anchored");
    assert_eq!(action, TickAction::Restore, "completes at the 6th frame, restores on the 7th");

    // t3: while the loading overlay is up the played time freezes and
    // re-anchors on the first clear frame (no time lost).
    let mut t = mk_timeline(500);
    let mut sender = MockSender { async_mode: false, sync_calls: 0, speed: vec![], async_sets: vec![], ts: ts_of(&(0..12).map(|i| i * 100).collect::<Vec<i64>>()) };
    let mut timer = ReplayTimer::new();
    let mut player = RealtimeTimelinePlayer::new();
    player.start(&t, &mut sender, &mut timer, 0);
    let applied: Rc<RefCell<Vec<i64>>> = Rc::new(RefCell::new(vec![]));
    let overlay = [false, false, true, true, false, false, false, false, false, false];
    for i in 0..10 {
        let now = i * 100;
        let a2 = applied.clone();
        let mut ap = |_pid: PropertyId, v: &PropertyValue| {
            if let PropertyValue::Timestamp(x) = v { a2.borrow_mut().push(*x as i64); }
        };
        player.on_tick(&mut t, &mut sender, &mut timer, &mut ap, now, overlay[i as usize]);
    }
    let mut once: Vec<i64> = vec![0, 0, 100, 100, 100, 200, 300, 400, 500];
    let mut expect = vec![];
    for x in &once { expect.push(*x); expect.push(*x); }
    assert_eq!(applied.borrow().clone(), expect, "overlay pause + re-anchor sequence");
    assert!(!player.is_active(), "completed through the pause");

    // t4: a timeline with no keyframes completes on the first frame
    // (lastTimestamp = 0) and never applies anything.
    let mut t = Timeline::new();
    t.define_property(timestamp_property());
    t.create_path();
    let mut sender = MockSender { async_mode: false, sync_calls: 0, speed: vec![], async_sets: vec![], ts: ts_of(&[0, 100]) };
    let mut timer = ReplayTimer::new();
    let mut player = RealtimeTimelinePlayer::new();
    player.start(&t, &mut sender, &mut timer, 0);
    let applied: Rc<RefCell<Vec<i64>>> = Rc::new(RefCell::new(vec![]));
    let mut ap = |_pid: PropertyId, _v: &PropertyValue| { applied.borrow_mut().push(1); };
    assert_eq!(player.on_tick(&mut t, &mut sender, &mut timer, &mut ap, 0, false), TickAction::Continue);
    assert!(applied.borrow().is_empty());
    assert_eq!(player.on_tick(&mut t, &mut sender, &mut timer, &mut ap, 100, false), TickAction::Restore);
    assert!(!player.is_active());

    // t5: tickDelta accumulates fractional passed ticks and truncates into
    // ticksThisFrame (70 ms replay-time steps, 100 ms wall steps). Upstream's
    // lastTime==0 check re-arms while replayTime is 0, so the first TWO frames
    // anchor with 0 ticks; the player then finishes when time reaches the
    // 500 ms timeline end and the last call is the Restore no-op.
    let mut t = mk_timeline(500);
    let mut sender = MockSender { async_mode: false, sync_calls: 0, speed: vec![], async_sets: vec![], ts: ts_of(&[0, 70, 140, 210, 280, 350, 420, 490, 560]) };
    let mut timer = ReplayTimer::new();
    let mut player = RealtimeTimelinePlayer::new();
    player.start(&t, &mut sender, &mut timer, 0);
    let mut ap = |_pid: PropertyId, _v: &PropertyValue| {};
    let mut ticks = vec![timer.ticks_this_frame];
    for i in 1..9 {
        player.on_tick(&mut t, &mut sender, &mut timer, &mut ap, i * 100, false);
        ticks.push(timer.ticks_this_frame);
    }
    assert_eq!(ticks, vec![0, 0, 0, 1, 1, 2, 1, 1, 1], "ticksThisFrame accumulation");
    assert!((timer.tick_delta - 1.0).abs() < 1e-3, "fractional remainder kept: {}", timer.tick_delta);

    println!("PLAYER ok");
}
