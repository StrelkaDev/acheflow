struct Object { mvp:mat4x4f, color:vec4f, params:vec4f }
@group(0) @binding(0) var<uniform> object:Object;
@group(0) @binding(1) var image:texture_2d<f32>;
@group(0) @binding(2) var image_sampler:sampler;
struct Vertex { @builtin(position) position:vec4f, @location(0) uv:vec2f }
@vertex fn vertex(@builtin(vertex_index) index:u32)->Vertex {
  let corners=array<vec2f,6>(vec2f(-0.5,-0.5),vec2f(0.5,-0.5),vec2f(-0.5,0.5),vec2f(-0.5,0.5),vec2f(0.5,-0.5),vec2f(0.5,0.5));
  let xy=corners[index%6u];
  var position=vec3f(xy,0.);
  if object.params.x>0.5 {
    let face=index/6u;
    if face==0u {position=vec3f(xy,0.5);}
    if face==1u {position=vec3f(-xy.x,xy.y,-0.5);}
    if face==2u {position=vec3f(0.5,xy.y,-xy.x);}
    if face==3u {position=vec3f(-0.5,xy.y,xy.x);}
    if face==4u {position=vec3f(xy.x,0.5,-xy.y);}
    if face==5u {position=vec3f(xy.x,-0.5,xy.y);}
  }
  var result:Vertex;
  result.position=object.mvp*vec4f(position,1.);
  result.uv=vec2f(xy.x+0.5,0.5-xy.y);
  return result;
}
@fragment fn fragment(in:Vertex)->@location(0) vec4f {
  let sampled=textureSample(image,image_sampler,in.uv);
  let color=sampled*vec4f(object.color.rgb*object.color.a,object.color.a);
  if color.a<0.001 {discard;}
  return color;
}
