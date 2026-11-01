//! Small lossless lexer shared by the CSS and WGSL views. Validation remains
//! the responsibility of cssparser/Naga; highlighting also accepts partial code.
use super::CodeKind;

pub fn tokens(text: &str, kind: CodeKind) -> Vec<(&str, &'static str)> {
    let mut out = vec![];
    let mut i = 0;
    while i < text.len() {
        let start = i;
        let rest = &text[i..];
        let first = rest.chars().next().unwrap();
        let class = if rest.starts_with("/*") {
            i += rest.find("*/").map_or(rest.len(), |n| n + 2);
            "syntax-comment"
        } else if kind == CodeKind::Wgsl && rest.starts_with("//") {
            i += rest.find('\n').unwrap_or(rest.len());
            "syntax-comment"
        } else if first == '\'' || first == '"' {
            i += first.len_utf8();
            let mut escaped = false;
            for c in text[i..].chars() {
                i += c.len_utf8();
                if c == first && !escaped {
                    break;
                }
                escaped = c == '\\' && !escaped;
            }
            "syntax-string"
        } else if first.is_ascii_digit() || first == '#' {
            i += rest
                .chars()
                .take_while(|c| c.is_ascii_alphanumeric() || matches!(c, '#' | '.' | '%'))
                .map(char::len_utf8)
                .sum::<usize>();
            "syntax-number"
        } else if first.is_alphabetic() || matches!(first, '_' | '-' | '@') {
            i += rest
                .chars()
                .take_while(|c| c.is_alphanumeric() || matches!(c, '_' | '-' | '@'))
                .map(char::len_utf8)
                .sum::<usize>();
            let word = &text[start..i];
            if kind == CodeKind::Css && text[i..].trim_start().starts_with(':') {
                "syntax-property"
            } else if word.starts_with('@')
                || (kind == CodeKind::Wgsl
                    && [
                        "fn", "let", "var", "return", "if", "else", "struct", "const", "true",
                        "false",
                    ]
                    .contains(&word))
            {
                "syntax-keyword"
            } else {
                "syntax-name"
            }
        } else {
            i += first.len_utf8();
            "syntax-punctuation"
        };
        out.push((&text[start..i], class));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn highlighting_is_lossless_even_for_partial_unicode_code() {
        for kind in [CodeKind::Css, CodeKind::Wgsl] {
            for text in [
                "/* комментарий */\n.layer {color:#ff0044;content:'☃';}\n",
                "fn effect(uv:vec2f)->vec4f { // x\n\treturn \"unfinished\\\"",
            ] {
                let highlighted = tokens(text, kind);
                assert_eq!(
                    highlighted.iter().map(|(s, _)| *s).collect::<String>(),
                    text
                );
            }
        }
    }
}
