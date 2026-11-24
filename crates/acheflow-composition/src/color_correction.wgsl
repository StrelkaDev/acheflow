fn effect(uv: vec2f, color: vec4f) -> vec4f {
    let primary = params.correction_primary;
    let black = vec3f(0.0);
    let white = vec3f(1.0);
    let midpoint = vec3f(0.5);
    let bright = clamp(color.rgb * primary.x, black, white);
    let contrasted = clamp((bright - midpoint) * primary.y + midpoint, black, white);
    let luma = vec3f(0.2126, 0.7152, 0.0722);
    let saturated = clamp(mix(vec3f(dot(contrasted, luma)), contrasted, primary.z), black, white);
    let grayscale = mix(saturated, vec3f(dot(saturated, luma)), primary.w);
    let corrected = pow(grayscale, vec3f(params.correction_secondary.x));
    return vec4f(corrected, color.a * params.correction_secondary.y);
}
