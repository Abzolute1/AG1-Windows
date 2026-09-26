//! Pure SDK path spelling compatibility. These helpers grant no filesystem access.
use crate::DomainError;
use icu_normalizer::DecomposingNormalizer;

pub fn expand(input: &str, home: Option<&str>) -> Result<String, DomainError> {
    if input.is_empty() || input.len() > 4096 || input.contains('\0') {
        return Err(DomainError::InvalidInput("file path".into()));
    }
    let value: String = input
        .strip_prefix('@')
        .unwrap_or(input)
        .chars()
        .map(|c| {
            if matches!(
                c,
                '\u{a0}' | '\u{2000}'..='\u{200a}' | '\u{202f}' | '\u{205f}' | '\u{3000}'
            ) {
                ' '
            } else {
                c
            }
        })
        .collect();
    let expanded = if value == "~" || value.starts_with("~/") {
        let home = home.ok_or_else(|| DomainError::Unavailable("home directory".into()))?;
        format!("{home}{}", &value[1..])
    } else {
        value
    };
    if expanded.is_empty() || expanded.len() > 4096 || expanded.contains('\0') {
        return Err(DomainError::InvalidInput("file path".into()));
    }
    Ok(expanded)
}

/// Ordered fallbacks are used only when the literal contained path is missing.
/// Every returned candidate must be checked by the caller's filesystem capability.
pub fn read_variants(path: &str) -> Vec<String> {
    let nfd = DecomposingNormalizer::new_nfd()
        .normalize(path)
        .into_owned();
    let candidates = [
        path.replace(" AM.", "\u{202f}AM.")
            .replace(" PM.", "\u{202f}PM."),
        nfd.clone(),
        path.replace('\'', "\u{2019}"),
        nfd.replace('\'', "\u{2019}"),
    ];
    let mut variants = Vec::new();
    for candidate in candidates {
        if candidate != path && candidate.len() <= 4096 && !variants.contains(&candidate) {
            variants.push(candidate);
        }
    }
    variants
}
