//! Transient editor view. None of this state changes the broadcast camera.
use acheflow_composition::spatial::{camera_matrix, transform_matrix};
use acheflow_media_api::spatial::{Camera, ObjectKind, Projection, SpatialScene, Transform};
use glam::{EulerRot, Quat, Vec2, Vec3};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Tool {
    Move,
    Rotate,
    Scale,
}
impl Tool {
    pub const ALL: [Self; 3] = [Self::Move, Self::Rotate, Self::Scale];
    pub fn label(self) -> &'static str {
        match self {
            Self::Move => "Move",
            Self::Rotate => "Rotate",
            Self::Scale => "Scale",
        }
    }
}
#[derive(Clone, Debug, PartialEq)]
pub struct EditorView {
    pub target: Vec3,
    pub yaw: f32,
    pub pitch: f32,
    pub distance: f32,
    pub perspective: bool,
    pub selected: Option<u64>,
    pub tool: Tool,
    pub grid: bool,
}
impl EditorView {
    pub fn new(height: f32) -> Self {
        Self {
            target: Vec3::ZERO,
            yaw: -25.,
            pitch: -18.,
            distance: height * 2.,
            perspective: true,
            selected: Some(1),
            tool: Tool::Move,
            grid: true,
        }
    }
    fn rotation(&self) -> Quat {
        Quat::from_rotation_y(self.yaw.to_radians())
            * Quat::from_rotation_x(self.pitch.to_radians())
    }
    pub fn camera(&self) -> Camera {
        let rotation = self.rotation();
        let (x, y, z) = rotation.to_euler(EulerRot::XYZ);
        Camera {
            transform: Transform {
                position: (self.target + rotation * Vec3::Z * self.distance).to_array(),
                rotation: [x.to_degrees(), y.to_degrees(), z.to_degrees()],
                ..Default::default()
            },
            projection: if self.perspective {
                Projection::Perspective { fov_degrees: 50. }
            } else {
                Projection::Orthographic {
                    height: self.distance,
                }
            },
            near: 0.1,
            far: 1_000_000.,
        }
    }
    pub fn orbit(&mut self, delta: Vec2) {
        self.yaw -= delta.x * 0.3;
        self.pitch = (self.pitch - delta.y * 0.3).clamp(-89.5, 89.5);
    }
    pub fn pan(&mut self, delta: Vec2, height: f32) {
        self.target -=
            self.rotation() * Vec3::new(delta.x, -delta.y, 0.) * (self.distance / height.max(1.));
    }
    pub fn zoom(&mut self, delta: f32) {
        self.distance = (self.distance * (delta * 0.0015).exp()).clamp(1., 100_000.);
    }
    pub fn frame(&mut self, transform: &Transform) {
        self.target = Vec3::from_array(transform.position);
        self.distance = (Vec3::from_array(transform.scale).length() * 1.8).clamp(100., 100_000.);
    }
    pub fn selected_transform<'a>(&self, scene: &'a SpatialScene) -> Option<&'a Transform> {
        selected_transform(scene, self.selected?)
    }
}
pub fn selected_transform(scene: &SpatialScene, id: u64) -> Option<&Transform> {
    if id == 0 {
        Some(&scene.camera.transform)
    } else {
        scene
            .objects
            .iter()
            .find(|o| o.id == id)
            .map(|o| &o.transform)
    }
}
pub fn project(camera: &Camera, point: Vec3, size: Vec2) -> Option<Vec2> {
    let clip = camera_matrix(camera, size.x / size.y) * point.extend(1.);
    if clip.w <= 0. || clip.z < 0. || clip.z > clip.w {
        return None;
    }
    let ndc = clip.truncate() / clip.w;
    Some(Vec2::new(
        (ndc.x + 1.) * size.x / 2.,
        (1. - ndc.y) * size.y / 2.,
    ))
}
fn ray(camera: &Camera, point: Vec2, size: Vec2) -> (Vec3, Vec3) {
    let inverse = camera_matrix(camera, size.x / size.y).inverse();
    let ndc = Vec2::new(point.x / size.x * 2. - 1., 1. - point.y / size.y * 2.);
    let near = inverse.project_point3(ndc.extend(0.));
    // Avoid floating-point cancellation at the far clip of a large editor scene.
    let far = inverse.project_point3(ndc.extend(0.99));
    (near, (far - near).normalize())
}
pub fn pick(view: &EditorView, scene: &SpatialScene, point: Vec2, size: Vec2) -> Option<u64> {
    let (origin, direction) = ray(&view.camera(), point, size);
    let camera_proxy = Transform {
        scale: [60.; 3],
        ..scene.camera.transform.clone()
    };
    scene
        .objects
        .iter()
        .filter(|o| o.visible)
        .map(|o| {
            (
                o.id,
                &o.transform,
                matches!(o.kind, ObjectKind::Cube { .. }),
            )
        })
        .chain(std::iter::once((0, &camera_proxy, true)))
        .filter_map(|(id, t, cube)| {
            hit_box(origin, direction, t, cube).map(|distance| (id, distance))
        })
        .min_by(|a, b| a.1.total_cmp(&b.1))
        .map(|(id, _)| id)
}
fn hit_box(origin: Vec3, direction: Vec3, transform: &Transform, cube: bool) -> Option<f32> {
    let inverse = transform_matrix(transform).inverse();
    let origin = inverse.transform_point3(origin);
    let direction = inverse.transform_vector3(direction);
    let half = Vec3::new(0.5, 0.5, if cube { 0.5 } else { 0.001 });
    let mut near = 0f32;
    let mut far = f32::INFINITY;
    for axis in 0..3 {
        if direction[axis].abs() < 1e-8 {
            if origin[axis].abs() > half[axis] {
                return None;
            }
            continue;
        }
        let a = (-half[axis] - origin[axis]) / direction[axis];
        let b = (half[axis] - origin[axis]) / direction[axis];
        near = near.max(a.min(b));
        far = far.min(a.max(b));
        if near > far {
            return None;
        }
    }
    (far >= 0.).then_some(near)
}
#[derive(Clone, Copy, Debug)]
pub struct AxisHandle {
    pub axis: usize,
    pub start: Vec2,
    pub end: Vec2,
    pub length: f32,
}
pub fn units_per_pixel(camera: &Camera, center: Vec3, height: f32) -> f32 {
    let depth = (transform_matrix(&camera.transform)
        .inverse()
        .transform_point3(center)
        .z)
        .abs();
    let height_world = match camera.projection {
        Projection::Orthographic { height } => height,
        Projection::Perspective { fov_degrees } => {
            2. * depth * (fov_degrees.to_radians() / 2.).tan()
        }
    };
    height_world / height.max(1.)
}
pub fn handles(view: &EditorView, scene: &SpatialScene, size: Vec2) -> Vec<AxisHandle> {
    let Some(transform) = view.selected_transform(scene) else {
        return vec![];
    };
    let center = Vec3::from_array(transform.position);
    let camera = view.camera();
    let Some(start) = project(&camera, center, size) else {
        return vec![];
    };
    let length = units_per_pixel(&camera, center, size.y) * 76.;
    [Vec3::X, Vec3::Y, Vec3::Z]
        .into_iter()
        .enumerate()
        .filter_map(|(axis, dir)| {
            let end = project(&camera, center + dir * length, size)?;
            (end.distance(start) > 12.).then_some(AxisHandle {
                axis,
                start,
                end,
                length,
            })
        })
        .collect()
}
pub fn pick_axis(
    view: &EditorView,
    scene: &SpatialScene,
    point: Vec2,
    size: Vec2,
) -> Option<AxisHandle> {
    handles(view, scene, size)
        .into_iter()
        .filter_map(|h| {
            let segment = h.end - h.start;
            let t = ((point - h.start).dot(segment) / segment.length_squared()).clamp(0.15, 1.);
            let distance = point.distance(h.start + segment * t);
            (distance < 10.).then_some((h, distance))
        })
        .min_by(|a, b| a.1.total_cmp(&b.1))
        .map(|(handle, _)| handle)
}
pub fn manipulate(
    start: &Transform,
    tool: Tool,
    handle: AxisHandle,
    delta: Vec2,
    snap: bool,
) -> Transform {
    let screen = handle.end - handle.start;
    let fraction = delta.dot(screen) / screen.length_squared();
    let mut next = start.clone();
    let quantize = |value: f32, step: f32| {
        if snap {
            (value / step).round() * step
        } else {
            value
        }
    };
    match tool {
        Tool::Move => next.position[handle.axis] += quantize(fraction * handle.length, 10.),
        Tool::Rotate => next.rotation[handle.axis] += quantize(fraction * 90., 15.),
        Tool::Scale => {
            next.scale[handle.axis] =
                (start.scale[handle.axis] * (1. + quantize(fraction, 0.1))).clamp(0.01, 100_000.)
        }
    }
    next
}

#[cfg(test)]
mod tests {
    use super::*;
    use acheflow_media_api::{spatial::SceneObject, workspace::Resolution};
    #[test]
    fn picking_uses_visible_transformed_geometry_and_nearest_depth() {
        let mut scene = SpatialScene::from_canvas(&Resolution {
            width: 1920,
            height: 1080,
        });
        let view = EditorView {
            yaw: 0.,
            pitch: 0.,
            ..EditorView::new(1080.)
        };
        let size = Vec2::new(800., 500.);
        scene.objects.push(SceneObject {
            id: 2,
            name: "Cube".into(),
            visible: true,
            transform: Transform {
                position: [100., 0., 200.],
                rotation: [0., 45., 0.],
                scale: [150.; 3],
            },
            kind: ObjectKind::Cube { color: [1.; 4] },
        });
        let point = project(&view.camera(), Vec3::new(100., 0., 200.), size).unwrap();
        assert_eq!(pick(&view, &scene, point, size), Some(2));
        scene.objects[1].visible = false;
        assert_eq!(pick(&view, &scene, point, size), Some(1));
        assert_eq!(pick(&view, &scene, Vec2::ZERO, size), None);
    }
    #[test]
    fn axis_drag_respects_world_axis_snap_rotation_and_positive_scale() {
        let start = Transform::default();
        let handle = AxisHandle {
            axis: 2,
            start: Vec2::ZERO,
            end: Vec2::new(0., 100.),
            length: 200.,
        };
        let moved = manipulate(&start, Tool::Move, handle, Vec2::new(40., 23.), true);
        assert_eq!(moved.position, [0., 0., 50.]);
        assert_eq!(
            manipulate(&start, Tool::Rotate, handle, Vec2::new(0., 50.), true).rotation,
            [0., 0., 45.]
        );
        assert_eq!(
            manipulate(&start, Tool::Scale, handle, Vec2::new(0., -200.), false).scale,
            [1., 1., 0.01]
        );
    }
    #[test]
    fn orbit_pan_zoom_and_frame_leave_broadcast_camera_unchanged() {
        let scene = SpatialScene::from_canvas(&Resolution {
            width: 1920,
            height: 1080,
        });
        let camera = scene.camera.clone();
        let mut view = EditorView::new(1080.);
        view.orbit(Vec2::new(20., 900.));
        view.pan(Vec2::new(30., 10.), 500.);
        view.zoom(-300.);
        assert!(
            view.camera()
                .transform
                .position
                .iter()
                .all(|v| v.is_finite())
        );
        assert_eq!(scene.camera, camera);
        view.frame(&scene.objects[0].transform);
        assert_eq!(view.target, Vec3::ZERO);
        let size = Vec2::new(800., 500.);
        let center = project(&view.camera(), view.target, size).unwrap();
        assert!(center.distance(size / 2.) < 0.01);
    }
}
