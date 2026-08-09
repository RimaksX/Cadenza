//! Cleaning up what tags actually contain.
//!
//! Real tags are messy: padded with spaces, carrying several genres in one
//! string with no agreed separator, and full of empty values that are not quite
//! absent. Normalising here means the rest of the system never has to wonder
//! whether `"Rock "` and `"rock"` are the same genre.

/// Separators that appear between genres inside a single tag value.
///
/// No standard agrees on one, so files carry all of these. Splitting on all of
/// them is wrong for the rare genre with a slash in its name; leaving them
/// joined is wrong far more often.
const GENRE_SEPARATORS: [char; 4] = [';', '/', ',', '|'];

/// Trims a tag value and discards it if nothing is left.
///
/// An empty tag is the same as a missing one, and treating them differently
/// would produce tracks titled `""`.
#[must_use]
pub fn tag_value(raw: Option<String>) -> Option<String> {
    let trimmed = raw?.trim().to_owned();
    if trimmed.is_empty() {
        None
    } else {
        Some(collapse_whitespace(&trimmed))
    }
}

/// Replaces runs of whitespace with a single space.
#[must_use]
pub fn collapse_whitespace(value: &str) -> String {
    value.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// Splits one genre tag into the genres it names.
#[must_use]
pub fn genres(raw: Option<String>) -> Vec<String> {
    let Some(value) = tag_value(raw) else {
        return Vec::new();
    };

    let mut seen: Vec<String> = Vec::new();
    for part in value.split(GENRE_SEPARATORS) {
        let cleaned = collapse_whitespace(part.trim());
        if cleaned.is_empty() {
            continue;
        }
        // The same genre twice in one tag is common enough to be worth ignoring.
        if !seen
            .iter()
            .any(|existing| existing.eq_ignore_ascii_case(&cleaned))
        {
            seen.push(cleaned);
        }
    }
    seen
}

/// The title to show when a file has no title tag.
///
/// The file stem, not "Unknown": an untagged file usually has a filename that
/// says exactly what it is, and showing a hundred rows of "Unknown" helps
/// nobody.
#[must_use]
pub fn title_from_stem(stem: &str) -> String {
    let cleaned = collapse_whitespace(&stem.replace('_', " "));
    if cleaned.is_empty() {
        "Untitled".to_owned()
    } else {
        cleaned
    }
}

#[cfg(test)]
mod tests {
    use super::{genres, tag_value, title_from_stem};

    #[test]
    fn padding_and_empty_values_are_dropped() {
        assert_eq!(
            tag_value(Some("  Portishead ".to_owned())).as_deref(),
            Some("Portishead")
        );
        assert_eq!(tag_value(Some("   ".to_owned())), None);
        assert_eq!(tag_value(Some(String::new())), None);
        assert_eq!(tag_value(None), None);
    }

    #[test]
    fn internal_whitespace_is_collapsed() {
        assert_eq!(
            tag_value(Some("Massive\t\tAttack".to_owned())).as_deref(),
            Some("Massive Attack")
        );
    }

    #[test]
    fn a_multi_genre_tag_becomes_several_genres() {
        assert_eq!(
            genres(Some("Trip-Hop; Electronic / Downtempo".to_owned())),
            vec!["Trip-Hop", "Electronic", "Downtempo"]
        );
    }

    #[test]
    fn repeats_and_blanks_inside_a_genre_tag_are_ignored() {
        assert_eq!(
            genres(Some("Rock;;rock, Rock".to_owned())),
            vec!["Rock"],
            "the same genre spelled differently is still one genre"
        );
        assert_eq!(genres(Some("  ".to_owned())), Vec::<String>::new());
        assert_eq!(genres(None), Vec::<String>::new());
    }

    #[test]
    fn an_untagged_file_is_named_after_itself() {
        assert_eq!(title_from_stem("01_Mysterons"), "01 Mysterons");
        assert_eq!(title_from_stem("  "), "Untitled");
    }
}
