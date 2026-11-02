//! Editor-only guides, projected with the same camera as the GPU scene.
use crate::spatial_view::{self, EditorView, Tool};
use acheflow_composition::spatial::transform_matrix;
use acheflow_media_api::spatial::{Camera, ObjectKind, SpatialScene, Transform};
use anyrender::{PaintScene, Scene};
use glam::{Vec2, Vec3};
use peniko::{
    Color, Fill,
    kurbo::{Affine, Circle, Line, Point, Rect, Stroke},
};

pub fn paint(output: &mut Scene, view: &EditorView, scene: &SpatialScene, size: Vec2) {
    let camera = view.camera();
    for object in scene.objects.iter().filter(|o| o.visible) {
        let selected = view.selected == Some(object.id);
        let color = if selected {
            Color::from_rgb8(255, 180, 79)
        } else {
            Color::from_rgba8(150, 167, 189, 110)
        };
        bounds(
            output,
            &camera,
            size,
            &object.transform,
            matches!(object.kind, ObjectKind::Cube { .. }),
            color,
            if selected { 2. } else { 1. },
        );
    }
    camera_guide(
        output,
        &camera,
        size,
        &scene.camera,
        view.selected == Some(0),
    );
    for handle in spatial_view::handles(view, scene, size) {
        let color = [
            Color::from_rgb8(241, 99, 108),
            Color::from_rgb8(123, 207, 143),
            Color::from_rgb8(102, 167, 255),
        ][handle.axis];
        line(
            output,
            handle.start,
            handle.end,
            Color::from_rgba8(8, 12, 19, 230),
            6.,
        );
        line(output, handle.start, handle.end, color, 3.);
        let end = Point::new(handle.end.x as f64, handle.end.y as f64);
        match view.tool {
            Tool::Rotate => output.stroke(
                &Stroke::new(3.),
                Affine::IDENTITY,
                color,
                None,
                &Circle::new(end, 7.),
            ),
            Tool::Scale => output.fill(
                Fill::NonZero,
                Affine::IDENTITY,
                color,
                None,
                &Rect::new(end.x - 5., end.y - 5., end.x + 5., end.y + 5.),
            ),
            Tool::Move => {
                let direction = (handle.end - handle.start).normalize();
                let side = Vec2::new(-direction.y, direction.x);
                line(
                    output,
                    handle.end,
                    handle.end - direction * 12. + side * 6.,
                    color,
                    3.,
                );
                line(
                    output,
                    handle.end,
                    handle.end - direction * 12. - side * 6.,
                    color,
                    3.,
                );
            }
        }
    }
    if let Some(center) = view
        .selected_transform(scene)
        .and_then(|t| spatial_view::project(&camera, Vec3::from_array(t.position), size))
    {
        output.fill(
            Fill::NonZero,
            Affine::IDENTITY,
            Color::from_rgb8(247, 231, 205),
            None,
            &Circle::new((center.x as f64, center.y as f64), 4.),
        );
    }
}
pub fn paint_grid(output: &mut Scene, view: &EditorView, size: Vec2) {
    let camera = view.camera();
    if view.grid {
        let step = 10f32.powf((view.distance / 12.).log10().floor());
        let extent = step * 12.;
        for i in -12..=12 {
            let d = i as f32 * step;
            let color = if i == 0 {
                Color::from_rgba8(79, 112, 106, 170)
            } else {
                Color::from_rgba8(77, 86, 101, 85)
            };
            world_line(
                output,
                &camera,
                size,
                Vec3::new(-extent, 0., d),
                Vec3::new(extent, 0., d),
                color,
                1.,
            );
            world_line(
                output,
                &camera,
                size,
                Vec3::new(d, 0., -extent),
                Vec3::new(d, 0., extent),
                color,
                1.,
            );
        }
    }
}
fn bounds(
    output: &mut Scene,
    camera: &Camera,
    size: Vec2,
    t: &Transform,
    cube: bool,
    color: Color,
    width: f64,
) {
    let matrix = transform_matrix(t);
    let z = if cube { 0.5 } else { 0. };
    let corners = [
        [-0.5, -0.5, -z],
        [0.5, -0.5, -z],
        [0.5, 0.5, -z],
        [-0.5, 0.5, -z],
        [-0.5, -0.5, z],
        [0.5, -0.5, z],
        [0.5, 0.5, z],
        [-0.5, 0.5, z],
    ]
    .map(|p| matrix.transform_point3(Vec3::from_array(p)));
    let edges = [
        (0, 1),
        (1, 2),
        (2, 3),
        (3, 0),
        (4, 5),
        (5, 6),
        (6, 7),
        (7, 4),
        (0, 4),
        (1, 5),
        (2, 6),
        (3, 7),
    ];
    for (a, b) in edges.into_iter().take(if cube { 12 } else { 4 }) {
        world_line(output, camera, size, corners[a], corners[b], color, width);
    }
}
fn camera_guide(output: &mut Scene, view: &Camera, size: Vec2, camera: &Camera, selected: bool) {
    let color = if selected {
        Color::from_rgb8(255, 180, 79)
    } else {
        Color::from_rgb8(140, 160, 185)
    };
    let transform = transform_matrix(&camera.transform);
    let origin = transform.transform_point3(Vec3::ZERO);
    let corners = [
        Vec3::new(-100., -60., -140.),
        Vec3::new(100., -60., -140.),
        Vec3::new(100., 60., -140.),
        Vec3::new(-100., 60., -140.),
    ]
    .map(|p| transform.transform_point3(p));
    for i in 0..4 {
        world_line(output, view, size, origin, corners[i], color, 1.5);
        world_line(
            output,
            view,
            size,
            corners[i],
            corners[(i + 1) % 4],
            color,
            1.5,
        );
    }
}
fn world_line(
    output: &mut Scene,
    camera: &Camera,
    size: Vec2,
    a: Vec3,
    b: Vec3,
    color: Color,
    width: f64,
) {
    if let (Some(a), Some(b)) = (
        spatial_view::project(camera, a, size),
        spatial_view::project(camera, b, size),
    ) {
        line(output, a, b, color, width);
    }
}
fn line(output: &mut Scene, a: Vec2, b: Vec2, color: Color, width: f64) {
    output.stroke(
        &Stroke::new(width),
        Affine::IDENTITY,
        color,
        None,
        &Line::new((a.x as f64, a.y as f64), (b.x as f64, b.y as f64)),
    );
}
