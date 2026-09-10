//! Performing artist.

use super::ids::ArtistId;
use super::value_objects::Timestamp;

/// An artist in the global catalogue.
///
/// Artists are global rather than per-profile: they describe the music, not the
/// listener. What is per-profile is which tracks a listener has in their library.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Artist {
    /// Stable identifier.
    pub id: ArtistId,
    /// Display name as read from tags.
    pub name: String,
    /// Name used for alphabetical ordering, e.g. `"Beatles, The"`.
    pub sort_name: String,
    /// When the row was created.
    pub created_at: Timestamp,
    /// When the row last changed.
    pub updated_at: Timestamp,
}

impl Artist {
    /// Derives a sort name by moving a leading English article to the end.
    ///
    /// Deliberately English-only and deliberately dumb: it covers the common case
    /// without pretending to understand every language's articles. Tags that carry
    /// an explicit sort name always win over this.
    pub fn derive_sort_name(name: &str) -> String {
        for article in ["The ", "A ", "An "] {
            if let Some(rest) = name.strip_prefix(article) {
                return format!("{rest}, {}", article.trim_end());
            }
        }
        name.to_owned()
    }
}

#[cfg(test)]
mod tests {
    use super::Artist;

    #[test]
    fn leading_articles_move_to_the_end() {
        assert_eq!(Artist::derive_sort_name("The Beatles"), "Beatles, The");
        assert_eq!(
            Artist::derive_sort_name("A Perfect Circle"),
            "Perfect Circle, A"
        );
    }

    #[test]
    fn names_without_an_article_are_left_alone() {
        assert_eq!(Artist::derive_sort_name("Portishead"), "Portishead");
        assert_eq!(
            Artist::derive_sort_name("Theatre of Tragedy"),
            "Theatre of Tragedy"
        );
    }
}
