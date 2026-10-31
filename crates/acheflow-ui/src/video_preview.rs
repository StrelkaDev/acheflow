//! Reuse GPU textures rather than growing Blitz's URL/image caches per snapshot.
use anyrender::{PaintRef, PaintScene, RenderContext, ResourceId, Scene};
use blitz_dom::{Widget, node::ComputedStyles};
use dioxus_native::{CustomWidgetAttr, prelude::*};
use peniko::{
    Fill, ImageBrush, ImageSampler,
    kurbo::{Affine, Rect},
};
use std::sync::{Arc, Mutex};
use wgpu_context::DeviceHandle;

struct Frame {
    sequence: u64,
    width: u32,
    height: u32,
    rgba: Vec<u8>,
}
type Latest = Arc<Mutex<Option<Arc<Frame>>>>;

#[component]
pub fn VideoPreview(
    url: String,
    #[props(default = "live-preview")] class: &'static str,
) -> Element {
    let latest: Latest = use_hook(|| Arc::new(Mutex::new(None)));
    let widget_latest = latest.clone();
    let widget = use_hook(move || {
        CustomWidgetAttr::new(PreviewWidget {
            latest: widget_latest,
            gpu: None,
        })
    });
    let mut version = use_signal(|| 0u64);
    use_resource(use_reactive((&url,), move |(url,)| {
        let latest = latest.clone();
        async move {
            let client = reqwest::Client::new();
            loop {
                let result = download_rgb(&client, &url).await.map(|image| {
                    let rgba = image::DynamicImage::ImageRgb8(image).to_rgba8();
                    (rgba.width(), rgba.height(), rgba.into_raw())
                });
                let sequence = *version.peek() + 1;
                *latest.lock().unwrap() = result.map(|(width, height, rgba)| {
                    Arc::new(Frame {
                        sequence,
                        width,
                        height,
                        rgba,
                    })
                });
                version.set(sequence);
                #[cfg(target_arch = "wasm32")]
                gloo_timers::future::TimeoutFuture::new(500).await;
                #[cfg(not(target_arch = "wasm32"))]
                tokio::time::sleep(std::time::Duration::from_millis(500)).await;
            }
        }
    }));
    rsx! {object {class,"data":widget,"data-frame":"{version}",role:"img","aria-label":"Live video snapshot"}}
}

pub(crate) async fn download_rgb(client: &reqwest::Client, url: &str) -> Option<image::RgbImage> {
    let response = client
        .get(url)
        .header(reqwest::header::CACHE_CONTROL, "no-cache")
        .timeout(std::time::Duration::from_secs(3))
        .send()
        .await
        .ok()?
        .error_for_status()
        .ok()?;
    if response
        .content_length()
        .is_some_and(|n| n > 2 * 1024 * 1024)
    {
        return None;
    }
    let bytes = response.bytes().await.ok()?;
    if bytes.len() > 2 * 1024 * 1024 {
        return None;
    }
    let mut reader =
        image::ImageReader::with_format(std::io::Cursor::new(bytes), image::ImageFormat::Jpeg);
    let mut limits = image::Limits::default();
    limits.max_image_width = Some(480);
    limits.max_image_height = Some(854);
    limits.max_alloc = Some(16 * 1024 * 1024);
    reader.limits(limits);
    Some(reader.decode().ok()?.to_rgb8())
}

struct Target {
    texture: wgpu::Texture,
    id: ResourceId,
}
struct Gpu {
    device: wgpu::Device,
    queue: wgpu::Queue,
    targets: Vec<Target>,
    sequence: u64,
    current: usize,
}
struct PreviewWidget {
    latest: Latest,
    gpu: Option<Gpu>,
}

impl Widget for PreviewWidget {
    fn can_create_surfaces(&mut self, ctx: &mut dyn RenderContext) {
        if let Some(handle) = ctx
            .renderer_specific_context()
            .and_then(|c| c.downcast::<DeviceHandle>().ok())
        {
            self.gpu = Some(Gpu {
                device: handle.device.clone(),
                queue: handle.queue.clone(),
                targets: vec![],
                sequence: 0,
                current: 0,
            });
        }
    }
    fn destroy_surfaces(&mut self) {
        self.gpu = None;
    }
    fn requires_redraw(&self) -> bool {
        self.latest.lock().unwrap().as_ref().is_some_and(|frame| {
            self.gpu
                .as_ref()
                .is_none_or(|gpu| gpu.sequence != frame.sequence)
        })
    }
    fn paint(
        &mut self,
        ctx: &mut dyn RenderContext,
        _styles: &ComputedStyles,
        width: u32,
        height: u32,
        _scale: f64,
    ) -> Scene {
        let mut scene = Scene::new();
        let Some(frame) = self.latest.lock().unwrap().clone() else {
            return scene;
        };
        if width == 0 || height == 0 {
            return scene;
        }
        if self.gpu.is_none() {
            self.can_create_surfaces(ctx);
        }
        let Some(gpu) = self.gpu.as_mut() else {
            return scene;
        };
        if gpu
            .targets
            .first()
            .is_some_and(|t| t.texture.width() != frame.width || t.texture.height() != frame.height)
        {
            for target in gpu.targets.drain(..) {
                ctx.unregister_resource(target.id);
            }
            gpu.sequence = 0;
        }
        if gpu.targets.is_empty() {
            for _ in 0..2 {
                let texture = gpu.device.create_texture(&wgpu::TextureDescriptor {
                    label: Some("Live preview reusable frame"),
                    size: wgpu::Extent3d {
                        width: frame.width,
                        height: frame.height,
                        depth_or_array_layers: 1,
                    },
                    mip_level_count: 1,
                    sample_count: 1,
                    dimension: wgpu::TextureDimension::D2,
                    format: wgpu::TextureFormat::Rgba8Unorm,
                    usage: wgpu::TextureUsages::TEXTURE_BINDING
                        | wgpu::TextureUsages::COPY_DST
                        | wgpu::TextureUsages::COPY_SRC,
                    view_formats: &[],
                });
                let Ok(id) = ctx.try_register_custom_resource(Box::new(texture.clone())) else {
                    return scene;
                };
                gpu.targets.push(Target { texture, id });
            }
        }
        if gpu.sequence != frame.sequence {
            gpu.current = (gpu.current + 1) % gpu.targets.len();
            gpu.queue.write_texture(
                wgpu::TexelCopyTextureInfo {
                    texture: &gpu.targets[gpu.current].texture,
                    mip_level: 0,
                    origin: wgpu::Origin3d::ZERO,
                    aspect: wgpu::TextureAspect::All,
                },
                &frame.rgba,
                wgpu::TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(frame.width * 4),
                    rows_per_image: Some(frame.height),
                },
                wgpu::Extent3d {
                    width: frame.width,
                    height: frame.height,
                    depth_or_array_layers: 1,
                },
            );
            gpu.sequence = frame.sequence;
        }
        scene.fill(
            Fill::NonZero,
            Affine::scale_non_uniform(
                width as f64 / frame.width as f64,
                height as f64 / frame.height as f64,
            ),
            PaintRef::Resource(ImageBrush {
                image: gpu.targets[gpu.current].id,
                sampler: ImageSampler::default(),
            }),
            None,
            &Rect::new(0., 0., frame.width as f64, frame.height as f64),
        );
        scene
    }
}
