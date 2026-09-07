//! Bringing a track in from a link, through the code the window runs.
//!
//! Ignored by default: it needs `yt-dlp`, `ffmpeg` and the internet, none of
//! which a build machine is promised. Run it by hand when a listener reports a
//! link that will not come in, with the link they reported:
//!
//! ```text
//! cargo test -p cadenza-infra --test fetch_link -- --ignored --nocapture
//! ```
//!
//! It exists because three rounds of guessing at somebody else's error message
//! cost more than one run of the real thing would have (`MASTER_ISSUES` 90).

use std::path::PathBuf;

use cadenza_core::domain::ports::fetcher::{FetchPort, FetchWhat};
use cadenza_infra::library::fetcher::ExternalFetcher;

/// The link a listener reported. Change it to whatever is being chased.
const LINK: &str = "https://www.youtube.com/watch?v=kXYiU_JCYtU";

#[test]
#[ignore = "needs yt-dlp, ffmpeg and the internet"]
fn a_link_comes_in_as_a_file() {
    let into = std::env::temp_dir().join("cdz-fetch-test");
    std::fs::create_dir_all(&into).expect("somewhere to put it");

    // No archive: this test asks for the same link every time it is run, and a
    // record of having fetched it once would make every run after the first
    // prove nothing.
    let landed: Vec<PathBuf> = ExternalFetcher::new(None)
        .fetch(
            LINK,
            &into,
            FetchWhat::OneTrack,
            &|report| {
                if report.percent % 25 == 0 {
                    println!("{}%", report.percent);
                }
            },
            &|| false,
        )
        .unwrap_or_else(|err| panic!("what it actually said: {err}"));

    let [file] = landed.as_slice() else {
        panic!("one link, one track, and {} came back", landed.len());
    };
    println!("landed at {}", file.display());
    assert!(file.exists(), "the file is where it says it is");
    assert_eq!(
        file.extension().and_then(|extension| extension.to_str()),
        Some("mp3")
    );

    std::fs::remove_file(file).expect("tidied up");
}
