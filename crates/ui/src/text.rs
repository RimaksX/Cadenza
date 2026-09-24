//! The words Rust puts on the screen, in the language the listener chose.
//!
//! The markup says `@tr("…")` and Slint does the rest, but a view model builds
//! sentences in Rust and Slint's bundle is not reachable from here: the
//! function that reads it takes tables the markup compiler wrote and is not
//! exported. So this is the same catalogue, read again.
//!
//! **One source of truth.** `build.rs` parses the very `.po` the markup is
//! translated from and writes the table this reads, so a phrase cannot be
//! translated for the interface and left English for the view model.
//!
//! Lookup is linear over a sorted table. There are about forty phrases and each
//! is asked for once per refresh; a map would be a data structure to justify.

include!(concat!(env!("OUT_DIR"), "/translations.rs"));

use std::cell::Cell;

thread_local! {
    /// Which column of the table to read. Zero is the original.
    static LANGUAGE: Cell<usize> = const { Cell::new(0) };
}

/// Chooses the language these words come back in.
///
/// An unknown tag selects the original rather than failing: the interface
/// showing English is a worse day than the interface showing nothing.
pub fn select(tag: &str) {
    let index = LANGUAGES
        .iter()
        .position(|known| *known == tag)
        .unwrap_or(0);
    LANGUAGE.with(|current| current.set(index));
}

/// The phrase, translated if there is a translation for it.
///
/// A phrase may carry a `|` and a note after it, which is how two identical
/// English words that need different translations are told apart - "track|one"
/// against "tracks|many". Nothing after the bar is ever shown.
#[must_use]
pub fn tr(original: &'static str) -> &'static str {
    let index = LANGUAGE.with(Cell::get);
    let english = original.split('|').next().unwrap_or(original);
    if index == 0 {
        return english;
    }
    PHRASES
        .binary_search_by_key(&original, |(msgid, _)| msgid)
        .ok()
        .and_then(|found| PHRASES[found].1.get(index - 1).copied())
        .filter(|translated| !translated.is_empty())
        .unwrap_or(english)
}

/// The right form of a counted noun.
///
/// English needs two forms and Russian three, so all three are always asked
/// for and the language decides which it wants. The rule is the one every
/// Slavic catalogue carries: one, a few, and many, with the teens counting as
/// many because eleven is not one.
#[must_use]
pub fn plural(
    count: u64,
    one: &'static str,
    few: &'static str,
    many: &'static str,
) -> &'static str {
    let index = LANGUAGE.with(Cell::get);
    if LANGUAGES.get(index).copied() != Some("ru") {
        return if count == 1 { tr(one) } else { tr(many) };
    }
    let last = count % 10;
    let teen = count % 100;
    if last == 1 && teen != 11 {
        tr(one)
    } else if (2..=4).contains(&last) && !(12..=14).contains(&teen) {
        tr(few)
    } else {
        tr(many)
    }
}

/// The same, for a phrase that arrives at runtime rather than as a literal.
///
/// The built-in moods and equaliser presets are rows in the database, put there
/// by a migration, so their names reach the screen as `String`. A listener's
/// own preset is a `String` too and is not in the catalogue, which is the
/// answer for it: a name somebody typed is theirs and stays as typed.
#[must_use]
pub fn tr_owned(original: &str) -> String {
    let index = LANGUAGE.with(Cell::get);
    if index == 0 {
        return original.to_owned();
    }
    PHRASES
        .binary_search_by_key(&original, |(msgid, _)| msgid)
        .ok()
        .and_then(|found| PHRASES[found].1.get(index - 1).copied())
        .filter(|translated| !translated.is_empty())
        .unwrap_or(original)
        .to_owned()
}

/// The phrase with `{}` replaced once.
///
/// Slint's own `@tr` takes arguments; this is the same idea for the Rust side,
/// and one placeholder is all any of these sentences has.
#[must_use]
pub fn tr1(original: &'static str, argument: &str) -> String {
    tr(original).replacen("{}", argument, 1)
}

/// A folder picker whose window titles are in the listener's language.
///
/// The titles are chosen by core, which knows nothing about languages and
/// should not, and shown by infrastructure, which knows nothing about the
/// listener. This sits between them and translates the one string that crosses.
/// The picker is only ever opened from the window, on the thread whose language
/// [`select`] set.
pub struct LocalisedPicker<P>(pub P);

impl<P: cadenza_core::domain::ports::folder_picker::FolderPickerPort>
    cadenza_core::domain::ports::folder_picker::FolderPickerPort for LocalisedPicker<P>
{
    fn pick_folder(&self, title: &str) -> cadenza_core::Result<Option<std::path::PathBuf>> {
        self.0.pick_folder(&tr_owned(title))
    }

    fn pick_image(&self, title: &str) -> cadenza_core::Result<Option<std::path::PathBuf>> {
        self.0.pick_image(&tr_owned(title))
    }

    fn suggested_music_folder(&self) -> Option<std::path::PathBuf> {
        self.0.suggested_music_folder()
    }

    fn pick_file(
        &self,
        title: &str,
        kind: cadenza_core::domain::ports::folder_picker::FileKind,
    ) -> cadenza_core::Result<Option<std::path::PathBuf>> {
        self.0.pick_file(&tr_owned(title), kind)
    }

    fn save_file(
        &self,
        title: &str,
        kind: cadenza_core::domain::ports::folder_picker::FileKind,
        suggested: &str,
    ) -> cadenza_core::Result<Option<std::path::PathBuf>> {
        self.0.save_file(&tr_owned(title), kind, suggested)
    }
}

#[cfg(test)]
mod tests {
    use super::{plural, select, tr, tr1};

    #[test]
    fn russian_counts_one_few_and_many_with_the_teens_as_many() {
        select("ru");
        let form = |n| plural(n, "track|one", "tracks|few", "tracks|many");
        assert_eq!(form(1), "трек");
        assert_eq!(form(3), "трека");
        assert_eq!(form(5), "треков");
        assert_eq!(form(11), "треков", "eleven is not one");
        assert_eq!(form(12), "треков", "the teens are many");
        assert_eq!(form(21), "трек");
        assert_eq!(form(22), "трека");
        select("");
    }

    #[test]
    fn english_is_the_original_and_never_shows_the_note() {
        select("");
        assert_eq!(plural(1, "track|one", "tracks|few", "tracks|many"), "track");
        assert_eq!(
            plural(2, "track|one", "tracks|few", "tracks|many"),
            "tracks"
        );
        assert_eq!(tr("of|found"), "of");
        assert_eq!(tr1("Welcome, {}", "Sasha"), "Welcome, Sasha");
    }

    #[test]
    fn an_unknown_language_falls_back_to_the_original() {
        select("xx");
        assert_eq!(tr("Unknown artist"), "Unknown artist");
        select("");
    }
}
