//! What counts as a link worth handing to a downloader.
//!
//! Deliberately not a URL parser. Cadenza does not fetch anything itself and
//! has no opinion about which sites exist, so validating a host or a path here
//! would be inventing knowledge this program does not have and would go stale
//! the week a site changed. What it does have an opinion about is the one thing
//! that matters at this boundary: whatever is handed to another program must be
//! a link and must not be able to pretend to be anything else.
//!
//! One exception, at the bottom: the handful of services whose audio is
//! encrypted and which therefore cannot be fetched by anything. Naming them is
//! not knowledge about the web that goes stale — it is the difference between
//! an answer and ten seconds of waiting for somebody else's error message.

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

/// The streaming services whose tracks no downloader can fetch.
///
/// Their audio is encrypted and handed out under a licence the player holds;
/// what a downloader would find at one of these addresses is a page, not a
/// recording. That is a fact about those services rather than a shortcoming of
/// whichever tool is installed, so no newer version of anything will change it.
const LOCKED: [(&str, &str); 3] = [
    ("music.apple.com", "Apple Music"),
    ("tidal.com", "Tidal"),
    ("deezer.com", "Deezer"),
];

/// The service whose links name a recording without holding one anybody can
/// have.
const MATCHED: &str = "spotify.com";

/// Which program answers a link, if any does.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LinkHandler {
    /// The downloader fetches what the link points at.
    Downloader,
    /// The link names a recording and the audio is found elsewhere.
    ///
    /// **Nothing takes audio out of Spotify.** Its streams are encrypted, and
    /// every service that claims otherwise does what this does: it reads the
    /// title, the artist and the album from the link and then finds that
    /// recording on YouTube. `spotDL` says so about itself in as many words —
    /// it uses YouTube as its source "to avoid any issues related to
    /// downloading music from Spotify".
    ///
    /// So what a listener gets is the same audio the other button gets, with
    /// better names on it, chosen by a match that is usually right and
    /// sometimes finds a live version instead (`MASTER_ISSUES` 99).
    Matcher,
    /// Nothing does, and the service is named so the answer arrives at once.
    Refused(&'static str),
}

/// Which program to hand a link to.
#[must_use]
pub fn handler_for(text: &str) -> LinkHandler {
    let text = text.to_lowercase();

    if let Some((_, service)) = LOCKED.iter().find(|(host, _)| text.contains(host)) {
        return LinkHandler::Refused(service);
    }
    if text.contains(MATCHED) {
        return LinkHandler::Matcher;
    }

    LinkHandler::Downloader
}

/// The service this link belongs to, when it is one nothing can fetch from.
///
/// Spotify is not among them any more, and it never should have been for the
/// stated reason: what was true is that nothing can take its *audio*, and what
/// this list is for is services where nothing can be got at all.
///
/// The one exception to "this is not a URL parser", and it earns itself: these
/// four are what people actually have open when they copy a link to a song.
/// Without this the refusal arrives ten seconds later as a page of somebody
/// else's error output, and it reads as a fault in Cadenza rather than as the
/// answer, which is: that track, on a site that will part with it.
///
/// A substring rather than a parsed host. Getting it wrong in the unlikely
/// direction costs a listener one refused link they can rephrase; there is
/// nothing here to exploit, because the only thing this decides is which
/// sentence to show.
#[must_use]
pub fn locked_service(text: &str) -> Option<&'static str> {
    let text = text.to_lowercase();

    LOCKED
        .iter()
        .find(|(host, _)| text.contains(host))
        .map(|(_, service)| *service)
}

#[cfg(test)]
mod tests {
    use super::{is_a_link, locked_service};

    #[test]
    fn which_program_answers_which_link() {
        use super::{LinkHandler, handler_for};

        // The one that names a recording without holding one: read the names,
        // find the recording elsewhere.
        assert_eq!(
            handler_for("https://open.spotify.com/track/4cOdK2wGLETKBW3PvgPWqT"),
            LinkHandler::Matcher
        );
        assert_eq!(
            handler_for("https://open.spotify.com/playlist/37i9dQZF1DX"),
            LinkHandler::Matcher
        );

        // The ones where nothing can be got at all.
        assert_eq!(
            handler_for("https://music.apple.com/us/album/x/1"),
            LinkHandler::Refused("Apple Music")
        );
        assert_eq!(
            handler_for("HTTPS://TIDAL.COM/track/1"),
            LinkHandler::Refused("Tidal")
        );

        // And everything else, which is the downloader's job.
        assert_eq!(
            handler_for("https://www.youtube.com/watch?v=abc"),
            LinkHandler::Downloader
        );
        assert_eq!(
            handler_for("https://soundcloud.com/a/b"),
            LinkHandler::Downloader
        );
        assert_eq!(
            handler_for("https://www.youtube.com/watch?v=spotify"),
            LinkHandler::Downloader,
            "the host is what is matched, not the word: this is a YouTube link"
        );
    }

    #[test]
    fn a_service_that_encrypts_its_audio_is_named_rather_than_attempted() {
        assert_eq!(
            locked_service("https://music.apple.com/us/album/x/1"),
            Some("Apple Music")
        );
        assert_eq!(locked_service("HTTPS://TIDAL.COM/track/1"), Some("Tidal"));

        // Spotify is answered rather than refused now: its links name a
        // recording, and the recording is findable.
        assert_eq!(
            locked_service("https://open.spotify.com/track/4cOdK2wGLETKBW3"),
            None
        );

        // What can be fetched is left alone.
        assert_eq!(locked_service("https://www.youtube.com/watch?v=abc"), None);
        assert_eq!(locked_service("https://soundcloud.com/a/b"), None);
    }

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
