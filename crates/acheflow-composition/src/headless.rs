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
    spatial: Option<crate::spatial::SpatialRenderer>,
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
            spatial: None,
        })
    }
    pub fn backend(&self) -> String {
        format!("wgpu · {}", self.handle.adapter.get_info().name)
    }
    pub fn render(&mut self, frame: &SceneFrame) -> Result<RgbImage, String> {
        if frame.size != self.size {
            return Err("GPU canvas size changed; restart the format".into());
        }
        let composed = if frame.scene.spatial.is_some() {
            if self.spatial.is_none() {
                self.spatial = Some(crate::spatial::SpatialRenderer::new(
                    self.handle.clone(),
                    self.size.clone(),
                ));
            }
            self.spatial.as_mut().unwrap().render(frame)?
        } else {
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
            self.handle.queue.submit([encoder.finish()]);
            self.texture.clone()
        };
        let mut encoder =
            self.handle
                .device
                .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                    label: Some("Final frame readback"),
                });
        let stride = (self.size.width * 4).next_multiple_of(256);
        encoder.copy_texture_to_buffer(
            wgpu::TexelCopyTextureInfo {
                texture: &composed,
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
    #[ignore = "requires an actual GPU adapter"]
    fn spatial_default_viewport_matches_legacy_and_depth_occludes() {
        use acheflow_media_api::spatial::*;
        let size = Resolution {
            width: 64,
            height: 32,
        };
        let mut workspace = Workspace::default();
        workspace.horizontal.base = size.clone();
        workspace.add_source();
        let id = workspace.add_scene(Orientation::Horizontal);
        workspace.add_layer(id, 1);
        workspace.scenes[0].layers[0].width = 64;
        workspace.scenes[0].layers[0].height = 32;
        let image = Arc::new(RgbImage::from_fn(64, 32, |x, _| {
            if x < 32 {
                image::Rgb([255, 0, 0])
            } else {
                image::Rgb([0, 0, 255])
            }
        }));
        let mut frame = SceneFrame {
            source_documents: Default::default(),
            epoch_ms: 0,
            scene: workspace.scenes[0].clone(),
            size: size.clone(),
            inputs: HashMap::from([(
                1,
                SourceFrame {
                    image,
                    size: size.clone(),
                },
            )]),
            time: 0.,
        };
        let mut renderer = super::Headless::new(size.clone()).unwrap();
        let flat = renderer.render(&frame).unwrap();
        frame.scene.spatial = Some(SpatialScene::from_canvas(&size));
        let spatial = renderer.render(&frame).unwrap();
        assert!(
            flat.as_raw()
                .iter()
                .zip(spatial.as_raw())
                .all(|(a, b)| a.abs_diff(*b) <= 2),
            "default viewport must preserve legacy pixels"
        );
        frame
            .scene
            .spatial
            .as_mut()
            .unwrap()
            .objects
            .push(SceneObject {
                id: 2,
                name: "Cube".into(),
                visible: true,
                transform: Transform {
                    position: [0., 0., 100.],
                    scale: [16., 16., 16.],
                    ..Default::default()
                },
                kind: ObjectKind::Cube {
                    color: [0., 1., 0., 1.],
                },
            });
        let occluded = renderer.render(&frame).unwrap();
        assert_eq!(occluded.get_pixel(32, 16).0, [0, 255, 0]);
        frame.scene.spatial.as_mut().unwrap().objects[1]
            .transform
            .position[2] = -100.;
        assert_eq!(
            renderer.render(&frame).unwrap().get_pixel(32, 16),
            spatial.get_pixel(32, 16)
        );
        let content = frame.scene.layers.clone();
        frame
            .scene
            .spatial
            .as_mut()
            .unwrap()
            .objects
            .push(SceneObject {
                id: 3,
                name: "Second viewport".into(),
                visible: true,
                transform: Transform {
                    position: [16., 0., 200.],
                    scale: [32., 16., 1.],
                    ..Default::default()
                },
                kind: ObjectKind::Viewport {
                    primary: false,
                    canvas: size,
                    layers: content,
                },
            });
        let second = renderer.render(&frame).unwrap();
        assert_eq!(
            second.get_pixel(40, 16).0,
            [255, 0, 0],
            "a second viewport has its own content texture and world transform"
        );
        assert_eq!(spatial.get_pixel(40, 16).0, [0, 0, 255]);
    }
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
            source_documents: Default::default(),
            epoch_ms: 0,
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
        frame.scene.layers[0].effects.push(LayerEffect { id: acheflow_media_api::workspace::new_entity_id(),kind:EffectKind::Wgsl,amount:100,enabled:true,shader:"fn effect(uv:vec2f,color:vec4f)->vec4f {return vec4f(vec3f(1.)-color.rgb,color.a);}".into()});
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

        // A test image is an ordinary GPU input: crop, fitting, decoration and
        // WGSL keep working when the editor substitutes it for missing video.
        let pattern = SourceFrame::test_pattern(&Resolution {
            width: 320,
            height: 180,
        });
        frame.inputs.insert(1, pattern.clone());
        frame.scene.layers[0].css = "left:0;top:0;width:128px;height:64px;".into();
        frame.scene.layers[0].effects.clear();
        let pattern_pixels = gpu.render(&frame).unwrap();
        assert!(pattern_pixels.pixels().any(|p| p[0] > 180 && p[1] < 100));
        frame.scene.layers[0].effects.push(LayerEffect {
            shader: "fn effect(uv:vec2f,c:vec4f)->vec4f{return vec4f(vec3f(1.)-c.rgb,c.a);}".into(),
            ..LayerEffect::new(EffectKind::Wgsl)
        });
        let inverted = gpu.render(&frame).unwrap();
        let before = pattern_pixels.get_pixel(40, 20);
        let after = inverted.get_pixel(40, 20);
        for c in 0..3 {
            assert!((i32::from(before[c]) + i32::from(after[c]) - 255).abs() < 4);
        }
        frame.inputs.insert(
            1,
            SourceFrame {
                image: Arc::new(RgbImage::from_pixel(320, 180, image::Rgb([0, 0, 255]))),
                size: pattern.size.clone(),
            },
        );
        frame.scene.layers[0].effects.clear();
        assert!(
            gpu.render(&frame).unwrap().get_pixel(40, 20)[2] > 240,
            "video replaces pattern"
        );
        frame.inputs.insert(1, pattern);
        assert_eq!(
            gpu.render(&frame).unwrap().get_pixel(40, 20),
            pattern_pixels.get_pixel(40, 20),
            "pattern returns after disconnect"
        );
        frame.inputs.clear();
        assert_eq!(
            gpu.render(&frame).unwrap().get_pixel(40, 20).0,
            [0, 0, 0],
            "broadcast missing input stays black"
        );
    }
    #[test]
    #[ignore = "requires an actual GPU adapter"]
    fn chat_visual_sources_preserve_transparency_and_scene_clock_phases() {
        use acheflow_media_api::visual::{SourceDocument, SourceNode};
        let size = Resolution {
            width: 128,
            height: 64,
        };
        let mut workspace = Workspace::default();
        workspace.horizontal.base = size.clone();
        workspace.add_source();
        workspace.add_source();
        let scene_id = workspace.add_scene(Orientation::Horizontal);
        workspace.add_layer(scene_id, 1);
        workspace.add_layer(scene_id, 2);
        let overlay = &mut workspace.scenes[0].layers[1];
        overlay.x = 16;
        overlay.y = 8;
        overlay.width = 64;
        overlay.height = 32;
        let document=SourceDocument{css:".card {width:100%;height:100%;background:#ff0000;color:white;font-size:12px;} .entering {animation:fade-in 0.2s linear both;} .leaving {animation:fade-out 0.3s linear both;} @keyframes fade-in {from {opacity:0;}to {opacity:1;}} @keyframes fade-out {from {opacity:1;}to {opacity:0;}}".into(),nodes:vec![SourceNode{classes:"card".into(),text:"Hello".into(),animation_at_ms:Some(2000),exit_at_ms:Some(4000),remove_at_ms:Some(4300),..Default::default()}]};
        let mut frame = SceneFrame {
            scene: workspace.scenes[0].clone(),
            size: size.clone(),
            inputs: HashMap::from([(
                1,
                SourceFrame {
                    image: Arc::new(RgbImage::from_pixel(128, 64, image::Rgb([0, 0, 255]))),
                    size: size.clone(),
                },
            )]),
            source_documents: HashMap::from([(2, document)]),
            epoch_ms: 1000,
            time: 0.5,
        };
        let mut renderer = super::Headless::new(size).unwrap();
        let before = renderer.render(&frame).unwrap();
        assert_eq!(before.get_pixel(76, 36).0, [0, 0, 255]);
        frame.time = 1.5;
        let visible = renderer.render(&frame).unwrap();
        assert!(visible.get_pixel(76, 36)[0] > 240);
        assert_eq!(visible.get_pixel(2, 2).0, [0, 0, 255]);
        frame.source_documents.get_mut(&2).unwrap().nodes[0].text = "Updated".into();
        frame.time = 1.6;
        assert!(
            renderer.render(&frame).unwrap().get_pixel(76, 36)[0] > 240,
            "document changes must not restart existing entry animations"
        );
        frame.time = 3.15;
        let fading = renderer.render(&frame).unwrap();
        let pixel = fading.get_pixel(76, 36);
        assert!(
            pixel[0] > 20 && pixel[0] < 235 && pixel[2] > 20,
            "exit must blend with lower layers: {pixel:?}"
        );
        frame.time = 3.4;
        assert_eq!(
            renderer.render(&frame).unwrap().get_pixel(76, 36).0,
            [0, 0, 255]
        );
    }
}
