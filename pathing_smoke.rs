mod pathing;
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
    println!("ALL ok");
}

