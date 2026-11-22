//! Reuse GPU textures rather than growing Blitz's URL/image caches per snapshot.
use anyrender::{PaintRef, PaintScene, RenderContext, ResourceId, Scene};
use blitz_dom::{Widget, node::ComputedStyles};
use dioxus_native::{CustomWidgetAttr, prelude::*};
use peniko::{
    Fill, ImageBrush, ImageSampler,
    kurbo::{Affine, Rect},
};
use std::sync::{Arc, Mutex};
use web_time::{Duration, Instant};
use wgpu_context::DeviceHandle;

struct Frame {
    sequence: u64,
    width: u32,
    height: u32,
    rgba: Vec<u8>,
}
type Latest = Arc<Mutex<Option<Arc<Frame>>>>;
const MAX_SNAPSHOT_BYTES: usize = 32 * 1024 * 1024;
pub(crate) const DEFAULT_PREVIEW_FPS: u32 = 10;
pub(crate) const MAX_PREVIEW_FPS: u32 = 60;

type PreviewPreferences = Signal<crate::workspaces::Catalog>;

pub(crate) fn preview_fps(preferences: Option<PreviewPreferences>) -> u32 {
    preferences.map_or(DEFAULT_PREVIEW_FPS, |catalog| catalog.peek().preview_fps())
}

pub(crate) async fn wait_for_next_snapshot(started: Instant, fps: u32, received: bool) {
    let target = if received {
        Duration::from_secs_f64(1. / f64::from(fps))
    } else {
        Duration::from_millis(500)
    };
    let remaining = target.saturating_sub(started.elapsed());
    #[cfg(target_arch = "wasm32")]
    crate::scene_preview::pause((remaining.as_secs_f64() * 1000.).ceil() as u32).await;
    #[cfg(not(target_arch = "wasm32"))]
    tokio::time::sleep(remaining).await;
}

#[component]
pub(crate) fn PreviewRate() -> Element {
    let Some(mut catalog) = try_consume_context::<PreviewPreferences>() else {
        return rsx! {};
    };
    rsx! {
        section {class:"panel",
            div {class:"panel-heading",h2 {"Preview"}}
            div {class:"padded",
                crate::slider::Slider {
                    label:"Preview FPS",unit:" FPS",value:catalog.read().preview_fps() as i32,
                    min:1,max:MAX_PREVIEW_FPS as i32,reset:Some(DEFAULT_PREVIEW_FPS as i32),
                    change:move |fps:i32| catalog.write().set_preview_fps(fps as u32),
                }
                p {class:"form-help","Target refresh rate for video previews. Actual FPS depends on the source, server and connection."}
            }
        }
    }
}

#[component]
pub fn VideoPreview(
    url: String,
    #[props(default = "live-preview")] class: &'static str,
) -> Element {
    let preferences = try_consume_context::<PreviewPreferences>();
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
            let client = crate::server_access::client();
            loop {
                let started = Instant::now();
                let result = download_rgb(&client, &url).await.map(|image| {
                    let rgba = image::DynamicImage::ImageRgb8(image).to_rgba8();
                    (rgba.width(), rgba.height(), rgba.into_raw())
                });
                let received = result.is_some();
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
                wait_for_next_snapshot(started, preview_fps(preferences), received).await;
            }
        }
    }));
    rsx! {object {class,"data":widget,"data-frame":"{version}",role:"img","aria-label":"Live video snapshot"}}
}

pub(crate) async fn download_rgb(client: &reqwest::Client, url: &str) -> Option<image::RgbImage> {
    download_snapshot(client, url).await.map(|(image, _)| image)
}

pub(crate) async fn download_snapshot(
    client: &reqwest::Client,
    url: &str,
) -> Option<(image::RgbImage, acheflow_media_api::workspace::Resolution)> {
    let response = crate::server_access::authorize(client.get(url), url)
        .ok()?
        .header(reqwest::header::CACHE_CONTROL, "no-cache")
        .timeout(std::time::Duration::from_secs(3))
        .send()
        .await
        .ok()?
        .error_for_status()
        .ok()?;
    let dimension = |name: &str| {
        response
            .headers()
            .get(name)?
            .to_str()
            .ok()?
            .parse::<u32>()
            .ok()
    };
    let native_size = dimension("x-frame-width").zip(dimension("x-frame-height"));
    if response
        .content_length()
        .is_some_and(|n| n > MAX_SNAPSHOT_BYTES as u64)
    {
        return None;
    }
    let bytes = response.bytes().await.ok()?;
    if bytes.len() > MAX_SNAPSHOT_BYTES {
        return None;
    }
    decode_snapshot(&bytes, native_size)
}

fn decode_snapshot(
    bytes: &[u8],
    native_size: Option<(u32, u32)>,
) -> Option<(image::RgbImage, acheflow_media_api::workspace::Resolution)> {
    let mut reader =
        image::ImageReader::with_format(std::io::Cursor::new(bytes), image::ImageFormat::Jpeg);
    let mut limits = image::Limits::default();
    // The server sends native-resolution JPEGs, including portrait and 4K frames.
    limits.max_image_width = Some(7680);
    limits.max_image_height = Some(7680);
    limits.max_alloc = Some(256 * 1024 * 1024);
    reader.limits(limits);
    let image = reader.decode().ok()?.to_rgb8();
    // Older servers have no dimension headers; retain their snapshot aspect ratio.
    let (width, height) = native_size.unwrap_or_else(|| image.dimensions());
    if width == 0 || height == 0 || width > 7680 || height > 7680 {
        return None;
    }
    Some((
        image,
        acheflow_media_api::workspace::Resolution { width, height },
    ))
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

#[cfg(test)]
mod tests {
    use super::*;

    fn jpeg(width: u32, height: u32) -> Vec<u8> {
        let image = image::RgbImage::from_pixel(width, height, image::Rgb([210, 50, 30]));
        let mut bytes = Vec::new();
        image::codecs::jpeg::JpegEncoder::new_with_quality(&mut bytes, 95)
            .encode_image(&image)
            .unwrap();
        bytes
    }

    #[test]
    fn native_hd_and_4k_snapshots_keep_their_pixels_and_dimensions() {
        for (width, height) in [(1920, 1080), (1080, 1920), (3840, 2160), (2160, 3840)] {
            let (image, size) = decode_snapshot(&jpeg(width, height), Some((width, height)))
                .expect("native server snapshots must reach source and studio previews");
            assert_eq!(image.dimensions(), (width, height));
            assert_eq!((size.width, size.height), (width, height));
            let pixel = image.get_pixel(width / 2, height / 2).0;
            assert!(pixel[0] > 190 && pixel[1] < 70 && pixel[2] < 50);
        }
    }

    #[test]
    fn legacy_snapshots_keep_native_headers_or_fall_back_to_jpeg_dimensions() {
        let bytes = jpeg(480, 270);
        for (headers, expected) in [(Some((1920, 1080)), (1920, 1080)), (None, (480, 270))] {
            let (image, size) = decode_snapshot(&bytes, headers).unwrap();
            assert_eq!(image.dimensions(), (480, 270));
            assert_eq!((size.width, size.height), expected);
        }
    }

    #[test]
    fn malformed_and_oversized_snapshot_dimensions_are_rejected() {
        assert!(decode_snapshot(b"invalid JPEG", None).is_none());
        assert!(decode_snapshot(&jpeg(7681, 1), None).is_none());
        let bytes = jpeg(16, 16);
        for size in [(0, 16), (16, 0), (7681, 16), (16, 7681)] {
            assert!(decode_snapshot(&bytes, Some(size)).is_none());
        }
    }
}
