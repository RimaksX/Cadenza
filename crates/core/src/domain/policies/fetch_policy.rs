//! What a refusal from the downloader means.
//!
//! Sites change what they hand out — the signature of a URL, the shape of a
//! player, whether an address needs to be signed at all — and `yt-dlp` follows
//! them within days. A copy that is a few weeks old therefore fails in a
//! handful of recognisable ways that have nothing to do with the link somebody
//! pasted, and everything to do with the copy being old.
//!
//! Recognising them is a decision and not a string operation, which is why it
//! is here: what the interface does about it — offer to update, and say so — is
//! the whole difference between "this did not work" and "this can be fixed by
//! pressing that".

/// True where the refusal reads like a downloader that has fallen behind.
///
/// Deliberately narrow. Everything not listed is passed on as it came: a guess
/// appended to an error nobody understands is worse than the error alone, and
/// offering to update after "this video is private" would teach people that
/// the offer means nothing.
#[must_use]
pub fn looks_out_of_date(reason: &str) -> bool {
    const SIGNATURES: [&str; 6] = [
        // The far end refusing a request it used to allow, which is what a
        // stale signature looks like from here.
        "403",
        // The check that appears when a request cannot be told apart from a
        // robot's, because the part that proves otherwise was not sent.
        "sign in to confirm",
        // The player changed shape and the copy on this machine cannot read it.
        "unable to extract",
        "failed to extract",
        "nsig",
        "player",
    ];

    let reason = reason.to_lowercase();
    SIGNATURES
        .iter()
        .any(|signature| reason.contains(signature))
}

#[cfg(test)]
mod tests {
    use super::looks_out_of_date;

    #[test]
    fn the_refusals_a_stale_copy_gives() {
        assert!(looks_out_of_date(
            "unable to download video data: HTTP Error 403: Forbidden"
        ));
        assert!(looks_out_of_date(
            "Sign in to confirm you're not a bot. Use --cookies-from-browser"
        ));
        assert!(looks_out_of_date("Unable to extract player response"));
        assert!(looks_out_of_date(
            "Some formats may be missing: nsig extraction failed"
        ));
    }

    #[test]
    fn and_the_refusals_no_update_would_answer() {
        // These are about the link, not about the copy. Offering an update
        // here would be an offer that never works, which is how an offer stops
        // being read at all.
        assert!(!looks_out_of_date(
            "Video unavailable. This video is private"
        ));
        assert!(!looks_out_of_date(
            "This video is available to this channel's members"
        ));
        assert!(!looks_out_of_date(
            "nothing came back that could be turned into an mp3"
        ));
        assert!(!looks_out_of_date("the download did not finish"));
    }
}
