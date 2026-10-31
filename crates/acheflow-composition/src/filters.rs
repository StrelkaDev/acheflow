//! Bounded, reusable GPU passes. Input/output textures contain straight RGBA.
use acheflow_media_api::{
    appearance::{shader_module, validate_shader},
    workspace::{EffectKind, LayerEffect},
};
use wgpu::*;

pub struct PassParams {
    pub crop: [f32; 4],
    pub time: f32,
    pub source_size: [f32; 2],
}
pub struct Filters {
    layout: BindGroupLayout,
    sampler: Sampler,
    crop: RenderPipeline,
    premultiply: RenderPipeline,
    passes: Vec<(LayerEffect, RenderPipeline)>,
}
pub fn texture(device: &Device, width: u32, height: u32) -> Texture {
    device.create_texture(&TextureDescriptor {
        label: Some("Acheflow layer texture"),
        size: Extent3d {
            width,
            height,
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: TextureDimension::D2,
        format: TextureFormat::Rgba8Unorm,
        usage: TextureUsages::COPY_DST
            | TextureUsages::TEXTURE_BINDING
            | TextureUsages::RENDER_ATTACHMENT
            | TextureUsages::COPY_SRC,
        view_formats: &[],
    })
}
impl Filters {
    pub fn new(device: &Device) -> Self {
        let layout = device.create_bind_group_layout(&BindGroupLayoutDescriptor {
            label: Some("Layer input"),
            entries: &[
                BindGroupLayoutEntry {
                    binding: 0,
                    visibility: ShaderStages::FRAGMENT,
                    ty: BindingType::Texture {
                        sample_type: TextureSampleType::Float { filterable: true },
                        view_dimension: TextureViewDimension::D2,
                        multisampled: false,
                    },
                    count: None,
                },
                BindGroupLayoutEntry {
                    binding: 1,
                    visibility: ShaderStages::FRAGMENT,
                    ty: BindingType::Sampler(SamplerBindingType::Filtering),
                    count: None,
                },
                BindGroupLayoutEntry {
                    binding: 2,
                    visibility: ShaderStages::FRAGMENT,
                    ty: BindingType::Buffer {
                        ty: BufferBindingType::Uniform,
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                },
            ],
        });
        let sampler = device.create_sampler(&SamplerDescriptor {
            label: Some("Layer linear sampler"),
            mag_filter: FilterMode::Linear,
            min_filter: FilterMode::Linear,
            ..Default::default()
        });
        let crop = pipeline(device, &layout, &shader_module(CROP));
        let premultiply = pipeline(
            device,
            &layout,
            &shader_module(
                "fn effect(uv:vec2f,color:vec4f)->vec4f {return vec4f(color.rgb*color.a,color.a);}",
            ),
        );
        Self {
            layout,
            sampler,
            crop,
            premultiply,
            passes: vec![],
        }
    }
    pub fn configure(&mut self, device: &Device, effects: &[LayerEffect]) -> Result<(), String> {
        if self
            .passes
            .iter()
            .map(|(e, _)| e)
            .eq(effects.iter().filter(|e| e.enabled))
        {
            return Ok(());
        }
        let mut passes = vec![];
        for effect in effects.iter().filter(|e| e.enabled) {
            let code = match effect.kind {
                EffectKind::Wgsl => {
                    validate_shader(&effect.shader)?;
                    effect.shader.clone()
                }
                _ => builtin(effect.kind),
            };
            passes.push((
                effect.clone(),
                pipeline(device, &self.layout, &shader_module(&code)),
            ));
        }
        self.passes = passes;
        Ok(())
    }
    /// crop = normalized left, top, right edge and bottom edge in source space.
    pub fn run(
        &self,
        device: &Device,
        queue: &Queue,
        input: &Texture,
        targets: &[Texture; 2],
        params: PassParams,
    ) -> usize {
        let PassParams {
            crop,
            time,
            source_size,
        } = params;
        let mut encoder = device.create_command_encoder(&CommandEncoderDescriptor {
            label: Some("Layer GPU effects"),
        });
        // Crop pass stores four normalized edges in the common parameter slots.
        self.draw(
            device,
            &mut encoder,
            input,
            &targets[0],
            &self.crop,
            [crop[0], crop[1], crop[2], crop[3], 1., 1.],
        );
        let mut current = 0;
        for (effect, pipeline) in &self.passes {
            let next = 1 - current;
            self.draw(
                device,
                &mut encoder,
                &targets[current],
                &targets[next],
                pipeline,
                [
                    time,
                    effect.amount as f32 / 100.,
                    source_size[0],
                    source_size[1],
                    input.width() as f32 / source_size[0],
                    input.height() as f32 / source_size[1],
                ],
            );
            current = next;
        }
        let next = 1 - current;
        self.draw(
            device,
            &mut encoder,
            &targets[current],
            &targets[next],
            &self.premultiply,
            [time, 1., source_size[0], source_size[1], 1., 1.],
        );
        queue.submit([encoder.finish()]);
        next
    }
    fn draw(
        &self,
        device: &Device,
        encoder: &mut CommandEncoder,
        input: &Texture,
        target: &Texture,
        pipeline: &RenderPipeline,
        params: [f32; 6],
    ) {
        let floats = [
            target.width() as f32,
            target.height() as f32,
            params[0],
            params[1],
            params[2],
            params[3],
            params[4],
            params[5],
        ];
        let bytes: Vec<u8> = floats.into_iter().flat_map(f32::to_le_bytes).collect();
        use wgpu::util::DeviceExt;
        let uniform = device.create_buffer_init(&util::BufferInitDescriptor {
            label: Some("Layer parameters"),
            contents: &bytes,
            usage: BufferUsages::UNIFORM,
        });
        let input = input.create_view(&Default::default());
        let output = target.create_view(&Default::default());
        let group = device.create_bind_group(&BindGroupDescriptor {
            label: Some("Layer pass bindings"),
            layout: &self.layout,
            entries: &[
                BindGroupEntry {
                    binding: 0,
                    resource: BindingResource::TextureView(&input),
                },
                BindGroupEntry {
                    binding: 1,
                    resource: BindingResource::Sampler(&self.sampler),
                },
                BindGroupEntry {
                    binding: 2,
                    resource: uniform.as_entire_binding(),
                },
            ],
        });
        let mut pass = encoder.begin_render_pass(&RenderPassDescriptor {
            label: Some("Layer effect"),
            color_attachments: &[Some(RenderPassColorAttachment {
                view: &output,
                depth_slice: None,
                resolve_target: None,
                ops: Operations {
                    load: LoadOp::Clear(Color::TRANSPARENT),
                    store: StoreOp::Store,
                },
            })],
            ..Default::default()
        });
        pass.set_pipeline(pipeline);
        pass.set_bind_group(0, &group, &[]);
        pass.draw(0..3, 0..1);
    }
}
fn pipeline(device: &Device, layout: &BindGroupLayout, source: &str) -> RenderPipeline {
    let module = device.create_shader_module(ShaderModuleDescriptor {
        label: Some("Layer WGSL"),
        source: ShaderSource::Wgsl(source.into()),
    });
    let layout = device.create_pipeline_layout(&PipelineLayoutDescriptor {
        label: None,
        bind_group_layouts: &[Some(layout)],
        immediate_size: 0,
    });
    device.create_render_pipeline(&RenderPipelineDescriptor {
        label: Some("Layer fragment pass"),
        layout: Some(&layout),
        vertex: VertexState {
            module: &module,
            entry_point: Some("vertex"),
            compilation_options: Default::default(),
            buffers: &[],
        },
        fragment: Some(FragmentState {
            module: &module,
            entry_point: Some("fragment"),
            compilation_options: Default::default(),
            targets: &[Some(ColorTargetState {
                format: TextureFormat::Rgba8Unorm,
                blend: None,
                write_mask: ColorWrites::ALL,
            })],
        }),
        primitive: PrimitiveState::default(),
        depth_stencil: None,
        multisample: MultisampleState::default(),
        multiview_mask: None,
        cache: None,
    })
}
// For the crop pass: time/strength/source_size carry normalized crop edges.
const CROP: &str = "fn effect(uv:vec2f,color:vec4f)->vec4f {return sample_source(mix(vec2f(params.time,params.strength),params.source_size,uv));}";
fn builtin(kind: EffectKind) -> String {
    let expression = match kind {
        EffectKind::Brightness => "vec4f(color.rgb * params.strength,color.a)",
        EffectKind::Contrast => {
            "vec4f((color.rgb - vec3f(0.5)) * params.strength + vec3f(0.5),color.a)"
        }
        EffectKind::Saturation => {
            "vec4f(mix(vec3f(dot(color.rgb,vec3f(0.2126,0.7152,0.0722))),color.rgb,params.strength),color.a)"
        }
        EffectKind::Grayscale => {
            "vec4f(mix(color.rgb,vec3f(dot(color.rgb,vec3f(0.2126,0.7152,0.0722))),params.strength),color.a)"
        }
        EffectKind::Opacity => "vec4f(color.rgb,color.a * params.strength)",
        EffectKind::Blur => return BLUR.into(),
        EffectKind::Wgsl => unreachable!(),
    };
    format!(
        "fn effect(uv:vec2f,color:vec4f)->vec4f {{return clamp({expression},vec4f(0.),vec4f(1.));}}"
    )
}
const BLUR: &str = "fn effect(uv:vec2f,color:vec4f)->vec4f {\n let radius=params.strength * 100.0 * params.padding.x;\n if radius<0.01 {return color;}\n var sum=vec4f(0.); var weight=0.;\n for(var y:i32=-2;y<=2;y++){for(var x:i32=-2;x<=2;x++){let d=vec2f(f32(x),f32(y));let w=exp(-dot(d,d)*0.5);sum+=sample_source(uv+d*radius/params.size*0.5)*w;weight+=w;}}\n return sum/weight; }";
