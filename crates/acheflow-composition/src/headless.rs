//! Windowless native GPU rendering and bounded readback for FFmpeg's RGB pipe.
use crate::gpu::{GpuScene, SceneFrame};
use acheflow_media_api::workspace::Resolution;
use anyrender::ResourceId;
use anyrender_vello_hybrid::{ImageManager, VelloHybridScenePainter};
use image::RgbImage;
use rustc_hash::FxHashMap;
use vello_gpu::{
    ClearSettings, RenderSize, RenderTargetConfig, Renderer, Resources, TargetInit, TextureBindings,
};
use wgpu_context::DeviceHandle;

pub struct Headless {
    handle: DeviceHandle,
    core: GpuScene,
    renderer: Renderer,
    resources: Resources,
    scene: vello_gpu::Scene,
    texture: wgpu::Texture,
    buffer: wgpu::Buffer,
    size: Resolution,
    bindings: FxHashMap<ResourceId, wgpu::TextureView>,
    images: FxHashMap<u64, vello_common::paint::ImageId>,
}
impl Headless {
    pub fn new(size: Resolution) -> Result<Self, String> {
        let handle = pollster::block_on(DeviceHandle::new_from_compatible_surface(
            wgpu::Instance::new(wgpu::InstanceDescriptor::new_without_display_handle()),
            None,
            None,
            None,
        ))
        .map_err(|e| format!("GPU compositor initialization: {e}"))?;
        let (renderer, resources) = Renderer::new(
            &handle.device,
            &RenderTargetConfig {
                format: wgpu::TextureFormat::Rgba8Unorm,
                width: size.width as u16,
                height: size.height as u16,
            },
        );
        let scene = vello_gpu::Scene::new(size.width as u16, size.height as u16);
        let texture = handle.device.create_texture(&wgpu::TextureDescriptor {
            label: Some("Server composed frame"),
            size: wgpu::Extent3d {
                width: size.width,
                height: size.height,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rgba8Unorm,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
            view_formats: &[],
        });
        let buffer = handle.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("FFmpeg frame readback"),
            size: u64::from((size.width * 4).next_multiple_of(256)) * u64::from(size.height),
            usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
            mapped_at_creation: false,
        });
        Ok(Self {
            core: GpuScene::new(handle.device.clone(), handle.queue.clone()),
            handle,
            renderer,
            resources,
            scene,
            texture,
            buffer,
            size,
            bindings: FxHashMap::default(),
            images: FxHashMap::default(),
        })
    }
    pub fn backend(&self) -> String {
        format!("wgpu · {}", self.handle.adapter.get_info().name)
    }
    pub fn render(&mut self, frame: &SceneFrame) -> Result<RgbImage, String> {
        if frame.size != self.size {
            return Err("GPU canvas size changed; restart the format".into());
        }
        self.scene.reset();
        let mut encoder =
            self.handle
                .device
                .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                    label: Some("Headless HTML/CSS composition"),
                });
        {
            let images = ImageManager::new(
                &mut self.renderer,
                &mut self.resources,
                &self.handle.device,
                &self.handle.queue,
                &mut encoder,
                &mut self.images,
            );
            let mut painter = VelloHybridScenePainter::new(
                &mut self.scene,
                images,
                &mut self.bindings,
                &self.handle,
            );
            self.core.prepare(&mut painter, frame)?;
            self.core.draw(&mut painter, &self.size);
        }
        let mut bindings = TextureBindings::new();
        for (id, view) in &self.bindings {
            bindings.insert(vello_common::TextureId(id.into_ffi()), view.clone());
        }
        self.renderer
            .render(
                &self.scene,
                &mut self.resources,
                &self.handle.device,
                &self.handle.queue,
                &mut encoder,
                &RenderSize {
                    width: self.size.width as u16,
                    height: self.size.height as u16,
                },
                &self.texture.create_view(&Default::default()),
                None,
                &bindings,
                TargetInit::Clear(ClearSettings::default()),
            )
            .map_err(|e| format!("GPU composition: {e}"))?;
        let stride = (self.size.width * 4).next_multiple_of(256);
        encoder.copy_texture_to_buffer(
            wgpu::TexelCopyTextureInfo {
                texture: &self.texture,
                mip_level: 0,
                origin: wgpu::Origin3d::ZERO,
                aspect: wgpu::TextureAspect::All,
            },
            wgpu::TexelCopyBufferInfo {
                buffer: &self.buffer,
                layout: wgpu::TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(stride),
                    rows_per_image: Some(self.size.height),
                },
            },
            wgpu::Extent3d {
                width: self.size.width,
                height: self.size.height,
                depth_or_array_layers: 1,
            },
        );
        self.handle.queue.submit([encoder.finish()]);
        let (tx, rx) = std::sync::mpsc::sync_channel(1);
        self.buffer
            .slice(..)
            .map_async(wgpu::MapMode::Read, move |result| {
                let _ = tx.send(result);
            });
        self.handle
            .device
            .poll(wgpu::PollType::wait_indefinitely())
            .map_err(|e| format!("GPU readback: {e}"))?;
        rx.recv()
            .map_err(|e| e.to_string())?
            .map_err(|e| format!("GPU frame map: {e}"))?;
        let data = self
            .buffer
            .slice(..)
            .get_mapped_range()
            .map_err(|e| format!("GPU mapped range: {e}"))?;
        let image = RgbImage::from_fn(self.size.width, self.size.height, |x, y| {
            let i = (y * stride + x * 4) as usize;
            image::Rgb([data[i], data[i + 1], data[i + 2]])
        });
        drop(data);
        self.buffer.unmap();
        Ok(image)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::gpu::SourceFrame;
    use acheflow_media_api::workspace::{EffectKind, LayerEffect, Orientation, Workspace};
    use std::{collections::HashMap, sync::Arc};
    #[test]
    #[ignore = "requires an actual GPU adapter; run explicitly on the integration host"]
    fn gpu_css_crop_fit_shader_animation_and_bounded_textures() {
        let size = Resolution {
            width: 128,
            height: 64,
        };
        let mut w = Workspace::default();
        w.add_source();
        w.sources[0].resolution = Resolution {
            width: 16,
            height: 8,
        };
        let id = w.add_scene(Orientation::Horizontal);
        w.add_layer(id, 1);
        let source = RgbImage::from_fn(16, 8, |x, _| {
            if x < 8 {
                image::Rgb([255, 0, 0])
            } else {
                image::Rgb([0, 0, 255])
            }
        });
        let mut frame = SceneFrame {
            scene: w.scenes[0].clone(),
            size: size.clone(),
            inputs: HashMap::from([(
                1,
                SourceFrame {
                    image: Arc::new(source),
                    size: w.sources[0].resolution.clone(),
                },
            )]),
            time: 0.,
        };
        frame.scene.layers[0].css=".layer {left:8px;top:8px;width:32px;height:32px;border:4px solid #00ff00;--crop-left:50%;}".into();
        let mut gpu = Headless::new(size).unwrap();
        let image = gpu.render(&frame).unwrap();
        assert!(
            image.get_pixel(9, 9)[1] > 240,
            "CSS border: {:?}",
            image.get_pixel(9, 9)
        );
        assert!(
            image.get_pixel(20, 20)[2] > 240,
            "crop selects blue: {:?}",
            image.get_pixel(20, 20)
        );
        frame.scene.layers[0].css =
            "left:8px;top:8px;width:32px;height:32px;object-fit:contain;".into();
        let image = gpu.render(&frame).unwrap();
        assert_eq!(image.get_pixel(16, 9).0, [0, 0, 0], "contain letterbox");
        assert!(image.get_pixel(10, 24)[0] > 240, "contain image");
        frame.scene.layers[0].css =
            "left:8px;top:8px;width:32px;height:32px;object-fit:cover;object-position:100% 50%;"
                .into();
        let image = gpu.render(&frame).unwrap();
        assert!(image.get_pixel(16, 16)[2] > 240, "cover aligns right side");
        frame.scene.layers[0].css =
            "left:8px;top:8px;width:32px;height:32px;--crop-left:8px;".into();
        frame.scene.layers[0].effects.push(LayerEffect {kind:EffectKind::Wgsl,amount:100,enabled:true,shader:"fn effect(uv:vec2f,color:vec4f)->vec4f {return vec4f(vec3f(1.)-color.rgb,color.a);}".into()});
        let image = gpu.render(&frame).unwrap();
        assert!(
            image.get_pixel(20, 20)[0] > 240
                && image.get_pixel(20, 20)[1] > 240
                && image.get_pixel(20, 20)[2] < 10,
            "custom WGSL invert: {:?}",
            image.get_pixel(20, 20)
        );
        frame.scene.layers[0].css=".layer {left:8px;top:8px;width:32px;height:32px;--crop-left:8px;animation:fade 2s linear infinite alternate;} @keyframes fade {from {opacity:0;}to {opacity:1;}}".into();
        frame.time = 1.;
        let half = gpu.render(&frame).unwrap();
        frame.time = 2.;
        let full = gpu.render(&frame).unwrap();
        assert!(
            (100..160).contains(&half.get_pixel(20, 20)[0]),
            "scene-clock animation: {:?}",
            half.get_pixel(20, 20)
        );
        assert!(full.get_pixel(20, 20)[0] > 240);
        for _ in 0..30 {
            gpu.render(&frame).unwrap();
        }
        assert_eq!(gpu.bindings.len(), 2, "two reusable textures per layer");
    }
}
