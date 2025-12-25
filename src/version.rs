#[derive(Debug, Clone)]
pub struct ParsedRef {
    pub original_ref: String,
    pub normalized_for_semver: String,
    pub previous_ref: String,
    pub has_refs_tags_prefix: bool,
}

pub fn normalize_semver(tag: &str) -> String {
    let (core, suffix) = tag
        .find(|c| ['-', '+'].contains(&c))
        .map(|idx| (&tag[..idx], &tag[idx..]))
        .unwrap_or((tag, ""));
    if core.is_empty() {
        return tag.to_string();
    }
    let dot_count = core.matches('.').count();
    let normalized_core = match dot_count {
        0 => format!("{core}.0.0"),
        1 => format!("{core}.0"),
        _ => core.to_string(),
    };
    format!("{normalized_core}{suffix}")
}

pub fn parse_ref(raw: &str, default_refs_tags_prefix: bool) -> ParsedRef {
    fn strip_until_char(s: &str, c: char) -> Option<String> {
        s.find(c).map(|index| s[index + 1..].to_string())
    }

    let mut maybe_version = raw.to_string();
    let mut previous_ref = String::new();
    let mut has_refs_tags_prefix = default_refs_tags_prefix;

    if let Some(normalized_version) = maybe_version.strip_prefix("refs/tags/") {
        has_refs_tags_prefix = true;
        previous_ref = maybe_version.clone();
        maybe_version = normalized_version.to_string();
    }

    if let Some(normalized_version) = maybe_version.strip_prefix('v') {
        previous_ref = maybe_version.clone();
        maybe_version = normalized_version.to_string();
    }

    if let Some(normalized_version) = strip_until_char(&maybe_version, '-') {
        previous_ref = maybe_version.clone();
        maybe_version = normalized_version.to_string();
    }

    if previous_ref.is_empty() {
        previous_ref = maybe_version.clone();
    }

    ParsedRef {
        original_ref: raw.to_string(),
        normalized_for_semver: normalize_semver(&maybe_version),
        previous_ref,
        has_refs_tags_prefix,
    }
}
