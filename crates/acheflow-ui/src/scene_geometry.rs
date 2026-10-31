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
pub fn transform(start: Bounds, kind: DragKind, dx: f64, dy: f64, canvas: (u32, u32)) -> Bounds {
    let x = f64::from(start.x);
    let y = f64::from(start.y);
    let right = x + f64::from(start.width);
    let bottom = y + f64::from(start.height);
    let (cw, ch) = (f64::from(canvas.0), f64::from(canvas.1));
    match kind {
        DragKind::Move => Bounds {
            x: (x + dx)
                .round()
                .clamp(0., (cw - f64::from(start.width)).max(0.)) as u32,
            y: (y + dy)
                .round()
                .clamp(0., (ch - f64::from(start.height)).max(0.)) as u32,
            ..start
        },
        DragKind::Resize(corner) => {
            let left_edge = matches!(corner, Corner::TopLeft | Corner::BottomLeft);
            let top_edge = matches!(corner, Corner::TopLeft | Corner::TopRight);
            let left = if left_edge {
                (x + dx).round().clamp(0., right - 1.)
            } else {
                x
            };
            let top = if top_edge {
                (y + dy).round().clamp(0., bottom - 1.)
            } else {
                y
            };
            let right = if left_edge {
                right
            } else {
                (right + dx).round().clamp(x + 1., cw.max(x + 1.))
            };
            let bottom = if top_edge {
                bottom
            } else {
                (bottom + dy).round().clamp(y + 1., ch.max(y + 1.))
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
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn every_corner_keeps_the_opposite_corner_and_clamps_to_canvas() {
        let b = Bounds {
            x: 20,
            y: 30,
            width: 60,
            height: 40,
        };
        for corner in Corner::ALL {
            let resized = transform(b, DragKind::Resize(corner), 10., 5., (200, 100));
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
            transform(b, DragKind::Resize(Corner::TopLeft), 999., 999., (200, 100)),
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
                (200, 100)
            ),
            Bounds {
                width: 180,
                height: 70,
                ..b
            }
        );
        assert_eq!(
            transform(b, DragKind::Move, -999., 999., (200, 100)),
            Bounds { x: 0, y: 60, ..b }
        );
    }
}
