use crate::workspace::{Layer, Resolution};
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Transform {
    pub position: [f32; 3],
    pub rotation: [f32; 3],
    pub scale: [f32; 3],
}
impl Default for Transform {
    fn default() -> Self {
        Self {
            position: [0.; 3],
            rotation: [0.; 3],
            scale: [1.; 3],
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
pub enum Projection {
    Orthographic { height: f32 },
    Perspective { fov_degrees: f32 },
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Camera {
    pub transform: Transform,
    pub projection: Projection,
    pub near: f32,
    pub far: f32,
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
pub enum ObjectKind {
    /// The primary viewport references the scene's legacy layers by stable identity.
    Viewport {
        primary: bool,
        canvas: Resolution,
        layers: Vec<Layer>,
    },
    Plane {
        color: [f32; 4],
    },
    Cube {
        color: [f32; 4],
    },
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SceneObject {
    pub id: u64,
    pub name: String,
    pub visible: bool,
    pub transform: Transform,
    pub kind: ObjectKind,
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SpatialScene {
    pub version: u32,
    pub camera: Camera,
    pub objects: Vec<SceneObject>,
}

impl SpatialScene {
    pub fn from_canvas(canvas: &Resolution) -> Self {
        Self {
            version: 1,
            camera: Camera {
                transform: Transform {
                    position: [0., 0., 1000.],
                    ..Default::default()
                },
                projection: Projection::Orthographic {
                    height: canvas.height as f32,
                },
                near: 0.1,
                far: 10000.,
            },
            objects: vec![SceneObject {
                id: 1,
                name: "Viewport".into(),
                visible: true,
                transform: Transform {
                    scale: [canvas.width as f32, canvas.height as f32, 1.],
                    ..Default::default()
                },
                kind: ObjectKind::Viewport {
                    primary: true,
                    canvas: canvas.clone(),
                    layers: vec![],
                },
            }],
        }
    }
    pub fn validate(&self, source_ids: &[u32]) -> Result<(), String> {
        if self.version != 1 || self.objects.len() > 32 {
            return Err("Unsupported 3D scene version or object limit".into());
        }
        self.camera.validate()?;
        let mut ids = std::collections::HashSet::new();
        for object in &self.objects {
            if object.id == 0 || !ids.insert(object.id) {
                return Err("Invalid or duplicate 3D object id".into());
            }
            object.validate(source_ids)?;
        }
        let viewports: Vec<_> = self
            .objects
            .iter()
            .filter_map(|o| match &o.kind {
                ObjectKind::Viewport {
                    primary, canvas, ..
                } => Some((*primary, canvas)),
                _ => None,
            })
            .collect();
        if viewports.iter().filter(|(primary, _)| *primary).count() > 1 || viewports.len() > 8 {
            return Err("At most one primary and eight total viewports".into());
        }
        let pixels: u64 = viewports
            .iter()
            .map(|(_, canvas)| u64::from(canvas.width) * u64::from(canvas.height))
            .sum();
        if pixels > 2 * 3840 * 2160 {
            return Err("Combined viewport pixel budget exceeded".into());
        }
        Ok(())
    }
}
impl Camera {
    fn validate(&self) -> Result<(), String> {
        validate_transform(&self.transform)?;
        if !self.near.is_finite() || !self.far.is_finite() {
            return Err("Camera clipping planes must be finite".into());
        }
        if self.near <= 0. || self.far <= self.near || self.far > 1_000_000. {
            return Err("Invalid camera clipping planes".into());
        }
        match self.projection {
            Projection::Orthographic { height }
                if height.is_finite() && (0.01..=100000.).contains(&height) =>
            {
                Ok(())
            }
            Projection::Perspective { fov_degrees }
                if fov_degrees.is_finite() && (1. ..=175.).contains(&fov_degrees) =>
            {
                Ok(())
            }
            _ => Err("Invalid camera projection".into()),
        }
    }
}
impl SceneObject {
    fn validate(&self, source_ids: &[u32]) -> Result<(), String> {
        if self.name.len() > 200 {
            return Err("3D object name exceeds its limit".into());
        }
        validate_transform(&self.transform)?;
        match &self.kind {
            ObjectKind::Viewport {
                primary,
                canvas,
                layers,
            } => validate_viewport(*primary, canvas, layers, source_ids),
            ObjectKind::Plane { color } | ObjectKind::Cube { color } => {
                if color
                    .iter()
                    .any(|v| !v.is_finite() || !(0. ..=1.).contains(v))
                {
                    return Err("Invalid object color".into());
                }
                Ok(())
            }
        }
    }
}
fn validate_viewport(
    primary: bool,
    canvas: &Resolution,
    layers: &[Layer],
    source_ids: &[u32],
) -> Result<(), String> {
    if !(16..=7680).contains(&canvas.width) || !(16..=7680).contains(&canvas.height) {
        return Err("Invalid viewport canvas dimensions".into());
    }
    if u64::from(canvas.width) * u64::from(canvas.height) > 3840 * 2160 {
        return Err("Viewport canvas pixel budget exceeded".into());
    }
    if layers.len() > 32 || (primary && !layers.is_empty()) {
        return Err("Invalid viewport layer count".into());
    }
    let mut ids = std::collections::HashSet::new();
    for layer in layers {
        if layer.id == 0 || !ids.insert(layer.id) {
            return Err("Invalid viewport layer identity".into());
        }
        validate_viewport_layer(layer, source_ids)?;
    }
    Ok(())
}
fn validate_viewport_layer(layer: &Layer, source_ids: &[u32]) -> Result<(), String> {
    if !source_ids.contains(&layer.source_id) {
        return Err("Viewport references missing source".into());
    }
    if layer.width == 0 || layer.height == 0 {
        return Err("Invalid viewport layer geometry".into());
    }
    if layer.effects.len() > 8 || layer.effects.iter().any(|e| e.amount > e.kind.max()) {
        return Err("Invalid viewport layer effect limits".into());
    }
    if !layer.crop.valid(&Resolution {
        width: 7680,
        height: 7680,
    }) {
        return Err("Invalid viewport crop".into());
    }
    crate::appearance::validate_css(&layer.css)?;
    for effect in &layer.effects {
        if effect.enabled && effect.kind == crate::layer::EffectKind::Wgsl {
            crate::appearance::validate_shader(&effect.shader)?;
        }
    }
    Ok(())
}
fn validate_transform(t: &Transform) -> Result<(), String> {
    if t.position
        .iter()
        .chain(&t.rotation)
        .any(|v| !v.is_finite() || v.abs() > 1_000_000.)
        || t.scale
            .iter()
            .any(|v| !v.is_finite() || *v < 0.0001 || *v > 100000.)
    {
        return Err("Invalid 3D transform".into());
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn default_camera_and_viewport_preserve_legacy_canvas() {
        let size = Resolution {
            width: 1920,
            height: 1080,
        };
        let scene = SpatialScene::from_canvas(&size);
        scene.validate(&[]).unwrap();
        assert_eq!(scene.objects[0].transform.scale, [1920., 1080., 1.]);
        assert_eq!(
            scene.camera.projection,
            Projection::Orthographic { height: 1080. }
        );
    }
    #[test]
    fn rejects_duplicate_objects_nonfinite_transforms_and_invalid_cameras() {
        let mut scene = SpatialScene::from_canvas(&Resolution {
            width: 100,
            height: 100,
        });
        scene.objects.push(scene.objects[0].clone());
        assert!(scene.validate(&[]).is_err());
        scene.objects.pop();
        scene.objects[0].transform.scale[0] = f32::NAN;
        assert!(scene.validate(&[]).is_err());
        scene.objects[0].transform.scale[0] = 100.;
        scene.camera.near = scene.camera.far;
        assert!(scene.validate(&[]).is_err());
    }
}
