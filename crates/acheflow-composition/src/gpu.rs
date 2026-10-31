//! One layer renderer for native/WASM editing and headless server composition.
use crate::filters::{Filters, texture};
use acheflow_media_api::{
    appearance::{default_layer_css, layer_stylesheet, validate_css},
    workspace::{Layer, Resolution, Scene as MediaScene},
};
use anyrender::{PaintScene, RenderContext, ResourceId, Scene};
use blitz_dom::CopyStyleGetters;
use blitz_dom::{DocumentConfig, Widget, node::ComputedStyles};
use blitz_html::HtmlDocument;
use blitz_traits::shell::{ColorScheme, Viewport};
use image::RgbImage;
use peniko::{
    Fill, ImageBrush, ImageSampler,
    kurbo::{Affine, Rect},
};
use std::{
    collections::HashMap,
    sync::{Arc, Mutex},
};

#[derive(Clone)]
pub struct SourceFrame {
    pub image: Arc<RgbImage>,
    pub size: Resolution,
}
#[derive(Clone)]
pub struct SceneFrame {
    pub scene: MediaScene,
    pub size: Resolution,
    pub inputs: HashMap<u32, SourceFrame>,
    pub time: f64,
}
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct LayerBox {
    pub x: f32,
    pub y: f32,
    pub width: f32,
    pub height: f32,
}
#[derive(Clone)]
struct ImageSlot {
    id: ResourceId,
    width: u32,
    height: u32,
    intrinsic: [f64; 2],
}
type Slot = Arc<Mutex<Option<ImageSlot>>>;
struct VideoWidget {
    slot: Slot,
}
impl Widget for VideoWidget {
    fn paint(
        &mut self,
        _ctx: &mut dyn RenderContext,
        styles: &ComputedStyles,
        width: u32,
        height: u32,
        _scale: f64,
    ) -> Scene {
        let mut scene = Scene::new();
        let Some(image) = self.slot.lock().unwrap().clone() else {
            return scene;
        };
        let (w, h) = (f64::from(width), f64::from(height));
        let (iw, ih) = (image.intrinsic[0], image.intrinsic[1]);
        use style::properties::generated::longhands::object_fit::computed_value::T as Fit;
        let scale = match styles.clone_object_fit() {
            Fit::Contain => (w / iw).min(h / ih),
            Fit::Cover => (w / iw).max(h / ih),
            Fit::None => 1.,
            Fit::ScaleDown => 1f64.min((w / iw).min(h / ih)),
            _ => 0.,
        };
        let (pw, ph) = if scale == 0. {
            (w, h)
        } else {
            (iw * scale, ih * scale)
        };
        let position = styles.get_object_position();
        let x = position
            .horizontal
            .resolve(style::values::computed::CSSPixelLength::new(
                (w - pw) as f32,
            ))
            .px() as f64;
        let y = position
            .vertical
            .resolve(style::values::computed::CSSPixelLength::new(
                (h - ph) as f32,
            ))
            .px() as f64;
        scene.push_clip_layer(Fill::NonZero, Affine::IDENTITY, &Rect::new(0., 0., w, h));
        scene.fill(
            Fill::NonZero,
            Affine::scale_non_uniform(pw / f64::from(image.width), ph / f64::from(image.height))
                .then_translate((x, y).into()),
            anyrender::PaintRef::Resource(ImageBrush {
                image: image.id,
                sampler: ImageSampler::default(),
            }),
            None,
            &Rect::new(0., 0., f64::from(image.width), f64::from(image.height)),
        );
        scene.pop_layer();
        scene
    }
}
struct LayerGpu {
    layer: Option<Layer>,
    canvas: Option<Resolution>,
    document: Option<HtmlDocument>,
    node: Option<blitz_dom::NodeId>,
    slot: Slot,
    input: Option<wgpu::Texture>,
    targets: Option<[wgpu::Texture; 2]>,
    ids: Vec<ResourceId>,
    source: Option<Arc<RgbImage>>,
    crop: Option<[f32; 4]>,
    filters: Filters,
}
impl LayerGpu {
    fn new(device: &wgpu::Device) -> Self {
        Self {
            layer: None,
            canvas: None,
            document: None,
            node: None,
            slot: Arc::new(Mutex::new(None)),
            input: None,
            targets: None,
            ids: vec![],
            source: None,
            crop: None,
            filters: Filters::new(device),
        }
    }
    fn document(&mut self, layer: &Layer, size: &Resolution) -> Result<(), String> {
        if self.layer.as_ref() == Some(layer) && self.canvas.as_ref() == Some(size) {
            return Ok(());
        }
        validate_css(&layer.css)?;
        let css = if layer.css.trim().is_empty() {
            default_layer_css(layer)
        } else {
            layer_stylesheet(&layer.css)
        };
        let defaults = format!(
            "html,body{{margin:0;width:100%;height:100%;background:transparent;}}body{{position:relative;overflow:hidden;}}.layer{{position:absolute;box-sizing:border-box;left:{}px;top:{}px;width:{}px;height:{}px;overflow:hidden;--crop-left:{}px;--crop-top:{}px;--crop-right:{}px;--crop-bottom:{}px;object-fit:fill;object-position:50% 50%;}}object{{display:block;width:100%;height:100%;object-fit:inherit;object-position:inherit;}}",
            layer.x,
            layer.y,
            layer.width,
            layer.height,
            layer.crop.left,
            layer.crop.top,
            layer.crop.right,
            layer.crop.bottom
        );
        let html = format!(
            "<!DOCTYPE html><html><head><style>{defaults}\n{css}</style></head><body><div class='layer'><object id='video'></object></div></body></html>"
        );
        let mut document = HtmlDocument::from_html(
            &html,
            DocumentConfig {
                viewport: Some(Viewport::new(
                    size.width,
                    size.height,
                    1.,
                    ColorScheme::Dark,
                )),
                ..Default::default()
            },
        );
        let node = document
            .query_selector(".layer")
            .map_err(|e| format!("CSS selector: {e:?}"))?
            .ok_or("Layer element missing")?;
        let video = document.query_selector("#video").unwrap().unwrap();
        document.mutate().set_custom_widget(
            video,
            Box::new(VideoWidget {
                slot: self.slot.clone(),
            }),
        );
        // All documents use the same zero-based scene timeline, even after edits.
        document.resolve(0.);
        self.document = Some(document);
        self.node = Some(node);
        self.layer = Some(layer.clone());
        self.canvas = Some(size.clone());
        // A static source must be reprocessed if an effect or crop was edited.
        self.source = None;
        Ok(())
    }
    fn release(&mut self, ctx: &mut dyn RenderContext) {
        for id in self.ids.drain(..) {
            ctx.unregister_resource(id);
        }
        self.targets = None;
        self.input = None;
        self.source = None;
    }
}

pub struct GpuScene {
    device: wgpu::Device,
    queue: wgpu::Queue,
    layers: Vec<LayerGpu>,
}
impl GpuScene {
    pub fn new(device: wgpu::Device, queue: wgpu::Queue) -> Self {
        Self {
            device,
            queue,
            layers: vec![],
        }
    }
    pub fn prepare(
        &mut self,
        ctx: &mut dyn RenderContext,
        frame: &SceneFrame,
    ) -> Result<Vec<LayerBox>, String> {
        while self.layers.len() > frame.scene.layers.len() {
            self.layers.pop().unwrap().release(ctx);
        }
        while self.layers.len() < frame.scene.layers.len() {
            self.layers.push(LayerGpu::new(&self.device));
        }
        let mut boxes = Vec::with_capacity(self.layers.len());
        for (index, (gpu, layer)) in self.layers.iter_mut().zip(&frame.scene.layers).enumerate() {
            gpu.document(layer, &frame.size)
                .map_err(|e| format!("Layer {}: {e}", index + 1))?;
            gpu.filters
                .configure(&self.device, &layer.effects)
                .map_err(|e| format!("Layer {}: {e}", index + 1))?;
            let document = gpu.document.as_mut().unwrap();
            document.resolve(frame.time);
            let node = gpu.node.unwrap();
            let layout = document.get_node(node).unwrap().final_layout();
            boxes.push(LayerBox {
                x: layout.location.x,
                y: layout.location.y,
                width: layout.size.width,
                height: layout.size.height,
            });
            let Some(source) = frame.inputs.get(&layer.source_id) else {
                *gpu.slot.lock().unwrap() = None;
                gpu.source = None;
                continue;
            };
            let left = crop_value(
                &document.resolved_style_value(node, "--crop-left"),
                source.size.width,
            )?;
            let top = crop_value(
                &document.resolved_style_value(node, "--crop-top"),
                source.size.height,
            )?;
            let right = crop_value(
                &document.resolved_style_value(node, "--crop-right"),
                source.size.width,
            )?;
            let bottom = crop_value(
                &document.resolved_style_value(node, "--crop-bottom"),
                source.size.height,
            )?;
            if left + right >= 1. || top + bottom >= 1. {
                return Err(format!(
                    "Layer {}: crop removes the entire source",
                    index + 1
                ));
            }
            let crop = [left, top, 1. - right, 1. - bottom];
            let (w, h) = (source.image.width(), source.image.height());
            let (cw, ch) = (
                ((1. - left - right) * w as f32).round().max(1.) as u32,
                ((1. - top - bottom) * h as f32).round().max(1.) as u32,
            );
            let replace = gpu
                .input
                .as_ref()
                .is_none_or(|t| t.width() != w || t.height() != h)
                || gpu
                    .targets
                    .as_ref()
                    .is_none_or(|t| t[0].width() != cw || t[0].height() != ch);
            if replace {
                gpu.release(ctx);
                gpu.input = Some(texture(&self.device, w, h));
                gpu.targets = Some([texture(&self.device, cw, ch), texture(&self.device, cw, ch)]);
                for target in gpu.targets.as_ref().unwrap() {
                    gpu.ids.push(
                        ctx.try_register_custom_resource(Box::new(target.clone()))
                            .map_err(|e| format!("GPU texture: {e:?}"))?,
                    );
                }
            }
            let changed = gpu
                .source
                .as_ref()
                .is_none_or(|old| !Arc::ptr_eq(old, &source.image));
            if changed {
                let rgba: Vec<u8> = source
                    .image
                    .pixels()
                    .flat_map(|p| [p[0], p[1], p[2], 255])
                    .collect();
                self.queue.write_texture(
                    wgpu::TexelCopyTextureInfo {
                        texture: gpu.input.as_ref().unwrap(),
                        mip_level: 0,
                        origin: wgpu::Origin3d::ZERO,
                        aspect: wgpu::TextureAspect::All,
                    },
                    &rgba,
                    wgpu::TexelCopyBufferLayout {
                        offset: 0,
                        bytes_per_row: Some(w * 4),
                        rows_per_image: Some(h),
                    },
                    wgpu::Extent3d {
                        width: w,
                        height: h,
                        depth_or_array_layers: 1,
                    },
                );
            }
            let current = if changed
                || gpu.crop != Some(crop)
                || layer
                    .effects
                    .iter()
                    .any(|e| e.enabled && e.kind == acheflow_media_api::workspace::EffectKind::Wgsl)
            {
                gpu.filters.run(
                    &self.device,
                    &self.queue,
                    gpu.input.as_ref().unwrap(),
                    gpu.targets.as_ref().unwrap(),
                    crate::filters::PassParams {
                        crop,
                        time: frame.time as f32,
                        source_size: [source.size.width as f32, source.size.height as f32],
                    },
                )
            } else {
                let slot = gpu.slot.lock().unwrap();
                slot.as_ref()
                    .and_then(|s| gpu.ids.iter().position(|id| *id == s.id))
                    .unwrap_or(0)
            };
            *gpu.slot.lock().unwrap() = Some(ImageSlot {
                id: gpu.ids[current],
                width: cw,
                height: ch,
                intrinsic: [
                    (1. - left - right) as f64 * source.size.width as f64,
                    (1. - top - bottom) as f64 * source.size.height as f64,
                ],
            });
            gpu.source = Some(source.image.clone());
            gpu.crop = Some(crop);
        }
        Ok(boxes)
    }
    pub fn draw(&mut self, scene: &mut impl PaintScene, size: &Resolution) {
        for gpu in &mut self.layers {
            if let Some(document) = gpu.document.as_mut() {
                blitz_paint::paint_scene(
                    scene,
                    document.as_mut(),
                    1.,
                    size.width,
                    size.height,
                    0,
                    0,
                );
            }
        }
    }
}
fn crop_value(text: &str, basis: u32) -> Result<f32, String> {
    let text = text.trim();
    if text.is_empty() {
        return Ok(0.);
    }
    let value = if let Some(p) = text.strip_suffix('%') {
        p.trim().parse::<f32>().ok().map(|p| p / 100.)
    } else {
        text.strip_suffix("px")
            .unwrap_or(text)
            .trim()
            .parse::<f32>()
            .ok()
            .map(|p| p / basis.max(1) as f32)
    };
    value
        .filter(|p| p.is_finite() && (0. ..1.).contains(p))
        .ok_or_else(|| {
            "Crop custom properties need a non-negative px or % value smaller than the source"
                .into()
        })
}
