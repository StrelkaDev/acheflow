//! GPU texture widget following Blitz's custom-widget/AnyRender lifecycle.
use anyrender::{PaintRef, PaintScene, RenderContext, ResourceId, Scene};
use blitz_dom::{Widget, node::ComputedStyles};
use dioxus_native::prelude::{Signal, WritableExt};
use peniko::{
    Fill, ImageBrush, ImageSampler,
    kurbo::{Affine, Rect},
};
use web_time::Instant;
use wgpu_context::DeviceHandle;

pub struct SparksWidget {
    gpu: Option<GpuSparks>,
    started: Instant,
    backend: Signal<String>,
}

impl SparksWidget {
    pub fn new(backend: Signal<String>) -> Self {
        Self {
            gpu: None,
            started: Instant::now(),
            backend,
        }
    }
}

impl Widget for SparksWidget {
    fn can_create_surfaces(&mut self, ctx: &mut dyn RenderContext) {
        if let Some(handle) = ctx
            .renderer_specific_context()
            .and_then(|context| context.downcast::<DeviceHandle>().ok())
        {
            let info = handle.adapter.get_info();
            let label = match info.backend {
                wgpu::Backend::BrowserWebGpu => "WebGPU",
                wgpu::Backend::Metal => "Metal",
                wgpu::Backend::Vulkan => "Vulkan",
                wgpu::Backend::Dx12 => "DirectX 12",
                wgpu::Backend::Gl => "OpenGL / WebGL",
                _ => "Other",
            };
            self.backend.set(format!("{label} / Blitz + wgpu"));
            self.gpu = Some(GpuSparks::new(&handle));
        } else {
            self.backend
                .set("GPU texture backend unavailable".to_owned());
        }
    }

    fn destroy_surfaces(&mut self) {
        self.gpu = None;
    }
    fn requires_redraw(&self) -> bool {
        self.gpu.is_some()
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
        if width == 0 || height == 0 {
            return scene;
        }
        // Dioxus can attach this widget after the surface-created notification.
        // Initialize on its first paint as well, including after re-enabling effects.
        if self.gpu.is_none() {
            self.can_create_surfaces(ctx);
        }
        if let Some(gpu) = &mut self.gpu {
            let id = gpu.render(
                ctx,
                width,
                height,
                self.started.elapsed().as_secs_f32() % 3600.0,
            );
            scene.fill(
                Fill::NonZero,
                Affine::IDENTITY,
                PaintRef::Resource(ImageBrush {
                    image: id,
                    sampler: ImageSampler::default(),
                }),
                None,
                &Rect::from_origin_size((0.0, 0.0), (width as f64, height as f64)),
            );
        }
        scene
    }
}

struct Target {
    texture: wgpu::Texture,
    resource: ResourceId,
}

struct GpuSparks {
    device: wgpu::Device,
    queue: wgpu::Queue,
    pipeline: wgpu::RenderPipeline,
    uniform: wgpu::Buffer,
    bindings: wgpu::BindGroup,
    targets: Vec<Target>,
    next: usize,
}

impl GpuSparks {
    fn new(handle: &DeviceHandle) -> Self {
        let device = &handle.device;
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("Acheflow sparks"),
            source: wgpu::ShaderSource::Wgsl(include_str!("sparks.wgsl").into()),
        });
        let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("Sparks uniform layout"),
            entries: &[wgpu::BindGroupLayoutEntry {
                binding: 0,
                visibility: wgpu::ShaderStages::VERTEX,
                ty: wgpu::BindingType::Buffer {
                    ty: wgpu::BufferBindingType::Uniform,
                    has_dynamic_offset: false,
                    min_binding_size: std::num::NonZeroU64::new(16),
                },
                count: None,
            }],
        });
        let uniform = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("Sparks time and dimensions"),
            size: 16,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let bindings = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: None,
            layout: &layout,
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: uniform.as_entire_binding(),
            }],
        });
        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: None,
            bind_group_layouts: &[Some(&layout)],
            immediate_size: 0,
        });
        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("Instanced spark quads"),
            layout: Some(&pipeline_layout),
            vertex: wgpu::VertexState {
                module: &shader,
                entry_point: Some("vs_main"),
                buffers: &[],
                compilation_options: Default::default(),
            },
            fragment: Some(wgpu::FragmentState {
                module: &shader,
                entry_point: Some("fs_main"),
                targets: &[Some(wgpu::ColorTargetState {
                    format: wgpu::TextureFormat::Rgba8Unorm,
                    blend: Some(wgpu::BlendState {
                        color: wgpu::BlendComponent {
                            src_factor: wgpu::BlendFactor::SrcAlpha,
                            dst_factor: wgpu::BlendFactor::One,
                            operation: wgpu::BlendOperation::Add,
                        },
                        alpha: wgpu::BlendComponent::OVER,
                    }),
                    write_mask: wgpu::ColorWrites::ALL,
                })],
                compilation_options: Default::default(),
            }),
            primitive: Default::default(),
            depth_stencil: None,
            multisample: Default::default(),
            multiview_mask: None,
            cache: None,
        });
        Self {
            device: device.clone(),
            queue: handle.queue.clone(),
            pipeline,
            uniform,
            bindings,
            targets: vec![],
            next: 0,
        }
    }

    fn render(
        &mut self,
        ctx: &mut dyn RenderContext,
        width: u32,
        height: u32,
        time: f32,
    ) -> ResourceId {
        // Both buffers must match the new viewport; old registrations are released on resize.
        if self.targets.first().is_some_and(|target| {
            target.texture.width() != width || target.texture.height() != height
        }) {
            for target in self.targets.drain(..) {
                ctx.unregister_resource(target.resource);
            }
        }
        if self.targets.is_empty() {
            for _ in 0..2 {
                let texture = self.device.create_texture(&wgpu::TextureDescriptor {
                    label: Some("Sparks offscreen target"),
                    size: wgpu::Extent3d {
                        width,
                        height,
                        depth_or_array_layers: 1,
                    },
                    mip_level_count: 1,
                    sample_count: 1,
                    dimension: wgpu::TextureDimension::D2,
                    format: wgpu::TextureFormat::Rgba8Unorm,
                    usage: wgpu::TextureUsages::RENDER_ATTACHMENT
                        | wgpu::TextureUsages::TEXTURE_BINDING
                        | wgpu::TextureUsages::COPY_SRC,
                    view_formats: &[],
                });
                let resource = ctx
                    .try_register_custom_resource(Box::new(texture.clone()))
                    .expect("The selected Blitz renderer must support wgpu textures");
                self.targets.push(Target { texture, resource });
            }
        }
        let bytes: Vec<u8> = [time, width as f32, height as f32, 1.0_f32]
            .into_iter()
            .flat_map(f32::to_le_bytes)
            .collect();
        self.queue.write_buffer(&self.uniform, 0, &bytes);
        let target = &self.targets[self.next];
        let view = target.texture.create_view(&Default::default());
        let mut encoder = self.device.create_command_encoder(&Default::default());
        {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("Sparks background pass"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &view,
                    resolve_target: None,
                    depth_slice: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color {
                            r: 0.047,
                            g: 0.063,
                            b: 0.09,
                            a: 1.0,
                        }),
                        store: wgpu::StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: None,
                timestamp_writes: None,
                multiview_mask: None,
                occlusion_query_set: None,
            });
            pass.set_pipeline(&self.pipeline);
            pass.set_bind_group(0, &self.bindings, &[]);
            pass.draw(0..6, 0..180);
        }
        self.queue.submit(Some(encoder.finish()));
        let resource = target.resource;
        self.next = (self.next + 1) % 2;
        resource
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn spark_shader_validates_without_optional_gpu_capabilities() {
        let module = naga::front::wgsl::parse_str(include_str!("sparks.wgsl"))
            .expect("WGSL must parse before the application launches");
        naga::valid::Validator::new(
            naga::valid::ValidationFlags::all(),
            naga::valid::Capabilities::empty(),
        )
        .validate(&module)
        .expect("Sparks must not require optional GPU features");
    }
}
