//! Backward-compatible image alternatives for Vision steps and guards.

use std::collections::HashSet;

pub const MAX_TEMPLATE_CANDIDATES: usize = 8;

/// Primary legacy path followed by distinct alternatives, preserving order.
pub fn candidate_paths<'a>(primary: &'a str, alternatives: &'a [String]) -> Vec<&'a str> {
    let mut seen = HashSet::new();
    std::iter::once(primary)
        .chain(alternatives.iter().map(String::as_str))
        .map(str::trim)
        .filter(|path| !path.is_empty() && seen.insert(*path))
        .collect()
}

pub fn candidate_limit_error(primary: &str, alternatives: &[String]) -> Option<String> {
    let count = candidate_paths(primary, alternatives).len();
    (count > MAX_TEMPLATE_CANDIDATES).then(|| {
        format!(
            "image alternatives exceed the {MAX_TEMPLATE_CANDIDATES}-image limit ({count} configured)"
        )
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn singular_image_stays_first_and_duplicates_are_removed() {
        let alternatives = vec![
            "hover.png".to_string(),
            "normal.png".to_string(),
            String::new(),
            "hover.png".to_string(),
        ];

        assert_eq!(
            candidate_paths("normal.png", &alternatives),
            vec!["normal.png", "hover.png"]
        );
    }

    #[test]
    fn alternatives_work_when_legacy_primary_is_empty() {
        assert_eq!(
            candidate_paths("", &["hover.png".to_string()]),
            vec!["hover.png"]
        );
    }

    #[test]
    fn limit_counts_distinct_non_empty_paths() {
        let paths = (0..=MAX_TEMPLATE_CANDIDATES)
            .map(|index| format!("{index}.png"))
            .collect::<Vec<_>>();
        assert!(candidate_limit_error("", &paths).is_some());
        assert!(candidate_limit_error(&paths[0], &paths[1..MAX_TEMPLATE_CANDIDATES]).is_none());
    }
}
