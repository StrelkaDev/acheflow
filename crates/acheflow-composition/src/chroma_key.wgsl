fn chroma(rgb: vec3<f32>) -> vec2<f32> {
    let luma = dot(rgb, vec3<f32>(0.2126, 0.7152, 0.0722));
    return vec2<f32>(0.5389 * (rgb.b - luma), 0.6350 * (rgb.r - luma));
}

fn matte(rgb: vec3<f32>, key_chroma: vec2<f32>) -> f32 {
    return smoothstep(params.key_settings.x, params.key_settings.x + params.key_settings.y, distance(chroma(rgb), key_chroma));
}

fn effect(uv: vec2<f32>, color: vec4<f32>) -> vec4<f32> {
    let key_chroma = chroma(params.key_color.rgb);
    let px = params.key_settings.z / params.size;
    let dx = vec2<f32>(px.x, 0.0);
    let dy = vec2<f32>(0.0, px.y);
    let center = matte(color.rgb, key_chroma);
    let adjacent = 0.25 * (
        matte(sample_source(uv - dx).rgb, key_chroma) +
        matte(sample_source(uv + dx).rgb, key_chroma) +
        matte(sample_source(uv - dy).rgb, key_chroma) +
        matte(sample_source(uv + dy).rgb, key_chroma)
    );
    let kept = min(center, mix(center, adjacent, 0.5));

    let key_axis = params.key_color.rgb - vec3<f32>((params.key_color.rgb.r + params.key_color.rgb.g + params.key_color.rgb.b) / 3.0);
    let axis = key_axis / max(length(key_axis), 0.0001);
    let spill = max(0.0, dot(color.rgb, axis));
    let color_match = 1.0 - smoothstep(params.key_settings.x, params.key_settings.x + params.key_settings.y + 0.20,
        distance(chroma(color.rgb), key_chroma));
    let near_edge = max(color_match, 1.0 - min(center, adjacent));
    let clean = clamp(color.rgb - axis * spill * near_edge * params.key_settings.w,
        vec3<f32>(0.0), vec3<f32>(1.0));

    let amount = clamp(params.strength, 0.0, 1.0);
    return vec4<f32>(mix(color.rgb, clean, amount), color.a * mix(1.0, kept, amount));
}
