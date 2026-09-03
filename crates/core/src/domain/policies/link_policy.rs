//! What counts as a link worth handing to a downloader.
//!
//! Deliberately not a URL parser. Cadenza does not fetch anything itself and
//! has no opinion about which sites exist, so validating a host or a path here
//! would be inventing knowledge this program does not have and would go stale
//! the week a site changed. What it does have an opinion about is the one thing
//! that matters at this boundary: whatever is handed to another program must be
//! a link and must not be able to pretend to be anything else.

/// Whether this is something to try fetching.
///
/// Three questions, and they are all about safety rather than taste.
///
/// It must begin with `http://` or `https://`. That is not politeness: a bare
/// word, a `-` or a Windows path would be read by the downloader as an option
/// or a filename, and an argument that can turn into a flag is how a text field
/// becomes a way to run something else. Anchoring the scheme means what arrives
/// can only be a link.
///
/// It must carry something after the scheme, because `https://` alone is a
/// press that would fail ten seconds later for no reason anybody could see.
///
/// And it must hold no whitespace or control characters. Those are how one
/// argument becomes two, and a newline in particular is how a line-based
/// protocol becomes two lines.
#[must_use]
pub fn is_a_link(text: &str) -> bool {
    let text = text.trim();

    let rest = text
        .strip_prefix("https://")
        .or_else(|| text.strip_prefix("http://"));

    let Some(rest) = rest else {
        return false;
    };

    !rest.is_empty()
        && !text
            .chars()
            .any(|character| character.is_whitespace() || character.is_control())
}

#[cfg(test)]
mod tests {
    use super::is_a_link;

    #[test]
    fn a_link_is_a_scheme_and_something_after_it() {
        assert!(is_a_link("https://example.com/watch?v=abc"));
        assert!(is_a_link("http://example.com/a.mp3"));
        assert!(
            is_a_link("  https://example.com/a  "),
            "surrounded by spaces"
        );

        assert!(!is_a_link(""));
        assert!(!is_a_link("https://"), "a scheme and nothing to fetch");
        assert!(!is_a_link("example.com/a"), "no scheme");
        assert!(!is_a_link("ftp://example.com/a"), "not a scheme we hand on");
    }

    #[test]
    fn nothing_that_could_become_a_second_argument_is_a_link() {
        // The whole reason this function exists. Each of these is a way for a
        // text field to stop being a text field.
        assert!(!is_a_link("--exec calc.exe"), "a bare option");
        assert!(!is_a_link("-x"), "a short option");
        assert!(!is_a_link("C:\\Windows\\System32\\calc.exe"), "a path");
        assert!(
            !is_a_link("https://example.com/a --exec calc.exe"),
            "an option smuggled in behind a real link"
        );
        assert!(
            !is_a_link("https://example.com/a\n--exec calc.exe"),
            "and the same thing on a second line"
        );
        assert!(
            !is_a_link("https://example.com/a\tb"),
            "any whitespace at all, not only a space"
        );
    }
}
