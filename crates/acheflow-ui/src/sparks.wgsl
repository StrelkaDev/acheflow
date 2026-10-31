struct Params { time: f32, width: f32, height: f32, intensity: f32 }
@group(0) @binding(0) var<uniform> params: Params;

struct Spark {
    @builtin(position) position: vec4<f32>,
    @location(0) uv: vec2<f32>,
    @location(1) life: f32,
    @location(2) heat: f32,
}

fn random(seed: f32) -> f32 { return fract(sin(seed * 127.1 + 311.7) * 43758.5453); }

@vertex
fn vs_main(@builtin(vertex_index) vertex: u32, @builtin(instance_index) instance: u32) -> Spark {
    let corners = array<vec2<f32>, 6>(vec2(-1.,-1.),vec2(1.,-1.),vec2(-1.,1.),vec2(-1.,1.),vec2(1.,-1.),vec2(1.,1.));
    let id = f32(instance) + 1.;
    let speed = 0.035 + random(id + 3.) * 0.07;
    let age = fract(params.time * speed + random(id + 9.));
    let x = random(id) * params.width + sin(age * 5. + id) * 32. + age * 50.;
    let y = params.height * (1.08 - age * 1.2);
    let size = 1. + random(id + 11.) * 1.7;
    let corner = corners[vertex];
    let pixel = vec2(x,y) + corner * vec2(size * 3., size * (4. + random(id + 15.) * 7.));
    var result: Spark;
    result.position = vec4(pixel.x / params.width * 2. - 1., 1. - pixel.y / params.height * 2., 0., 1.);
    result.uv = corner;
    result.life = sin(age * 3.14159) * params.intensity;
    result.heat = random(id + 20.);
    return result;
}

@fragment
fn fs_main(spark: Spark) -> @location(0) vec4<f32> {
    let core = exp(-dot(spark.uv * vec2(3., 1.8), spark.uv * vec2(3., 1.8)));
    let halo = exp(-dot(spark.uv, spark.uv) * 3.) * 0.16;
    let color = mix(vec3(1., 0.22, 0.035), vec3(1., 0.68, 0.26), spark.heat);
    return vec4(color, (core + halo) * spark.life * 0.72);
}
