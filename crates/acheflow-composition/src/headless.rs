//! Windowless native GPU rendering and bounded readback for FFmpeg's RGB pipe.
use crate::filters::validate_texture_size;
use crate::gpu::{GpuScene, SceneFrame};
use crate::gpu_profile::GpuProfile;
use acheflow_media_api::live::GpuFrameProfile;
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
    profile: Option<GpuProfile>,
}
impl Headless {
    pub fn new(size: Resolution) -> Result<Self, String> {
        validate_texture_size(size.width, size.height, u16::MAX.into())?;
        let handle = pollster::block_on(DeviceHandle::new_from_compatible_surface(
            wgpu::Instance::new(wgpu::InstanceDescriptor::new_without_display_handle()),
            None,
            Some(GpuProfile::FEATURES),
            None,
        ))
        .map_err(|e| format!("GPU compositor initialization: {e}"))?;
        let limits = handle.device.limits();
        validate_texture_size(size.width, size.height, limits.max_texture_dimension_2d)?;
        let readback_bytes =
            u64::from((size.width * 4).next_multiple_of(256)) * u64::from(size.height);
        if readback_bytes > limits.max_buffer_size {
            return Err(format!(
                "GPU canvas {}×{} exceeds the device's readback buffer limit",
                size.width, size.height
            ));
        }
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
            size: readback_bytes,
            usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
            mapped_at_creation: false,
        });
        let profile = GpuProfile::new(&handle.device);
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
            profile,
        })
    }
    pub fn backend(&self) -> String {
        format!("wgpu · {}", self.handle.adapter.get_info().name)
    }
    pub fn render(&mut self, frame: &SceneFrame) -> Result<RgbImage, String> {
        self.render_profiled(frame, false).map(|(image, _)| image)
    }

    pub fn render_profiled(
        &mut self,
        frame: &SceneFrame,
        sample_gpu: bool,
    ) -> Result<(RgbImage, Option<GpuFrameProfile>), String> {
        if frame.size != self.size {
            return Err("GPU canvas size changed; restart the format".into());
        }
        let mut profile = self.profile.as_mut().filter(|_| sample_gpu);
        if let Some(profile) = &mut profile {
            profile.reset();
        }
        let composed = if frame.scene.spatial.is_some() {
            if self.spatial.is_none() {
                self.spatial = Some(crate::spatial::SpatialRenderer::new(
                    self.handle.clone(),
                    self.size.clone(),
                )?);
            }
            self.spatial
                .as_mut()
                .unwrap()
                .render_profiled(frame, profile.as_deref_mut())?
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
                self.core
                    .prepare_profiled(&mut painter, frame, profile.as_deref_mut(), None)?;
                self.core.draw(&mut painter, &self.size);
            }
            let mut bindings = TextureBindings::new();
            for (id, view) in &self.bindings {
                bindings.insert(vello_common::TextureId(id.into_ffi()), view.clone());
            }
            let end = profile
                .as_mut()
                .and_then(|profile| profile.begin(&mut encoder, "Scene composition", None, None));
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
            if let Some(profile) = &profile {
                profile.end(&mut encoder, end);
            }
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
        let end = profile
            .as_mut()
            .and_then(|profile| profile.begin(&mut encoder, "Frame readback copy", None, None));
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
        if let Some(profile) = &profile {
            profile.end(&mut encoder, end);
            profile.resolve(&mut encoder);
        }
        self.handle.queue.submit([encoder.finish()]);
        let timing_map = profile.as_ref().map(|profile| profile.map());
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
        let timings = profile.zip(timing_map).and_then(|(profile, map)| {
            profile.finish(
                frame.scene.id,
                self.handle.queue.get_timestamp_period(),
                matches!(map.recv(), Ok(Ok(()))),
            )
        });
        rx.recv()
            .map_err(|e| e.to_string())?
            .map_err(|e| format!("GPU frame map: {e}"))?;
        let data = self
            .buffer
            .slice(..)
            .get_mapped_range()
            .map_err(|e| format!("GPU mapped range: {e}"))?;
        let image = rgb_from_readback(&data, self.size.width, self.size.height, stride as usize);
        drop(data);
        self.buffer.unmap();
        Ok((image, timings))
    }
}

fn rgb_from_readback(data: &[u8], width: u32, height: u32, stride: usize) -> RgbImage {
    let row_bytes = width as usize * 3;
    let mut pixels = vec![0; row_bytes * height as usize];
    for (source, destination) in data
        .chunks_exact(stride)
        .zip(pixels.chunks_exact_mut(row_bytes))
    {
        let (source, _) = source.as_chunks::<4>();
        let (destination, _) = destination.as_chunks_mut::<3>();
        for (&[r, g, b, _], pixel) in source.iter().zip(destination) {
            *pixel = [r, g, b];
        }
    }
    RgbImage::from_raw(width, height, pixels).expect("RGB buffer matches readback dimensions")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::gpu::SourceFrame;
    use acheflow_media_api::workspace::{EffectKind, LayerEffect, Orientation, Workspace};
    use std::{collections::HashMap, sync::Arc};

    #[test]
    fn headless_rejects_an_empty_canvas() {
        let result = Headless::new(Resolution {
            width: 0,
            height: 32,
        });
        assert!(result.err().is_some_and(|error| error.contains("0×32")));
    }

    #[test]
    fn readback_preserves_rgb_order_and_discards_alpha_and_row_padding() {
        let first = [1, 2, 3, 111, 4, 5, 6, 112, 200, 201, 202, 203];
        let second = [7, 8, 9, 113, 10, 11, 12, 114, 204, 205, 206, 207];
        let raw = [first, second].concat();
        let image = rgb_from_readback(&raw, 2, 2, 12);
        assert_eq!(image.dimensions(), (2, 2));
        assert_eq!(image.into_raw(), (1..=12).collect::<Vec<u8>>());
    }

    #[test]
    #[ignore = "requires an actual GPU adapter"]
    fn layer_can_retry_after_partial_texture_registration() {
        use anyrender::{RegisterResourceError, RegisterResourceErrorKind, RenderContext};

        #[derive(Default)]
        struct Context {
            registered: Vec<ResourceId>,
            fail_second: bool,
        }

        impl RenderContext for Context {
            fn try_register_custom_resource(
                &mut self,
                _: Box<dyn std::any::Any>,
            ) -> Result<ResourceId, RegisterResourceError> {
                if self.fail_second && self.registered.len() == 1 {
                    return Err(RegisterResourceErrorKind::Other.into());
                }
                let id = ResourceId::new();
                self.registered.push(id);
                Ok(id)
            }

            fn unregister_resource(&mut self, id: ResourceId) {
                self.registered.retain(|&registered| registered != id);
            }
        }

        let size = Resolution {
            width: 64,
            height: 32,
        };
        let mut workspace = Workspace::default();
        workspace.horizontal.base = size.clone();
        workspace.add_source();
        let scene = workspace.add_scene(Orientation::Horizontal);
        workspace.add_layer(scene, 1);
        let frame = SceneFrame {
            size: size.clone(),
            scene: workspace.scenes.remove(0),
            inputs: HashMap::from([(1, SourceFrame::test_pattern(&size))]),
            source_documents: HashMap::new(),
            epoch_ms: 0,
            time: 0.,
        };

        let mut gpu = Headless::new(size).unwrap();
        let mut ctx = Context {
            fail_second: true,
            ..Default::default()
        };
        assert!(gpu.core.prepare(&mut ctx, &frame).is_err());
        assert!(ctx.registered.is_empty());
        ctx.fail_second = false;
        assert!(gpu.core.prepare(&mut ctx, &frame).is_ok());
        assert_eq!(ctx.registered.len(), 2);
    }

    #[test]
    #[ignore = "requires an actual GPU adapter"]
    fn chroma_key_removes_the_configured_color_and_can_change_it_live() {
        use acheflow_media_api::layer::ChromaKey;
        let size = Resolution {
            width: 64,
            height: 32,
        };
        let mut workspace = Workspace::default();
        workspace.horizontal.base = size.clone();
        workspace.add_source();
        let scene = workspace.add_scene(Orientation::Horizontal);
        workspace.add_layer(scene, 1);
        let mut effect = LayerEffect::new(EffectKind::ChromaKey);
        effect.chroma_key = Some(ChromaKey {
            color: [0, 255, 0],
            ..Default::default()
        });
        workspace.scenes[0].layers[0].effects.push(effect);
        let source = RgbImage::from_fn(64, 32, |x, _| {
            if x < 32 {
                image::Rgb([0, 255, 0])
            } else {
                image::Rgb([255, 0, 0])
            }
        });
        let mut frame = SceneFrame {
            source_documents: Default::default(),
            epoch_ms: 0,
            scene: workspace.scenes[0].clone(),
            size: size.clone(),
            inputs: HashMap::from([(
                1,
                SourceFrame {
                    image: Arc::new(source),
                    size: size.clone(),
                },
            )]),
            time: 0.,
        };
        let mut gpu = Headless::new(size).unwrap();
        let (pixels, profiling) = gpu.render_profiled(&frame, true).unwrap();
        if gpu.profile.is_some() {
            let profiling = profiling.expect("read GPU timestamps on a supported adapter");
            assert_eq!(profiling.scene_id, scene);
            assert!(!profiling.truncated);
            for stage in ["Crop", "Chroma key", "Premultiply alpha"] {
                let timing = profiling
                    .passes
                    .iter()
                    .find(|pass| {
                        pass.stage.eq_ignore_ascii_case(stage)
                            && pass.layer_id == Some(frame.scene.layers[0].id)
                    })
                    .expect("the layer ran the expected GPU pass");
                assert_eq!(timing.source_id, Some(1));
                assert!(timing.elapsed_ms.is_finite() && timing.elapsed_ms >= 0.);
            }
            assert!(
                profiling
                    .passes
                    .iter()
                    .any(|pass| pass.stage == "Scene composition")
            );
            assert!(
                profiling
                    .passes
                    .iter()
                    .any(|pass| pass.stage == "Frame readback copy")
            );
            assert!(profiling.passes.iter().any(|pass| pass.elapsed_ms > 0.));
            eprintln!(
                "Timed {} GPU passes on {}",
                profiling.passes.len(),
                gpu.backend()
            );
        } else {
            eprintln!("GPU timestamps unavailable on {}", gpu.backend());
            assert!(profiling.is_none());
        }
        assert_eq!(pixels.get_pixel(12, 16).0, [0, 0, 0]);
        assert_eq!(pixels.get_pixel(48, 16).0, [255, 0, 0]);
        frame.scene.layers[0].effects[0]
            .chroma_key
            .as_mut()
            .unwrap()
            .color = [255, 0, 0];
        let pixels = gpu.render(&frame).unwrap();
        assert_eq!(pixels.get_pixel(12, 16).0, [0, 255, 0]);
        assert_eq!(pixels.get_pixel(48, 16).0, [0, 0, 0]);
    }

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
        let (spatial, timings) = renderer.render_profiled(&frame, true).unwrap();
        if renderer.profile.is_some() {
            let timings = timings.expect("read spatial GPU timestamps");
            assert!(
                timings.passes.iter().any(|pass| pass.stage == "3D camera"
                    && pass.viewport_id.is_none()
                    && pass.elapsed_ms > 0.),
                "spatial GPU timings: {:?}",
                timings.passes
            );
            assert!(timings.passes.iter().any(|pass| pass.stage == "Viewport composition" && pass.viewport_id == Some(1)));
            assert!(
                timings
                    .passes
                    .iter()
                    .any(|pass| pass.stage == "Crop" && pass.viewport_id == Some(1))
            );
        }
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
        frame.scene.layers[0].effects.push(LayerEffect {
                chroma_key: None, id: acheflow_media_api::workspace::new_entity_id(),kind:EffectKind::Wgsl,amount:100,enabled:true,shader:"fn effect(uv:vec2f,color:vec4f)->vec4f {return vec4f(vec3f(1.)-color.rgb,color.a);}".into()});
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
            chroma_key: None,
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
