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
        fn scan<'i>(
            parser: &mut Parser<'i>,
            aliases: &mut Vec<std::ops::Range<usize>>,
        ) -> Result<(), cssparser::ParseError<()>> {
            let mut group = false;
            while let Ok(token) = parser.next_including_whitespace_and_comments().cloned() {
                match token {
                    Token::AtKeyword(name) => {
                        group = [
                            "media",
                            "supports",
                            "container",
                            "layer",
                            "scope",
                            "starting-style",
                        ]
                        .iter()
                        .any(|n| name.eq_ignore_ascii_case(n));
                    }
                    Token::Colon => {
                        let start = parser.position().byte_index() - 1;
                        if parser
                            .try_parse(|p| p.expect_ident_matching("layer"))
                            .is_ok()
                        {
                            aliases.push(start..parser.position().byte_index());
                        }
                    }
                    Token::CurlyBracketBlock if group => {
                        parser.parse_nested_block(|p| scan(p, aliases))?;
                        group = false;
                    }
                    Token::CurlyBracketBlock | Token::Semicolon => group = false,
                    _ => {}
                }
            }
            Ok(())
        }
        let mut aliases = vec![];
        // Skip declaration blocks, including animation:layer_pulse and strings.
        let _ = scan(&mut Parser::new(css), &mut aliases);
        let mut css = css.to_owned();
        for range in aliases.into_iter().rev() {
            css.replace_range(range, ".layer");
        }
        css
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
struct LayerRule {
    rule: std::ops::Range<usize>,
    body: std::ops::Range<usize>,
}

/// Only match a top-level plain selector, never a keyframe or nested rule.
fn last_layer_rule(css: &str) -> Option<LayerRule> {
    let mut parser = Parser::new(css);
    let mut selector_start = None;
    let mut target = None;
    loop {
        let start = parser.position().byte_index();
        let Ok(token) = parser.next_including_whitespace_and_comments().cloned() else {
            break;
        };
        if matches!(token, Token::WhiteSpace(_) | Token::Comment(_)) {
            continue;
        }
        let rule_start = *selector_start.get_or_insert(start);
        let end = parser.position().byte_index();
        if matches!(token, Token::CurlyBracketBlock) {
            let selector = &css[rule_start..end - 1];
            let mut selector = Parser::new(selector);
            let layer = selector.expect_delim('.').is_ok()
                && selector.expect_ident_matching("layer").is_ok()
                && selector.expect_exhausted().is_ok();
            let range = parser.parse_nested_block(|p| {
                let a = p.position().byte_index();
                while p.next_including_whitespace_and_comments().is_ok() {}
                Ok::<_, cssparser::ParseError<()>>((a, p.position().byte_index()))
            });
            if layer && let Ok((a, b)) = range {
                target = Some(LayerRule {
                    rule: rule_start..parser.position().byte_index(),
                    body: a..b,
                });
            }
        }
        if matches!(token, Token::CurlyBracketBlock | Token::Semicolon) {
            selector_start = None;
        }
    }
    target
}

fn declarations(body: &str) -> Vec<&str> {
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
    declarations
}

fn edit_layer_rule(css: &str, updates: &str, remove: &[&str]) -> String {
    let mut css = css.to_owned();
    if let Some(target) = last_layer_rule(&css) {
        let retained = declarations(&css[target.body.clone()])
            .into_iter()
            .filter(|declaration| {
                let mut p = Parser::new(declaration);
                let name = p.expect_ident_cloned().ok();
                !name.is_some_and(|name| remove.iter().any(|n| name.eq_ignore_ascii_case(n)))
            })
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .collect::<Vec<_>>()
            .join("\n  ");
        let separator = if retained.is_empty() || retained.ends_with(';') {
            ""
        } else {
            ";"
        };
        css.replace_range(
            target.body,
            &format!("\n  {retained}{separator}\n  {}\n", updates.trim()),
        );
    } else {
        css.push_str(&format!("\n.layer {{\n  {}\n}}", updates.trim()));
    }
    css
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
    edit_layer_rule(
        &layer_stylesheet(&css),
        &format!(
            "left: {x}px; top: {y}px;\n  width: {width}px; height: {height}px;\n  right: auto; bottom: auto;"
        ),
        &["left", "top", "right", "bottom", "width", "height", "inset"],
    )
}

fn merge_layer_rule(css: &str, rule: &str) -> (String, String) {
    let mut css = css.to_owned();
    let mut rule = rule.to_owned();
    if let Some(target) = last_layer_rule(&rule) {
        let updates = &rule[target.body];
        let names = declarations(updates)
            .into_iter()
            .filter_map(|d| {
                Parser::new(d)
                    .expect_ident_cloned()
                    .ok()
                    .map(|n| n.to_string())
            })
            .collect::<Vec<_>>();
        let names = names.iter().map(String::as_str).collect::<Vec<_>>();
        css = edit_layer_rule(&css, updates, &names);
        rule.replace_range(target.rule, "");
    }
    (css, rule)
}

/// Apply preset declarations in place; only extra rules need a managed block.
pub fn managed_css(css: &str, name: &str, rule: &str) -> String {
    let start = format!("/* editor {name} */");
    let end = format!("/* end editor {name} */");
    let mut css = css.to_owned();
    let mut previous = None;
    if let Some(a) = css.find(&start)
        && let Some(b) = css[a + start.len()..].find(&end)
    {
        let b = a + start.len() + b;
        previous = Some(css[a + start.len()..b].to_owned());
        css.replace_range(a..b + end.len(), "");
    }
    let mut css = layer_stylesheet(css.trim());
    if let Some(previous) = previous {
        // Older presets owned a second .layer rule. Preserve any geometry
        // edits made there when moving its declarations into the main rule.
        css = merge_layer_rule(&css, &layer_stylesheet(&previous)).0;
    }
    let (css, rule) = merge_layer_rule(&css, &layer_stylesheet(rule));
    if rule.trim().is_empty() {
        css
    } else {
        format!("{}\n{start}\n{}\n{end}", css.trim(), rule.trim())
    }
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
    fn layer_alias_does_not_rewrite_property_values_or_strings() {
        let css = ":layer {animation:layer_pulse 2s;--label:':layer';} @keyframes layer_pulse {from {opacity:.45;}to {opacity:1;}}";
        let normalized = layer_stylesheet(css);
        assert_eq!(normalized, css.replacen(":layer", ".layer", 1));
        assert_eq!(layer_stylesheet(&normalized), normalized);
        let nested =
            "@media (min-width:0px) {:layer {animation:layer_pulse 2s;}}:layer {opacity:1;}";
        assert_eq!(
            layer_stylesheet(nested),
            nested.replace(":layer {", ".layer {")
        );
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

    #[test]
    fn animation_and_dragging_keep_one_layer_rule() {
        let animation = "@keyframes layer_pulse {from {opacity:0.45;} to {opacity:1;}} .layer {animation:layer_pulse 2s ease-in-out infinite alternate;}";
        let keyframes = animation.split(" .layer").next().unwrap();
        let mut css = ".layer {left:0px;top:0px;width:100px;height:100px;--crop-left:10%;object-fit:cover;/* keep decoration */border:3px solid red;}".to_owned();
        for x in 0..10 {
            css = managed_css(&css, "animation", animation);
            assert_eq!(css.matches(".layer").count(), 1, "{css}");
            css = geometry_css(&css, x, 78, 1115, 763);
        }
        assert_eq!(css.matches(".layer").count(), 1, "{css}");
        assert_eq!(css.matches("@keyframes").count(), 1);
        assert_eq!(css.matches("animation:").count(), 1);
        assert!(css.contains(keyframes));
        assert!(css.contains("left: 9px"));
        assert!(css.contains("top: 78px"));
        assert!(css.contains("width: 1115px; height: 763px"));
        assert!(css.contains("--crop-left:10%"));
        assert!(css.contains("object-fit:cover"));
        assert!(css.contains("/* keep decoration */border:3px solid red;"));
        assert!(validate_css(&css).is_ok());
    }

    #[test]
    fn presets_replace_properties_in_the_existing_layer_rule() {
        for initial in [
            "object-fit:cover;opacity:.5",
            ":layer {object-fit:cover;opacity:.5}",
        ] {
            let mut css = initial.to_owned();
            for _ in 0..3 {
                css = managed_css(&css, "fit", ".layer {object-fit:contain;}");
                css = managed_css(
                    &css,
                    "frame",
                    ".layer {border:4px solid #ff9944;border-radius:24px;}",
                );
            }
            assert_eq!(css.matches(".layer").count(), 1, "{css}");
            assert_eq!(css.matches("object-fit:").count(), 1);
            assert_eq!(css.matches("border:").count(), 1);
            assert!(css.contains("object-fit:contain;"));
            assert!(css.contains("opacity:.5"));
            assert!(validate_css(&css).is_ok());
        }
    }

    #[test]
    fn replacing_legacy_animation_keeps_dragged_geometry() {
        let css = ".layer {left:0px;top:0px;width:100px;height:100px;object-fit:cover;}\n/* editor animation */\n@keyframes layer_pulse {from {opacity:.45;}to {opacity:1;}} .layer {left:134px;top:78px;width:1115px;height:763px;animation.layer_pulse 2s;}\n/* end editor animation */";
        let css = managed_css(
            css,
            "animation",
            "@keyframes layer_pulse {from {opacity:.45;}to {opacity:1;}} .layer {animation:layer_pulse 2s;}",
        );
        assert_eq!(css.matches(".layer").count(), 1, "{css}");
        assert_eq!(css.matches("@keyframes").count(), 1);
        assert!(css.contains("left:134px;"));
        assert!(css.contains("top:78px;"));
        assert!(css.contains("width:1115px;"));
        assert!(css.contains("height:763px;"));
        assert!(css.contains("animation:layer_pulse 2s;"));
        assert!(css.contains("object-fit:cover;"));
    }
}
