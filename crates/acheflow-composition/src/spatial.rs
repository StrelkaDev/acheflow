//! Shared native/web/headless 3D pass over GPU-rendered 2D viewport textures.
use crate::gpu::{GpuScene, SceneFrame};
use acheflow_media_api::{
    spatial::{Camera, ObjectKind, Projection, Transform},
    workspace::Resolution,
};
use anyrender::ResourceId;
use anyrender_vello_hybrid::{ImageManager, VelloHybridScenePainter};
use glam::{EulerRot, Mat4, Quat, Vec3};
use rustc_hash::FxHashMap;
use std::collections::HashMap;
use vello_gpu::{
    ClearSettings, RenderSize, RenderTargetConfig, Renderer, Resources, TargetInit, TextureBindings,
};
use wgpu::util::DeviceExt;
use wgpu_context::DeviceHandle;

struct Viewport {
    core: GpuScene,
    renderer: Renderer,
    resources: Resources,
    scene: vello_gpu::Scene,
    texture: wgpu::Texture,
    bindings: FxHashMap<ResourceId, wgpu::TextureView>,
    images: FxHashMap<u64, vello_common::paint::ImageId>,
    size: Resolution,
}
impl Viewport {
    fn new(handle: &DeviceHandle, size: Resolution) -> Self {
        let (renderer, resources) = Renderer::new(
            &handle.device,
            &RenderTargetConfig {
                format: wgpu::TextureFormat::Rgba8Unorm,
                width: size.width as u16,
                height: size.height as u16,
            },
        );
        Self {
            core: GpuScene::new(handle.device.clone(), handle.queue.clone()),
            renderer,
            resources,
            scene: vello_gpu::Scene::new(size.width as u16, size.height as u16),
            texture: crate::filters::texture(&handle.device, size.width, size.height),
            bindings: FxHashMap::default(),
            images: FxHashMap::default(),
            size,
        }
    }
    fn render(&mut self, handle: &DeviceHandle, frame: &SceneFrame) -> Result<(), String> {
        self.scene.reset();
        let mut encoder = handle
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("Viewport composition"),
            });
        {
            let images = ImageManager::new(
                &mut self.renderer,
                &mut self.resources,
                &handle.device,
                &handle.queue,
                &mut encoder,
                &mut self.images,
            );
            let mut painter =
                VelloHybridScenePainter::new(&mut self.scene, images, &mut self.bindings, handle);
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
                &handle.device,
                &handle.queue,
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
            .map_err(|e| format!("Viewport render: {e}"))?;
        handle.queue.submit([encoder.finish()]);
        Ok(())
    }
}

pub struct SpatialRenderer {
    handle: DeviceHandle,
    size: Resolution,
    output: wgpu::Texture,
    depth: wgpu::Texture,
    white: wgpu::Texture,
    pipeline: wgpu::RenderPipeline,
    layout: wgpu::BindGroupLayout,
    sampler: wgpu::Sampler,
    viewports: HashMap<u64, Viewport>,
    uniforms: HashMap<u64, wgpu::Buffer>,
}
impl SpatialRenderer {
    pub fn new(handle: DeviceHandle, size: Resolution) -> Self {
        use wgpu::*;
        let device = &handle.device;
        let layout = device.create_bind_group_layout(&BindGroupLayoutDescriptor {
            label: Some("3D scene objects"),
            entries: &[
                BindGroupLayoutEntry {
                    binding: 0,
                    visibility: ShaderStages::VERTEX_FRAGMENT,
                    ty: BindingType::Buffer {
                        ty: BufferBindingType::Uniform,
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                },
                BindGroupLayoutEntry {
                    binding: 1,
                    visibility: ShaderStages::FRAGMENT,
                    ty: BindingType::Texture {
                        sample_type: TextureSampleType::Float { filterable: true },
                        view_dimension: TextureViewDimension::D2,
                        multisampled: false,
                    },
                    count: None,
                },
                BindGroupLayoutEntry {
                    binding: 2,
                    visibility: ShaderStages::FRAGMENT,
                    ty: BindingType::Sampler(SamplerBindingType::Filtering),
                    count: None,
                },
            ],
        });
        let shader = device.create_shader_module(ShaderModuleDescriptor {
            label: Some("Scene camera and viewport WGSL"),
            source: ShaderSource::Wgsl(include_str!("spatial.wgsl").into()),
        });
        let pipeline_layout = device.create_pipeline_layout(&PipelineLayoutDescriptor {
            label: None,
            bind_group_layouts: &[Some(&layout)],
            immediate_size: 0,
        });
        let pipeline = device.create_render_pipeline(&RenderPipelineDescriptor {
            label: Some("3D scene"),
            layout: Some(&pipeline_layout),
            vertex: VertexState {
                module: &shader,
                entry_point: Some("vertex"),
                compilation_options: Default::default(),
                buffers: &[],
            },
            fragment: Some(FragmentState {
                module: &shader,
                entry_point: Some("fragment"),
                compilation_options: Default::default(),
                targets: &[Some(ColorTargetState {
                    format: TextureFormat::Rgba8Unorm,
                    blend: Some(BlendState::PREMULTIPLIED_ALPHA_BLENDING),
                    write_mask: ColorWrites::ALL,
                })],
            }),
            primitive: PrimitiveState::default(),
            depth_stencil: Some(DepthStencilState {
                format: TextureFormat::Depth32Float,
                depth_write_enabled: Some(true),
                depth_compare: Some(CompareFunction::LessEqual),
                stencil: Default::default(),
                bias: Default::default(),
            }),
            multisample: Default::default(),
            multiview_mask: None,
            cache: None,
        });
        let output = crate::filters::texture(device, size.width, size.height);
        let depth = device.create_texture(&TextureDescriptor {
            label: Some("3D scene depth"),
            size: Extent3d {
                width: size.width,
                height: size.height,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: TextureDimension::D2,
            format: TextureFormat::Depth32Float,
            usage: TextureUsages::RENDER_ATTACHMENT,
            view_formats: &[],
        });
        let white = crate::filters::texture(device, 1, 1);
        handle.queue.write_texture(
            white.as_image_copy(),
            &[255, 255, 255, 255],
            TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(4),
                rows_per_image: Some(1),
            },
            Extent3d {
                width: 1,
                height: 1,
                depth_or_array_layers: 1,
            },
        );
        let sampler = device.create_sampler(&SamplerDescriptor {
            label: Some("Viewport sampler"),
            mag_filter: FilterMode::Linear,
            min_filter: FilterMode::Linear,
            ..Default::default()
        });
        Self {
            handle,
            size,
            output,
            depth,
            white,
            pipeline,
            layout,
            sampler,
            viewports: HashMap::new(),
            uniforms: HashMap::new(),
        }
    }
    pub fn size(&self) -> &Resolution {
        &self.size
    }
    pub fn render(&mut self, frame: &SceneFrame) -> Result<wgpu::Texture, String> {
        self.render_with_clear(frame, wgpu::Color::BLACK)
    }
    /// Transparent empty space lets editor guides sit behind solid scene objects.
    pub fn render_editor(&mut self, frame: &SceneFrame) -> Result<wgpu::Texture, String> {
        self.render_with_clear(frame, wgpu::Color::TRANSPARENT)
    }
    fn render_with_clear(
        &mut self,
        frame: &SceneFrame,
        clear: wgpu::Color,
    ) -> Result<wgpu::Texture, String> {
        let scene = frame.scene.spatial.as_ref().ok_or("3D scene is missing")?;
        if frame.size != self.size {
            return Err("3D output size changed".into());
        }
        scene.validate(
            &frame
                .scene
                .layers
                .iter()
                .map(|l| l.source_id)
                .chain(scene.objects.iter().flat_map(|o| match &o.kind {
                    ObjectKind::Viewport { layers, .. } => {
                        layers.iter().map(|l| l.source_id).collect::<Vec<_>>()
                    }
                    _ => vec![],
                }))
                .collect::<Vec<_>>(),
        )?;
        self.viewports.retain(|id, _| {
            scene
                .objects
                .iter()
                .any(|o| o.id == *id && matches!(o.kind, ObjectKind::Viewport { .. }))
        });
        self.uniforms
            .retain(|id, _| scene.objects.iter().any(|o| o.id == *id));
        let aspect = self.size.width as f32 / self.size.height as f32;
        let view_projection = camera_matrix(&scene.camera, aspect);
        let mut draws = vec![];
        for object in scene.objects.iter().filter(|o| o.visible) {
            let (texture, color, cube) = match &object.kind {
                ObjectKind::Viewport {
                    primary,
                    canvas,
                    layers,
                } => {
                    if self
                        .viewports
                        .get(&object.id)
                        .is_none_or(|v| v.size != *canvas)
                    {
                        self.viewports
                            .insert(object.id, Viewport::new(&self.handle, canvas.clone()));
                    }
                    let viewport = self.viewports.get_mut(&object.id).unwrap();
                    let mut contents = frame.clone();
                    contents.size = canvas.clone();
                    contents.scene.spatial = None;
                    if !primary {
                        contents.scene.layers = layers.clone();
                    }
                    viewport.render(&self.handle, &contents)?;
                    (viewport.texture.clone(), [1.; 4], false)
                }
                ObjectKind::Plane { color } => (self.white.clone(), *color, false),
                ObjectKind::Cube { color } => (self.white.clone(), *color, true),
            };
            let mvp = view_projection * transform_matrix(&object.transform);
            let mut floats = mvp.to_cols_array().to_vec();
            floats.extend_from_slice(&color);
            floats.extend_from_slice(&[f32::from(cube), 0., 0., 0.]);
            let bytes: Vec<_> = floats.iter().flat_map(|f| f.to_le_bytes()).collect();
            let uniform = self.uniforms.entry(object.id).or_insert_with(|| {
                self.handle
                    .device
                    .create_buffer_init(&wgpu::util::BufferInitDescriptor {
                        label: Some("3D object transform"),
                        contents: &bytes,
                        usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
                    })
            });
            self.handle.queue.write_buffer(uniform, 0, &bytes);
            let group = self
                .handle
                .device
                .create_bind_group(&wgpu::BindGroupDescriptor {
                    label: Some("3D object"),
                    layout: &self.layout,
                    entries: &[
                        wgpu::BindGroupEntry {
                            binding: 0,
                            resource: uniform.as_entire_binding(),
                        },
                        wgpu::BindGroupEntry {
                            binding: 1,
                            resource: wgpu::BindingResource::TextureView(
                                &texture.create_view(&Default::default()),
                            ),
                        },
                        wgpu::BindGroupEntry {
                            binding: 2,
                            resource: wgpu::BindingResource::Sampler(&self.sampler),
                        },
                    ],
                });
            draws.push((group, if cube { 36 } else { 6 }));
        }
        let mut encoder =
            self.handle
                .device
                .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                    label: Some("3D scene render"),
                });
        {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("Camera view"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &self.output.create_view(&Default::default()),
                    depth_slice: None,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(clear),
                        store: wgpu::StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                    view: &self.depth.create_view(&Default::default()),
                    depth_ops: Some(wgpu::Operations {
                        load: wgpu::LoadOp::Clear(1.),
                        store: wgpu::StoreOp::Store,
                    }),
                    stencil_ops: None,
                }),
                timestamp_writes: None,
                occlusion_query_set: None,
                multiview_mask: None,
            });
            pass.set_pipeline(&self.pipeline);
            for (group, count) in &draws {
                pass.set_bind_group(0, group, &[]);
                pass.draw(0..*count, 0..1);
            }
        }
        self.handle.queue.submit([encoder.finish()]);
        Ok(self.output.clone())
    }
}
/// Object-to-world transform shared with editor picking and gizmos.
pub fn transform_matrix(t: &Transform) -> Mat4 {
    Mat4::from_scale_rotation_translation(
        Vec3::from_array(t.scale),
        Quat::from_euler(
            EulerRot::XYZ,
            t.rotation[0].to_radians(),
            t.rotation[1].to_radians(),
            t.rotation[2].to_radians(),
        ),
        Vec3::from_array(t.position),
    )
}

/// Right-handed camera projection used by the renderer and editor overlays.
pub fn camera_matrix(camera: &Camera, aspect: f32) -> Mat4 {
    let projection = match camera.projection {
        Projection::Orthographic { height } => Mat4::orthographic_rh(
            -height * aspect / 2.,
            height * aspect / 2.,
            -height / 2.,
            height / 2.,
            camera.near,
            camera.far,
        ),
        Projection::Perspective { fov_degrees } => {
            Mat4::perspective_rh(fov_degrees.to_radians(), aspect, camera.near, camera.far)
        }
    };
    projection * transform_matrix(&camera.transform).inverse()
}
