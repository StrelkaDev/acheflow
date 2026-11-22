//! Pointer-independent editor geometry, in scene pixels.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Bounds {
    pub x: u32,
    pub y: u32,
    pub width: u32,
    pub height: u32,
}
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Corner {
    TopLeft,
    TopRight,
    BottomLeft,
    BottomRight,
}
impl Corner {
    pub const ALL: [Self; 4] = [
        Self::TopLeft,
        Self::TopRight,
        Self::BottomLeft,
        Self::BottomRight,
    ];
    pub fn class(self) -> &'static str {
        match self {
            Self::TopLeft => "top-left",
            Self::TopRight => "top-right",
            Self::BottomLeft => "bottom-left",
            Self::BottomRight => "bottom-right",
        }
    }
    pub fn label(self) -> &'static str {
        match self {
            Self::TopLeft => "Resize top left",
            Self::TopRight => "Resize top right",
            Self::BottomLeft => "Resize bottom left",
            Self::BottomRight => "Resize bottom right",
        }
    }
}
#[derive(Clone, Copy)]
pub enum DragKind {
    Move,
    Resize(Corner),
}

#[derive(Clone, Copy, Default)]
pub struct DragConstraints {
    pub aspect: Option<f64>,
    pub snap: (f64, f64),
}

pub fn transform(
    start: Bounds,
    kind: DragKind,
    dx: f64,
    dy: f64,
    canvas: (u32, u32),
    constraints: DragConstraints,
) -> Bounds {
    let x = f64::from(start.x);
    let y = f64::from(start.y);
    let right = x + f64::from(start.width);
    let bottom = y + f64::from(start.height);
    let (cw, ch) = (f64::from(canvas.0), f64::from(canvas.1));
    let (snap_x, snap_y) = constraints.snap;
    match kind {
        DragKind::Move => Bounds {
            x: snap_translation(x + dx, cw - f64::from(start.width), snap_x),
            y: snap_translation(y + dy, ch - f64::from(start.height), snap_y),
            ..start
        },
        DragKind::Resize(corner) => {
            let left_edge = matches!(corner, Corner::TopLeft | Corner::BottomLeft);
            let top_edge = matches!(corner, Corner::TopLeft | Corner::TopRight);
            if let Some(ratio) = constraints.aspect.filter(|r| r.is_finite() && *r > 0.) {
                let anchor = (
                    if left_edge { right } else { x },
                    if top_edge { bottom } else { y },
                );
                let limit = (
                    if left_edge { anchor.0 } else { cw - anchor.0 },
                    if top_edge { anchor.1 } else { ch - anchor.1 },
                );
                let intended = (
                    f64::from(start.width) + if left_edge { -dx } else { dx },
                    f64::from(start.height) + if top_edge { -dy } else { dy },
                );
                let max_height = (limit.0 / ratio).min(limit.1);
                if max_height > 0. {
                    // Project the pointer onto the resize line through the fixed corner.
                    let min_height = (1_f64 / ratio).max(1.).min(max_height);
                    let mut height = ((intended.0 * ratio + intended.1) / (ratio * ratio + 1.))
                        .clamp(min_height, max_height);
                    let (gap_x, gap_y) = (limit.0 - height * ratio, limit.1 - height);
                    if (gap_x <= snap_x && limit.0 / ratio <= limit.1)
                        || (gap_y <= snap_y && limit.1 <= limit.0 / ratio)
                    {
                        height = max_height;
                    }
                    let width = (height * ratio).round().max(1.);
                    let height = height.round().max(1.);
                    return Bounds {
                        x: (anchor.0 - if left_edge { width } else { 0. }).max(0.) as u32,
                        y: (anchor.1 - if top_edge { height } else { 0. }).max(0.) as u32,
                        width: width as u32,
                        height: height as u32,
                    };
                }
            }
            let left = if left_edge {
                snap_to(x + dx, 0., snap_x).round().clamp(0., right - 1.)
            } else {
                x
            };
            let top = if top_edge {
                snap_to(y + dy, 0., snap_y).round().clamp(0., bottom - 1.)
            } else {
                y
            };
            let right = if left_edge {
                right
            } else {
                snap_to(right + dx, cw, snap_x)
                    .round()
                    .clamp(x + 1., cw.max(x + 1.))
            };
            let bottom = if top_edge {
                bottom
            } else {
                snap_to(bottom + dy, ch, snap_y)
                    .round()
                    .clamp(y + 1., ch.max(y + 1.))
            };
            Bounds {
                x: left as u32,
                y: top as u32,
                width: (right - left) as u32,
                height: (bottom - top) as u32,
            }
        }
    }
}

fn snap_to(value: f64, edge: f64, distance: f64) -> f64 {
    if (value - edge).abs() <= distance {
        edge
    } else {
        value
    }
}

fn snap_translation(value: f64, max: f64, distance: f64) -> u32 {
    let max = max.max(0.);
    let value = value.clamp(0., max);
    let nearest = if value <= max / 2. { 0. } else { max };
    snap_to(value, nearest, distance).round() as u32
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn proportional_resize_preserves_the_fixed_corner_and_snaps_at_the_canvas() {
        let start = Bounds {
            x: 52,
            y: 26,
            width: 40,
            height: 20,
        };
        let result = transform(
            start,
            DragKind::Resize(Corner::TopLeft),
            -45.,
            -22.,
            (200, 120),
            DragConstraints {
                aspect: Some(2.),
                snap: (8., 8.),
            },
        );
        assert_eq!(
            result,
            Bounds {
                x: 0,
                y: 0,
                width: 92,
                height: 46
            }
        );

        let source_ratio = transform(
            start,
            DragKind::Resize(Corner::BottomRight),
            20.,
            20.,
            (200, 120),
            DragConstraints {
                aspect: Some(1.),
                snap: (8., 8.),
            },
        );
        assert_eq!(
            source_ratio,
            Bounds {
                width: 50,
                height: 50,
                ..start
            }
        );
        let moved = transform(
            start,
            DragKind::Move,
            -46.,
            69.,
            (200, 120),
            DragConstraints {
                aspect: None,
                snap: (8., 8.),
            },
        );
        assert_eq!(
            moved,
            Bounds {
                x: 0,
                y: 100,
                ..start
            }
        );
    }

    #[test]
    fn every_corner_keeps_the_opposite_corner_and_clamps_to_canvas() {
        let b = Bounds {
            x: 20,
            y: 30,
            width: 60,
            height: 40,
        };
        for corner in Corner::ALL {
            let resized = transform(
                b,
                DragKind::Resize(corner),
                10.,
                5.,
                (200, 100),
                DragConstraints::default(),
            );
            assert!(resized.width > 0 && resized.height > 0);
            if matches!(corner, Corner::TopLeft | Corner::BottomLeft) {
                assert_eq!(resized.x + resized.width, 80);
            } else {
                assert_eq!(resized.x, 20);
            }
            if matches!(corner, Corner::TopLeft | Corner::TopRight) {
                assert_eq!(resized.y + resized.height, 70);
            } else {
                assert_eq!(resized.y, 30);
            }
        }
        assert_eq!(
            transform(
                b,
                DragKind::Resize(Corner::TopLeft),
                999.,
                999.,
                (200, 100),
                DragConstraints::default()
            ),
            Bounds {
                x: 79,
                y: 69,
                width: 1,
                height: 1
            }
        );
        assert_eq!(
            transform(
                b,
                DragKind::Resize(Corner::BottomRight),
                999.,
                999.,
                (200, 100),
                DragConstraints::default()
            ),
            Bounds {
                width: 180,
                height: 70,
                ..b
            }
        );
        assert_eq!(
            transform(
                b,
                DragKind::Move,
                -999.,
                999.,
                (200, 100),
                DragConstraints::default()
            ),
            Bounds { x: 0, y: 60, ..b }
        );
    }
}
