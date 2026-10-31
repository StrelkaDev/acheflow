//! Serializable appearance contracts are validated before reaching a GPU.
use cssparser::{Parser, Token};

pub const DEFAULT_SHADER: &str = "fn effect(uv: vec2<f32>, color: vec4<f32>) -> vec4<f32> {\n    let wave = 0.5 + 0.5 * sin(params.time * 2.0 + uv.y * 12.0);\n    let tint = mix(vec3<f32>(1.0, 0.45, 0.15), vec3<f32>(0.2, 0.65, 1.0), wave);\n    return vec4<f32>(mix(color.rgb, color.rgb * tint, params.strength), color.a);\n}";
pub const SHADER_HEADER: &str = "struct Params { size: vec2<f32>, time: f32, strength: f32, source_size: vec2<f32>, padding: vec2<f32> };\n@group(0) @binding(0) var input_texture: texture_2d<f32>;\n@group(0) @binding(1) var input_sampler: sampler;\n@group(0) @binding(2) var<uniform> params: Params;\nfn sample_source(uv: vec2<f32>) -> vec4<f32> { return textureSampleLevel(input_texture, input_sampler, uv, 0.0); }\n";
pub const SHADER_ENTRY: &str = "\n@vertex fn vertex(@builtin(vertex_index) index: u32) -> @builtin(position) vec4<f32> {\n let x = f32((index << 1u) & 2u); let y = f32(index & 2u);\n return vec4<f32>(x * 2.0 - 1.0, 1.0 - y * 2.0, 0.0, 1.0);\n}\n@fragment fn fragment(@builtin(position) position: vec4<f32>) -> @location(0) vec4<f32> {\n let uv = position.xy / params.size; return effect(uv, sample_source(uv));\n}\n";
pub fn shader_module(code: &str) -> String {
    format!("{SHADER_HEADER}\n{code}\n{SHADER_ENTRY}")
}

pub fn validate_shader(code: &str) -> Result<(), String> {
    if code.len() > 32768 {
        return Err("WGSL must be at most 32 KiB".into());
    }
    let module = naga::front::wgsl::parse_str(&shader_module(code))
        .map_err(|e| e.emit_to_string(&shader_module(code)))?;
    naga::valid::Validator::new(
        naga::valid::ValidationFlags::all(),
        naga::valid::Capabilities::empty(),
    )
    .validate(&module)
    .map_err(|e| format!("WGSL: {e}"))?;
    // Resource bindings and entry points belong to the host. Effects are bounded
    // fragment functions, with no loops or writes to buffers/textures.
    if module.entry_points.len() != 2 || module.global_variables.len() != 3 {
        return Err("WGSL effects cannot declare resources or entry points".into());
    }
    let Some((_, effect)) = module
        .functions
        .iter()
        .find(|(_, f)| f.name.as_deref() == Some("effect"))
    else {
        return Err("WGSL needs fn effect(uv: vec2<f32>, color: vec4<f32>) -> vec4<f32>".into());
    };
    if effect.arguments.len() != 2 {
        return Err("WGSL effect needs exactly uv and color arguments".into());
    }
    fn bounded(block: &naga::Block) -> bool {
        block.iter().all(|s| match s {
            naga::Statement::Loop { .. }
            | naga::Statement::ImageStore { .. }
            | naga::Statement::Atomic { .. } => false,
            naga::Statement::Block(b) => bounded(b),
            naga::Statement::If { accept, reject, .. } => bounded(accept) && bounded(reject),
            naga::Statement::Switch { cases, .. } => cases.iter().all(|c| bounded(&c.body)),
            _ => true,
        })
    }
    if module
        .functions
        .iter()
        .any(|(_, f)| f.expressions.len() > 4096 || !bounded(&f.body))
    {
        return Err(
            "WGSL effects do not support loops, atomics, storage writes or oversized functions"
                .into(),
        );
    }
    Ok(())
}

/// External resources are disabled. Each stylesheet lives in an isolated
/// document containing only this layer; it cannot style the control panel.
pub fn validate_css(css: &str) -> Result<(), String> {
    if css.len() > 16384 || css.contains(['<', '\0']) {
        return Err("Layer CSS must be at most 16 KiB and contain no HTML".into());
    }
    fn scan<'i>(p: &mut Parser<'i>, depth: u32) -> Result<(), cssparser::ParseError<String>> {
        if depth > 16 {
            return Err(cssparser::ParseError::custom::<String>(
                "CSS nesting is too deep".into(),
            ));
        }
        while !p.is_exhausted() {
            let token = p.next_including_whitespace_and_comments()?.clone();
            match token {
                Token::UnquotedUrl(_) | Token::BadUrl(_) | Token::BadString(_) => {
                    return Err(cssparser::ParseError::custom::<String>(
                        "CSS URLs and malformed strings are not supported".into(),
                    ));
                }
                Token::AtKeyword(ref name)
                    if ["import", "font-face", "namespace"]
                        .iter()
                        .any(|n| name.eq_ignore_ascii_case(n)) =>
                {
                    return Err(cssparser::ParseError::custom::<String>(
                        "External CSS resources are not supported".into(),
                    ));
                }
                Token::Function(ref name) if name.eq_ignore_ascii_case("url") => {
                    return Err(cssparser::ParseError::custom::<String>(
                        "CSS URLs are not supported".into(),
                    ));
                }
                Token::Function(_)
                | Token::ParenthesisBlock
                | Token::SquareBracketBlock
                | Token::CurlyBracketBlock => p.parse_nested_block(|p| scan(p, depth + 1))?,
                _ => {}
            }
        }
        Ok(())
    }
    scan(&mut Parser::new(css), 0).map_err(|e| format!("CSS: {e:?}"))
}
/// Bare declarations are convenient; a full stylesheet may use `.layer`
/// (or `:layer`) and local @keyframes for more elaborate styling.
pub fn layer_stylesheet(css: &str) -> String {
    if css.contains('{') {
        css.replace(":layer", ".layer")
    } else {
        format!(".layer {{ {css} }}")
    }
}

pub fn default_layer_css(layer: &crate::workspace::Layer) -> String {
    format!(
        ".layer {{\n  left: {}px;\n  top: {}px;\n  width: {}px;\n  height: {}px;\n  --crop-left: {}px;\n  --crop-top: {}px;\n  --crop-right: {}px;\n  --crop-bottom: {}px;\n  object-fit: fill;\n  object-position: 50% 50%;\n}}",
        layer.x,
        layer.y,
        layer.width,
        layer.height,
        layer.crop.left,
        layer.crop.top,
        layer.crop.right,
        layer.crop.bottom
    )
}
/// Edits the last plain `.layer` rule in place, retaining crop, decoration,
/// keyframes and comments. Dragging does not accumulate override rules.
pub fn geometry_css(css: &str, x: u32, y: u32, width: u32, height: u32) -> String {
    let mut css = css.to_owned();
    let old_start = "/* editor geometry */";
    let old_end = "/* end editor geometry */";
    if let Some(a) = css.find(old_start)
        && let Some(b) = css[a..].find(old_end)
    {
        css.replace_range(a..a + b + old_end.len(), "");
    }
    let mut css = layer_stylesheet(&css);
    let mut parser = Parser::new(&css);
    let mut selector_start = 0;
    let mut target = None;
    while let Ok(token) = parser.next_including_whitespace_and_comments().cloned() {
        let end = parser.position().byte_index();
        if matches!(token, Token::CurlyBracketBlock) {
            let selector = &css[selector_start..end - 1];
            let mut selector = Parser::new(selector);
            let layer = selector.expect_delim('.').is_ok()
                && selector.expect_ident_matching("layer").is_ok()
                && selector.expect_exhausted().is_ok();
            let range = parser.parse_nested_block(|p| {
                let a = p.position().byte_index();
                while p.next_including_whitespace_and_comments().is_ok() {}
                Ok::<_, cssparser::ParseError<()>>((a, p.position().byte_index()))
            });
            if layer && let Ok(range) = range {
                target = Some(range);
            }
            selector_start = parser.position().byte_index();
        }
    }
    drop(parser);
    let geometry = format!(
        "\n  left: {x}px; top: {y}px;\n  width: {width}px; height: {height}px;\n  right: auto; bottom: auto;\n"
    );
    if let Some((a, b)) = target {
        let body = &css[a..b];
        let mut p = Parser::new(body);
        let mut start = 0;
        let mut declarations = vec![];
        while let Ok(token) = p.next_including_whitespace_and_comments().cloned() {
            if matches!(token, Token::Semicolon) {
                let end = p.position().byte_index();
                declarations.push(&body[start..end]);
                start = end;
            }
        }
        declarations.push(&body[start..]);
        let retained = declarations
            .into_iter()
            .filter(|declaration| {
                let mut p = Parser::new(declaration);
                let name = p.expect_ident_cloned().ok();
                !name.is_some_and(|name| {
                    ["left", "top", "right", "bottom", "width", "height", "inset"]
                        .iter()
                        .any(|n| name.eq_ignore_ascii_case(n))
                })
            })
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .collect::<Vec<_>>()
            .join("\n  ");
        drop(p);
        css.replace_range(a..b, &format!("{geometry}  {retained}\n"));
    } else {
        css.push_str(&format!("\n.layer {{{geometry}}}"));
    }
    css
}

pub fn managed_css(css: &str, name: &str, rule: &str) -> String {
    let start = format!("/* editor {name} */");
    let end = format!("/* end editor {name} */");
    let mut css = css.to_owned();
    if let Some(a) = css.find(&start)
        && let Some(b) = css[a..].find(&end)
    {
        css.replace_range(a..a + b + end.len(), "");
    }
    format!("{}\n{start}\n{rule}\n{end}", layer_stylesheet(css.trim()))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn shader_contract_rejects_invalid_or_unbounded_code() {
        assert!(validate_shader(DEFAULT_SHADER).is_ok());
        assert!(validate_shader("fn effect(uv:vec2f,c:vec4f)->vec4f {return c.bgra;}").is_ok());
        assert!(validate_shader("fn effect(uv:vec2f,c:vec4f)->vec4f {loop {} return c;}").is_err());
        assert!(validate_shader("fn effect()->vec4f {return vec4f(0.);}").is_err());
        assert!(validate_shader("broken").is_err());
    }
    #[test]
    fn css_is_local_but_supports_keyframes() {
        assert!(validate_css("border:3px solid #f90;opacity:.5;").is_ok());
        assert!(validate_css(".layer {animation:pulse 1s infinite;} @keyframes pulse {from {opacity:0;} to {opacity:1;}}").is_ok());
        for css in [
            "@import 'https://example.com/style.css';",
            "background:u\\72l(https://example.com)",
            "</style><script>",
        ] {
            assert!(validate_css(css).is_err(), "{css}");
        }
    }
    #[test]
    fn dragging_edits_geometry_without_hiding_user_properties() {
        let mut css=".layer {left:0px;top:0px;width:100px;height:100px;--crop-left:10%;border:3px solid red;object-fit:cover;} @keyframes pulse {from {opacity:0;}to {opacity:1;}}".to_owned();
        for x in 0..100 {
            css = geometry_css(&css, x, 20, 300, 200);
        }
        assert_eq!(css.matches("left:").count(), 2); // left and --crop-left
        assert!(css.contains("left: 99px"));
        assert!(!css.contains("left:0px"));
        assert!(css.contains("--crop-left:10%"));
        assert!(css.contains("@keyframes pulse"));
        assert!(css.contains("object-fit:cover"));
        assert!(css.len() < 400);
        assert!(
            css.lines().count() < 20,
            "dragging must not accumulate blank lines"
        );
        assert!(validate_css(&css).is_ok());
    }
}
