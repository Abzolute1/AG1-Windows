//! Data-only desktop theme plugins. No CSS, URLs, scripts or executable hooks.
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ThemePlugin {
    pub schema_version: u32,
    pub id: String,
    pub name: String,
    pub mode: String,
    pub colors: BTreeMap<String, String>,
}
const COLORS: [&str; 22] = [
    "background",
    "foreground",
    "sidebar",
    "card",
    "popover",
    "muted",
    "muted-foreground",
    "border",
    "primary",
    "primary-foreground",
    "accent",
    "accent-foreground",
    "input",
    "ring",
    "secondary",
    "secondary-foreground",
    "destructive",
    "code",
    "link",
    "syntax-keyword",
    "syntax-string",
    "syntax-comment",
];
pub fn is_builtin(id: &str) -> bool {
    static IDS: std::sync::OnceLock<Vec<String>> = std::sync::OnceLock::new();
    id == "system"
        || IDS
            .get_or_init(|| {
                let themes: Vec<ThemePlugin> =
                    serde_json::from_str(include_str!("../../src/builtin-themes.json"))
                        .expect("bundled theme data");
                themes.into_iter().map(|t| t.id).collect()
            })
            .iter()
            .any(|value| value == id)
}
pub fn validate(value: serde_json::Value) -> Result<ThemePlugin, String> {
    if value.to_string().len() > 32768 {
        return Err("Theme file exceeds 32 KB".into());
    }
    let theme: ThemePlugin = serde_json::from_value(value).map_err(|_| "Invalid AG1 theme file")?;
    if theme.schema_version != 1
        || !["dark", "light"].contains(&theme.mode.as_str())
        || theme.id.is_empty()
        || theme.id.len() > 80
        || !theme
            .id
            .bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-')
        || is_builtin(&theme.id)
        || theme.name.trim().is_empty()
        || theme.name.len() > 80
        || theme.name.chars().any(char::is_control)
        || theme.colors.len() < 19
        || COLORS[..19]
            .iter()
            .any(|key| !theme.colors.contains_key(*key))
        || theme.colors.iter().any(|(key, color)| {
            !COLORS.contains(&key.as_str())
                || color.len() != 7
                || !color.starts_with('#')
                || !color[1..].bytes().all(|b| b.is_ascii_hexdigit())
        })
    {
        return Err(
            "Theme needs a unique lowercase ID, a name, dark/light mode and valid palette colors"
                .into(),
        );
    }
    Ok(theme)
}
#[cfg(test)]
mod tests {
    use super::*;
    fn theme() -> serde_json::Value {
        let mut t: serde_json::Value =
            serde_json::from_str(include_str!("../../src/builtin-themes.json")).unwrap();
        t[0]["id"] = "custom-graphite".into();
        t[0].clone()
    }
    #[test]
    fn data_plugin_rejects_css_scripts_reserved_ids_and_missing_tokens() {
        assert!(validate(theme()).is_ok());
        for bad in ["url(https://example.invalid)", "#fff", "#ff00xz", "#é0000"] {
            let mut t = theme();
            t["colors"]["background"] = bad.into();
            assert!(validate(t).is_err());
        }
        let mut t = theme();
        t["script"] = "alert(1)".into();
        assert!(validate(t).is_err());
        let mut t = theme();
        t["id"] = "dark".into();
        assert!(validate(t).is_err());
        let mut t = theme();
        t["colors"].as_object_mut().unwrap().remove("sidebar");
        assert!(validate(t).is_err());
    }
}
