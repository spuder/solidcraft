#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
use crate::Session;
use serde_json::{Value, json};
use solidcraft_doc::construct::ConstructGeom;
use solidcraft_doc::{Document, FeatureKind, PlaneRef};
use solidcraft_geom::Plane;

fn run(s: &mut Session, command: &str, params: Value) -> Value {
    s.execute(command, &params).unwrap_or_else(|e| panic!("{command} {params}: {e}"))
}

fn plane(s: &Session, name: &str) -> Plane {
    let state = s.model.state();
    match state.construct.iter().find(|c| c.name == name).map(|c| &c.geom) {
        Some(ConstructGeom::Plane(p)) => *p,
        other => panic!("construction plane `{name}`: {other:?}"),
    }
}

fn volume(s: &Session) -> f64 {
    s.model.state().bodies.iter().map(|b| solidcraft_kernel::measure(&b.body).unwrap().volume).sum()
}

#[test]
fn offset_plane_from_a_planar_face_follows_the_face() {
    let mut s = Session::default();
    run(&mut s, "solid.box", json!({"length": 40, "width": 30, "height": 20, "name": "Base"}));

    // Top face (z = 20), 5 mm out along its normal.
    run(&mut s, "construct.plane.offset", json!({"base": {"face": [5, 5, 20]}, "offset": 5, "name": "Above"}));
    let above = plane(&s, "Above");
    assert!((above.origin.z - 25.0).abs() < 1e-9, "{above:?}");
    assert!((above.normal().z.abs() - 1.0).abs() < 1e-9, "{above:?}");
    // Stored as an exact face attachment, not a frozen copy of the plane.
    let feature = s.doc.find_feature("Above").unwrap();
    let FeatureKind::ConstructionPlane { plane: PlaneRef::Offset { base, .. } } = &feature.kind else { panic!("offset plane") };
    assert!(matches!(base.as_ref(), PlaneRef::Face { anchor: Some(_), .. }), "{base:?}");

    // Side face (x = 40), 3 mm out.
    run(&mut s, "construct.plane.offset", json!({"base": {"face": [40, 15, 10]}, "offset": 3, "name": "Side"}));
    let side = plane(&s, "Side");
    assert!((side.origin.x - 43.0).abs() < 1e-9, "{side:?}");
    assert!((side.normal().x.abs() - 1.0).abs() < 1e-9, "{side:?}");

    // A 10 × 10 × 10 pad sketched on the new plane sits 5 mm above the box.
    run(&mut s, "sketch.create", json!({"plane": "Above"}));
    run(&mut s, "sketch.rectangle.two_point", json!({"p0": [0, 0], "p1": [10, 10]}));
    run(&mut s, "sketch.finish", json!({}));
    run(&mut s, "solid.extrude", json!({"distance": 10, "operation": "new"}));
    s.refresh();
    assert!(s.model.results.iter().all(|r| r.error.is_none()), "{:?}", s.model.results);
    assert!((volume(&s) - (40.0 * 30.0 * 20.0 + 1000.0)).abs() < 1e-6, "{}", volume(&s));

    // Taller and longer box: both planes move with their faces.
    run(&mut s, "timeline.edit", json!({"feature": "Base", "set": {"height": "30", "length": "50"}}));
    assert!((plane(&s, "Above").origin.z - 35.0).abs() < 1e-9);
    assert!((plane(&s, "Side").origin.x - 53.0).abs() < 1e-9);
    assert!(s.model.results.iter().all(|r| r.error.is_none()), "{:?}", s.model.results);

    // Saved and reopened, the attachment still resolves.
    let reopened = Session::new(Document::from_json(&s.doc.to_json()).unwrap());
    assert!((plane(&reopened, "Above").origin.z - 35.0).abs() < 1e-9);
    assert!((plane(&reopened, "Side").origin.x - 53.0).abs() < 1e-9);
}

#[test]
fn offset_plane_rejects_bad_face_picks_without_panicking() {
    let mut s = Session::default();
    run(&mut s, "solid.box", json!({"length": 40, "width": 30, "height": 20}));
    for base in [
        json!({"face": [100, 100, 100]}),
        json!({"face": [1e300, 0, 0]}),
        json!({"face": "top"}),
        json!({"face": [5, 5]}),
        json!({"face": [5, 5, 20], "index": 0}),
        json!({"face": [5, 5, 20], "body": 7, "index": 0}),
    ] {
        let features = s.doc.features.len();
        assert!(s.execute("construct.plane.offset", &json!({"base": base, "offset": 5})).is_err(), "{base}");
        assert_eq!(s.doc.features.len(), features, "{base}");
    }
}

/// Distance from `p` to the plane.
fn off(pl: &Plane, p: [f64; 3]) -> f64 {
    (solidcraft_geom::Vec3::new(p[0], p[1], p[2]) - pl.origin).dot(pl.normal())
}

#[test]
fn plane_at_angle_about_an_edge_follows_the_edge() {
    let mut s = Session::default();
    run(&mut s, "solid.box", json!({"length": 40, "width": 30, "height": 20, "name": "Base"}));
    run(&mut s, "construct.plane.offset", json!({"base": {"face": [5, 5, 20]}, "offset": 0, "name": "Lid"}));
    // Hinged on the top edge along X at y = 30 (#2229).
    run(&mut s, "construct.plane.at_angle", json!({"base": "Lid", "axis": {"edge": [20, 30, 20]}, "angle": "45 deg", "name": "Tilt"}));
    let tilt = plane(&s, "Tilt");
    let h = std::f64::consts::FRAC_1_SQRT_2;
    assert!((tilt.normal().z.abs() - h).abs() < 1e-9 && (tilt.normal().y.abs() - h).abs() < 1e-9, "{tilt:?}");
    for x in [0.0, 40.0] {
        assert!(off(&tilt, [x, 30.0, 20.0]).abs() < 1e-9, "{tilt:?}");
    }
    // Kept as the edge, not a frozen line.
    let feature = s.doc.find_feature("Tilt").unwrap();
    let FeatureKind::ConstructionPlane { plane: PlaneRef::AtAngle { line, .. } } = &feature.kind else { panic!("plane at angle") };
    assert!(matches!(line.as_deref(), Some(solidcraft_doc::construct::GeoRef::Edge { name: Some(_), .. })), "{line:?}");

    // A taller, wider box moves the edge to y = 40, z = 30: the plane still holds it.
    run(&mut s, "timeline.edit", json!({"feature": "Base", "set": {"height": "30", "width": "40"}}));
    assert!(s.model.results.iter().all(|r| r.error.is_none()), "{:?}", s.model.results);
    let tilt = plane(&s, "Tilt");
    for x in [0.0, 40.0] {
        assert!(off(&tilt, [x, 40.0, 30.0]).abs() < 1e-9, "{tilt:?}");
    }

    // Saved and reopened, it still resolves through the edge.
    let reopened = Session::new(Document::from_json(&s.doc.to_json()).unwrap());
    assert!(off(&plane(&reopened, "Tilt"), [20.0, 40.0, 30.0]).abs() < 1e-9);
}

#[test]
fn plane_at_angle_about_a_sketch_line_or_construction_axis() {
    let mut s = Session::default();
    run(&mut s, "sketch.create", json!({"plane": "XY", "name": "Hinge"}));
    run(&mut s, "sketch.line", json!({"points": [[0, 10], [40, 10]]}));
    run(&mut s, "sketch.finish", json!({}));
    run(&mut s, "construct.plane.at_angle", json!({"base": "XY", "axis": {"sketch": "Hinge", "curve": "l1"}, "angle": 90, "name": "Up"}));
    let up = plane(&s, "Up");
    assert!(up.normal().y.abs() > 1.0 - 1e-9, "{up:?}");
    assert!(off(&up, [0.0, 10.0, 0.0]).abs() < 1e-9, "{up:?}");

    run(&mut s, "solid.box", json!({"length": 40, "width": 30, "height": 20}));
    run(&mut s, "construct.axis.edge", json!({"edge": {"edge": [40, 15, 0]}, "name": "A1"}));
    run(&mut s, "construct.plane.at_angle", json!({"base": "XY", "axis": "A1", "angle": 90, "name": "Side"}));
    let side = plane(&s, "Side");
    assert!(side.normal().x.abs() > 1.0 - 1e-9, "{side:?}");
    assert!(off(&side, [40.0, 0.0, 0.0]).abs() < 1e-9, "{side:?}");
}

/// The base plane 10 mm above the edge: the plane still goes through the edge, at the angle to
/// the base; an axis that crosses the base plane has no such plane.
#[test]
fn plane_at_angle_about_an_edge_goes_through_it_from_a_plane_above() {
    let mut s = Session::default();
    run(&mut s, "solid.box", json!({"length": 40, "width": 30, "height": 20}));
    run(&mut s, "construct.plane.offset", json!({"base": "XY", "offset": 30, "name": "Above"}));
    run(&mut s, "construct.plane.at_angle", json!({"base": "Above", "axis": {"edge": [20, 0, 20]}, "angle": 30, "name": "Tilt"}));
    let tilt = plane(&s, "Tilt");
    for x in [0.0, 40.0] {
        assert!(off(&tilt, [x, 0.0, 20.0]).abs() < 1e-9, "{tilt:?}");
    }
    assert!((tilt.normal().z.abs() - 30f64.to_radians().cos()).abs() < 1e-9, "{tilt:?}");

    let features = s.doc.features.len();
    let err = s.execute("construct.plane.at_angle", &json!({"base": "Above", "axis": {"edge": [40, 0, 10]}, "angle": 30})).unwrap_err();
    assert!(err.to_string().contains("parallel"), "{err}");
    assert_eq!(s.doc.features.len(), features);
}

#[test]
fn plane_at_angle_rejects_bad_axes_without_panicking() {
    let mut s = Session::default();
    run(&mut s, "solid.box", json!({"length": 40, "width": 30, "height": 20}));
    for axis in [
        json!({"edge": [100, 100, 100]}),
        json!({"edge": [1e300, 0, 0]}),
        json!({"edge": [f64::MAX, 0, 0]}),
        json!({"edge": [5, 5]}),
        json!({"sketch": 999, "curve": "l1"}),
        json!({"face": [5, 5, 20]}),
        json!("NoSuchAxis"),
        json!({}),
        json!([1, 2, 3]),
        json!({"origin": [0, 0, 0]}),
    ] {
        let features = s.doc.features.len();
        assert!(s.execute("construct.plane.at_angle", &json!({"base": "XY", "axis": axis, "angle": 30})).is_err(), "{axis}");
        assert_eq!(s.doc.features.len(), features, "{axis}");
    }
}
