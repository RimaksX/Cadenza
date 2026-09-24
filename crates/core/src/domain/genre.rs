//! Genre label.

use super::ids::GenreId;

/// A genre label, unique by name across the catalogue.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Genre {
    /// Stable identifier.
    pub id: GenreId,
    /// Normalised genre name.
    pub name: String,
}

impl Genre {
    /// Normalises a raw tag value so that `"Drum & Bass"`, `"drum & bass"` and
    /// `"  Drum & Bass "` collapse to a single genre row.
    ///
    /// Case folding is ASCII-only, which is enough for the genre vocabulary that
    /// actually appears in tags.
    pub fn normalize(raw: &str) -> String {
        raw.trim().to_ascii_lowercase()
    }
}

#[cfg(test)]
mod tests {
    use super::Genre;

    #[test]
    fn spelling_variants_collapse_to_one_name() {
        assert_eq!(Genre::normalize("  Drum & Bass "), "drum & bass");
        assert_eq!(
            Genre::normalize("DRUM & BASS"),
            Genre::normalize("drum & bass")
        );
    }
}
