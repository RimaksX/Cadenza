//! A temporary command line, until the UI exists.
//!
//! Scaffolding, deliberately: M6 brings the real interface and this goes away.
//! It exists because the M3 requirements — "a profile can be created", "a
//! profile can be switched", "the data is saved" — are worth being able to check
//! by hand and not only in a test run.
//!
//! Hand-rolled over `std::env::args` rather than an argument parser. Adding a
//! dependency to something scheduled for deletion is how dependencies become
//! permanent.

/// What the user asked for.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Command {
    /// Show the active profile and everything else that exists.
    Status,
    /// Create a profile and, if it is the first, start as it.
    Create(String),
    /// Switch to an existing profile by name.
    Switch(String),
    /// Delete a profile and everything scoped to it.
    Delete {
        /// Which profile.
        name: String,
        /// Set by `--yes`. Without it the command refuses.
        confirmed: bool,
    },
    /// Turn listening history on or off for the active profile.
    History(bool),
    /// List the folders the active profile scans.
    Folders,
    /// Add a folder to the active profile's library.
    AddFolder {
        /// Which folder.
        path: String,
        /// Whether to descend into subdirectories.
        recursive: bool,
    },
    /// Scan every folder of the active profile.
    Scan,
    /// List what is in the active profile's library.
    Tracks,
    /// List files waiting for an import decision.
    Reviews,
    /// Print where Cadenza keeps its files.
    Paths,
    /// Print the usage text.
    Help,
}

/// What to print when asked, and when the arguments make no sense.
pub const USAGE: &str = "\
cadenza — local music player (milestone M3: profiles and settings)

USAGE:
    cadenza                          show the active profile and the others
    cadenza create <name>            create a profile
    cadenza switch <name>            switch to a profile
    cadenza delete <name> --yes      delete a profile and all of its data
    cadenza history <on|off>         set history for the active profile

    cadenza folders                  list the folders being scanned
    cadenza add-folder <path> [-r]   add a folder, -r to include subfolders
    cadenza scan                     scan every folder and import what is new
    cadenza tracks                   list the library
    cadenza reviews                  list files waiting for a decision

    cadenza paths                    show where data is stored
    cadenza help                     this text

This command line is temporary scaffolding and is replaced by the interface in
milestone M6.";

/// Reads the arguments, excluding the program name.
pub fn parse<I: IntoIterator<Item = String>>(args: I) -> Result<Command, String> {
    let mut args = args.into_iter();

    let Some(verb) = args.next() else {
        return Ok(Command::Status);
    };

    let command = match verb.as_str() {
        "status" => Command::Status,
        "paths" => Command::Paths,
        "help" | "--help" | "-h" => Command::Help,

        "create" => Command::Create(require_value(args.next(), "create", "a profile name")?),
        "switch" => Command::Switch(require_value(args.next(), "switch", "a profile name")?),

        "delete" => {
            let name = require_value(args.next(), "delete", "a profile name")?;
            // Deleting a profile takes its library membership, playlists,
            // history and presets with it, and none of that comes back.
            let confirmed = args.next().as_deref() == Some("--yes");
            Command::Delete { name, confirmed }
        }

        "folders" => Command::Folders,
        "scan" => Command::Scan,
        "tracks" => Command::Tracks,
        "reviews" => Command::Reviews,

        "add-folder" => {
            let path = require_value(args.next(), "add-folder", "a folder path")?;
            // Subfolders are opt-in. Pointing at a drive root and walking all of
            // it by default is a surprise nobody wants twice.
            let recursive = matches!(args.next().as_deref(), Some("-r" | "--recursive"));
            Command::AddFolder { path, recursive }
        }

        "history" => match require_value(args.next(), "history", "on or off")?.as_str() {
            "on" => Command::History(true),
            "off" => Command::History(false),
            other => return Err(format!("history takes on or off, not {other:?}")),
        },

        other => return Err(format!("unknown command {other:?}")),
    };

    Ok(command)
}

fn require_value(value: Option<String>, verb: &str, expected: &str) -> Result<String, String> {
    match value {
        Some(value) if !value.trim().is_empty() => Ok(value),
        _ => Err(format!("{verb} needs {expected}")),
    }
}

#[cfg(test)]
mod tests {
    use super::{Command, parse};

    fn parse_args(args: &[&str]) -> Result<Command, String> {
        parse(args.iter().map(|arg| (*arg).to_owned()))
    }

    #[test]
    fn no_arguments_shows_the_status() {
        assert_eq!(parse_args(&[]), Ok(Command::Status));
    }

    #[test]
    fn the_verbs_parse() {
        assert_eq!(
            parse_args(&["create", "Sasha"]),
            Ok(Command::Create("Sasha".to_owned()))
        );
        assert_eq!(
            parse_args(&["switch", "Kim"]),
            Ok(Command::Switch("Kim".to_owned()))
        );
        assert_eq!(parse_args(&["history", "on"]), Ok(Command::History(true)));
        assert_eq!(parse_args(&["history", "off"]), Ok(Command::History(false)));
        assert_eq!(parse_args(&["paths"]), Ok(Command::Paths));
    }

    #[test]
    fn deleting_needs_saying_so_twice() {
        assert_eq!(
            parse_args(&["delete", "Sasha"]),
            Ok(Command::Delete {
                name: "Sasha".to_owned(),
                confirmed: false
            }),
            "the parse succeeds; the refusal happens where the consequence is known"
        );
        assert_eq!(
            parse_args(&["delete", "Sasha", "--yes"]),
            Ok(Command::Delete {
                name: "Sasha".to_owned(),
                confirmed: true
            })
        );
    }

    #[test]
    fn a_missing_or_blank_value_is_reported_rather_than_guessed() {
        assert!(parse_args(&["create"]).is_err());
        assert!(parse_args(&["create", "   "]).is_err());
        assert!(parse_args(&["switch"]).is_err());
        assert!(parse_args(&["history"]).is_err());
        assert!(parse_args(&["history", "maybe"]).is_err());
    }

    #[test]
    fn an_unknown_verb_names_itself_in_the_error() {
        let err = parse_args(&["frobnicate"]).expect_err("not a command");
        assert!(err.contains("frobnicate"), "got {err}");
    }
}
