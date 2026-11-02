//! Scrub untrusted server text before adding our own terminal styling.
use serde_json::Value;

pub fn safe_text(text: &str, token: Option<&str>) -> String {
    let clean: String = text
        .chars()
        .filter(|c| {
            (!c.is_control() || matches!(c, '\n' | '\t'))
                && !matches!(c, '\u{202a}'..='\u{202e}' | '\u{2066}'..='\u{2069}')
        })
        .collect();
    match token.filter(|token| !token.is_empty()) {
        Some(token) => clean.replace(token, "[redacted]"),
        None => clean,
    }
}

pub fn scrub_json(value: &mut Value, token: Option<&str>) {
    match value {
        Value::String(text) => *text = safe_text(text, token),
        Value::Array(items) => items.iter_mut().for_each(|item| scrub_json(item, token)),
        Value::Object(object) => {
            for (key, mut value) in std::mem::take(object) {
                let normalized: String = key
                    .chars()
                    .filter(char::is_ascii_alphanumeric)
                    .flat_map(char::to_lowercase)
                    .collect();
                if matches!(
                    normalized.as_str(),
                    "token"
                        | "authorization"
                        | "accesstoken"
                        | "refreshtoken"
                        | "codeverifier"
                        | "clientsecret"
                        | "password"
                        | "secret"
                        | "cookie"
                        | "setcookie"
                        | "apikey"
                        | "bearertoken"
                ) {
                    value = Value::String("[redacted]".into());
                } else {
                    scrub_json(&mut value, token);
                }
                object.insert(safe_text(&key, token), value);
            }
        }
        _ => {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn strips_terminal_controls_and_bidi_before_redacting_known_secrets() {
        let text = "title\x1b]52;c;evil\x07\u{009b}31m\u{202e} se\x1bcret\n日本語";
        let result = safe_text(text, Some("secret"));
        assert!(!result.contains("secret"));
        assert!(
            !result
                .chars()
                .any(|c| c == '\x1b' || c == '\x07' || c == '\u{009b}' || c == '\u{202e}')
        );
        assert!(result.contains("[redacted]\n日本語"));
    }

    #[test]
    fn redacts_nested_secret_fields_and_decoded_json_strings() {
        let mut value = serde_json::json!({"data": [{"access_token":"other-secret", "title":"secret\u{001b}title"}], "Authorization":"Bearer unrelated"});
        scrub_json(&mut value, Some("secret"));
        let text = value.to_string();
        assert!(!text.contains("other-secret") && !text.contains("unrelated"));
        assert!(!text.contains("secret"));
        assert!(text.contains("[redacted]title"));
    }
}
