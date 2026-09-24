//! What a filename says about a recording when its tags say nothing.
//!
//! Almost every music file that arrives without tags is called
//! `Artist - Title`. It is the convention of every downloader, every rip and
//! every person who has ever named a file by hand, and it carries the one
//! thing the tags were missing.
//!
//! Reading it is a decision rather than a string operation, which is why it is
//! here: getting it wrong shows a listener the wrong artist on a row they
//! cannot correct without knowing where it came from.

/// The separator, with its spaces.
///
/// Spaces on both sides, and that is the whole of the caution: `Wham!` and
/// `Nine Inch Nails` keep their punctuation, while `AC/DC - Back in Black`
/// splits where it should. A hyphen with no spaces belongs to the words around
/// it — `Jay-Z`, `Sun-Ra`, `re-recording` — and splitting on those would
/// invent an artist out of half a word.
const BETWEEN: &str = " - ";

/// The artist and the title a name carries, where the tags carried neither.
///
/// The artist is `None` where the name says nothing about one, which is
/// honest: half of "Untitled 3" is not an artist.
#[must_use]
pub fn artist_and_title(name: &str) -> (Option<&str>, &str) {
    let Some((artist, title)) = name.split_once(BETWEEN) else {
        return (None, name);
    };

    let artist = artist.trim();
    let title = title.trim();

    // Both halves have to be worth having. A name that begins or ends with the
    // separator is a name with a gap in it, not two facts.
    if artist.is_empty() || title.is_empty() {
        return (None, name);
    }

    (Some(artist), title)
}

#[cfg(test)]
mod tests {
    use super::artist_and_title;

    #[test]
    fn a_name_that_says_who_made_it() {
        assert_eq!(
            artist_and_title("Calvin Harris - This Is What You Came For"),
            (Some("Calvin Harris"), "This Is What You Came For")
        );
        assert_eq!(
            artist_and_title("AC/DC - Back in Black"),
            (Some("AC/DC"), "Back in Black")
        );

        // The first separator is the one that counts: what follows is the
        // title, dashes and all.
        assert_eq!(
            artist_and_title("Lost Frequencies - Are You With Me - Radio Edit"),
            (Some("Lost Frequencies"), "Are You With Me - Radio Edit")
        );
    }

    #[test]
    fn and_a_name_that_says_nothing_about_one() {
        assert_eq!(artist_and_title("Untitled 3"), (None, "Untitled 3"));

        // A hyphen inside a word is part of the word.
        assert_eq!(artist_and_title("Jay-Z"), (None, "Jay-Z"));

        // A gap where a half should be is not a fact.
        assert_eq!(artist_and_title(" - Hello"), (None, " - Hello"));
        assert_eq!(artist_and_title("Hello - "), (None, "Hello - "));
    }
}
