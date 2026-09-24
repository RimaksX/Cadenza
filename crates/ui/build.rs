//! Compiles the Slint markup into Rust, and the message catalogue with it.
//!
//! Only the root `.slint` file is named: everything it imports is followed, and
//! cargo is told to rerun when any of them changes.

use std::collections::BTreeMap;
use std::path::Path;

/// Where the catalogues live, one directory per language.
const TRANSLATIONS: &str = "translations";

fn main() {
    // Bundled rather than loaded through gettext: a player that reads no
    // network should not need a message catalogue on disk either, and
    // `libintl` on Windows is a dependency taken for one call. Slint looks for
    // `translations/<lang>/LC_MESSAGES/<crate>.po`.
    //
    // **No default context.** Slint would otherwise key every phrase by the
    // component it sits in, so moving a label between components would lose its
    // translation without saying anything.
    let config = slint_build::CompilerConfiguration::new()
        .with_bundled_translations(TRANSLATIONS)
        .with_default_translation_context(slint_build::DefaultTranslationContext::None);
    slint_build::compile_with_config("slint/app_window.slint", config)
        .expect("the Slint markup should compile");

    write_rust_catalogue();
}

/// Turns the same `.po` files into a table Rust can read.
///
/// The markup's half of the interface is translated by Slint; the sentences a
/// view model builds are not, and the function that reads Slint's bundle is not
/// exported. Rather than keep a second catalogue that could disagree with the
/// first, this reads the first one again.
fn write_rust_catalogue() {
    let root = Path::new(TRANSLATIONS);
    let mut languages: Vec<String> = Vec::new();
    // msgid -> one translation per language, in the order of `languages`.
    let mut phrases: BTreeMap<String, Vec<String>> = BTreeMap::new();

    let mut directories: Vec<_> = std::fs::read_dir(root)
        .expect("the translations directory should exist")
        .filter_map(std::result::Result::ok)
        .map(|entry| entry.path())
        .filter(|path| path.is_dir())
        .collect();
    directories.sort();

    for directory in &directories {
        let language = directory
            .file_name()
            .expect("a language directory has a name")
            .to_string_lossy()
            .into_owned();
        let catalogue = directory.join("LC_MESSAGES").join("cadenza-ui.po");
        if !catalogue.exists() {
            continue;
        }
        println!("cargo::rerun-if-changed={}", catalogue.display());

        let text = std::fs::read_to_string(&catalogue).expect("the catalogue should be readable");
        let here = languages.len();
        languages.push(language);
        for (msgid, msgstr) in parse_po(&text) {
            let row = phrases
                .entry(msgid)
                .or_insert_with(|| vec![String::new(); here]);
            row.resize(here, String::new());
            row.push(msgstr);
        }
    }

    for row in phrases.values_mut() {
        row.resize(languages.len(), String::new());
    }

    let mut out = String::from(
        "/// The languages the table below carries, in column order.\n\
         static LANGUAGES: &[&str] = &[\"\",",
    );
    for language in &languages {
        out.push_str(&format!("{language:?},"));
    }
    out.push_str("];\n\n/// Every phrase, sorted, with one translation per language.\n");
    out.push_str("static PHRASES: &[(&str, &[&str])] = &[\n");
    for (msgid, row) in &phrases {
        out.push_str(&format!("    ({msgid:?}, &["));
        for translated in row {
            out.push_str(&format!("{translated:?},"));
        }
        out.push_str("]),\n");
    }
    out.push_str("];\n");

    let destination =
        Path::new(&std::env::var("OUT_DIR").expect("cargo sets OUT_DIR")).join("translations.rs");
    std::fs::write(destination, out).expect("the catalogue table should be writable");
}

/// The `msgid`/`msgstr` pairs of a `.po`, ignoring everything else.
///
/// Enough of the format for a catalogue this project writes by hand: no
/// wrapped strings, no plurals, no obsolete entries. A file that grows those
/// will need more, and will say so by losing a phrase.
fn parse_po(text: &str) -> Vec<(String, String)> {
    let mut pairs = Vec::new();
    let mut msgid: Option<String> = None;

    for line in text.lines() {
        let line = line.trim();
        if let Some(rest) = line.strip_prefix("msgid ") {
            msgid = unquote(rest);
        } else if let Some(rest) = line.strip_prefix("msgstr ")
            && let (Some(id), Some(value)) = (msgid.take(), unquote(rest))
            && !id.is_empty()
            && !value.is_empty()
        {
            pairs.push((id, value));
        }
    }
    pairs
}

/// The contents of a quoted `.po` value.
fn unquote(raw: &str) -> Option<String> {
    let inner = raw.trim().strip_prefix('"')?.strip_suffix('"')?;
    Some(inner.replace("\\\"", "\"").replace("\\n", "\n"))
}
