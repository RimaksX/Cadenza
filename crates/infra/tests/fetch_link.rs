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

use cadenza_core::domain::ports::fetcher::FetchPort;
use cadenza_infra::library::fetcher::ExternalFetcher;

/// The link a listener reported. Change it to whatever is being chased.
const LINK: &str = "https://www.youtube.com/watch?v=kXYiU_JCYtU";

#[test]
#[ignore = "needs yt-dlp, ffmpeg and the internet"]
fn a_link_comes_in_as_a_file() {
    let into = std::env::temp_dir().join("cdz-fetch-test");
    std::fs::create_dir_all(&into).expect("somewhere to put it");

    let landed: PathBuf = ExternalFetcher::new()
        .fetch(LINK, &into, &|percent| {
            if percent % 25 == 0 {
                println!("{percent}%");
            }
        })
        .unwrap_or_else(|err| panic!("what it actually said: {err}"));

    println!("landed at {}", landed.display());
    assert!(landed.exists(), "the file is where it says it is");
    assert_eq!(
        landed.extension().and_then(|extension| extension.to_str()),
        Some("mp3")
    );

    std::fs::remove_file(&landed).expect("tidied up");
}
