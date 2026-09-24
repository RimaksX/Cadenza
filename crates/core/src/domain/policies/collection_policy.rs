//! Which tracks an artist holds, and the order they play in.
//!
//! One answer for two askers: the page that lists an artist and the queue that
//! plays through them must agree on what comes next, or pressing the third row
//! would play the fourth.

use std::cmp::Ordering;
use std::collections::HashMap;

use crate::domain::ids::{AlbumId, ArtistId, MediaFileId};
use crate::domain::track::Track;

/// An artist's tracks in the library: album by album, the oldest first as a
/// discography reads, each in its own order; then whatever belongs to no
/// album, by title.
///
/// An album's age is the earliest year among its tracks, so the answer needs
/// nothing but the tracks - the queue, which has no albums to ask, and the
/// page, which has, must agree. Two albums of one year go in a fixed order.
#[must_use]
pub fn artist_order(tracks: &[Track], artist: ArtistId) -> Vec<MediaFileId> {
    let mut held: Vec<&Track> = tracks
        .iter()
        .filter(|track| track.is_in_library() && track.artist_id == Some(artist))
        .collect();
    // Each album's earliest year, among the tracks of it this artist has.
    let mut years: HashMap<AlbumId, Option<u16>> = HashMap::new();
    for track in &held {
        if let Some(album) = track.album_id {
            let earliest = years.entry(album).or_insert(None);
            if let Some(year) = track.year {
                *earliest = Some(earliest.map_or(year, |known| known.min(year)));
            }
        }
    }
    let album_key = |track: &Track| {
        track
            .album_id
            .map(|album| (years.get(&album).copied().flatten(), album))
    };
    held.sort_by(|a, b| match (album_key(a), album_key(b)) {
        (Some((a_year, a_album)), Some((b_year, b_album))) => {
            // An album with no year goes after those that have one.
            let years = match (a_year, b_year) {
                (Some(a), Some(b)) => a.cmp(&b),
                (Some(_), None) => Ordering::Less,
                (None, Some(_)) => Ordering::Greater,
                (None, None) => Ordering::Equal,
            };
            years
                .then_with(|| a_album.cmp(&b_album))
                .then_with(|| on_the_record(a, b))
        }
        (Some(_), None) => Ordering::Less,
        (None, Some(_)) => Ordering::Greater,
        (None, None) => folded(&a.title).cmp(&folded(&b.title)),
    });
    held.into_iter().map(|track| track.media_file_id).collect()
}

fn on_the_record(a: &Track, b: &Track) -> Ordering {
    // A number that is there comes before one that is not.
    let number = |track: &Track| {
        (
            track.disc_no.unwrap_or(u16::MAX),
            track.track_no.unwrap_or(u16::MAX),
        )
    };
    number(a)
        .cmp(&number(b))
        .then_with(|| folded(&a.title).cmp(&folded(&b.title)))
}

fn folded(text: &str) -> String {
    text.to_lowercase()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::ids::ProfileId;
    use crate::domain::value_objects::Timestamp;

    fn track(
        title: &str,
        artist: Option<ArtistId>,
        album: Option<AlbumId>,
        number: Option<u16>,
    ) -> Track {
        Track {
            profile_id: ProfileId::new(),
            media_file_id: MediaFileId::new(),
            title: title.into(),
            artist_id: artist,
            album_id: album,
            track_no: number,
            disc_no: None,
            year: None,
            added_at: Timestamp::from_millis(0),
            removed_at: None,
        }
    }

    #[test]
    fn an_artist_plays_album_by_album_oldest_first_then_the_rest() {
        let artist = ArtistId::new();
        let (old, new) = (AlbumId::new(), AlbumId::new());
        let dated = |mut t: Track, year: u16| {
            t.year = Some(year);
            t
        };
        let mut gone = track("gone", Some(artist), Some(old), Some(3));
        gone.removed_at = Some(Timestamp::from_millis(1));
        let tracks = vec![
            track("loose b", Some(artist), None, None),
            dated(track("new 1", Some(artist), Some(new), Some(1)), 2019),
            dated(track("old 2", Some(artist), Some(old), Some(2)), 2001),
            track("someone else", Some(ArtistId::new()), Some(old), Some(1)),
            track("loose a", Some(artist), None, None),
            dated(track("old 1", Some(artist), Some(old), Some(1)), 2001),
            gone,
        ];
        let order = artist_order(&tracks, artist);
        let titles: Vec<&str> = order
            .iter()
            .map(|id| {
                tracks
                    .iter()
                    .find(|t| t.media_file_id == *id)
                    .unwrap()
                    .title
                    .as_str()
            })
            .collect();
        assert_eq!(titles, ["old 1", "old 2", "new 1", "loose a", "loose b"]);
    }
}
