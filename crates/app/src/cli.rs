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
    /// Open the window. What running Cadenza with no arguments means.
    Ui,
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
    /// Set the active profile's theme. `true` is dark.
    Theme(bool),
    /// Turn crossfading on or off, and say how long it takes.
    Crossfade {
        /// Whether ordinary tracks fade into each other at all.
        enabled: bool,
        /// How long the fade runs. `None` leaves the stored length alone.
        seconds: Option<u64>,
    },
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
    /// Correct the genres of one track, for the active profile only.
    Genre {
        /// Position in the `tracks` listing, counting from one.
        index: usize,
        /// The genres to file it under. Empty means none at all.
        names: Vec<String>,
        /// Set by `--reset`: forget the correction and use the file's own tags.
        reset: bool,
    },
    /// List the active profile's playlists.
    Playlists,
    /// Do something to one playlist.
    Playlist(PlaylistCommand),
    /// List files waiting for an import decision.
    Reviews,
    /// Analyse files that have not been analysed yet.
    Analyze {
        /// How many files to work through. `None` means until there are none
        /// left.
        limit: Option<usize>,
    },
    /// Watch the folders and import changes as they happen.
    Watch,
    /// Play a file, with pause, seek and volume from the keyboard.
    Play(String),
    /// Print where Cadenza keeps its files.
    Paths,
    /// Print the usage text.
    Help,
}

/// What to do to one playlist.
///
/// Playlists are named and their entries numbered, for the same reason tracks
/// are: nobody is going to type a UUID at a command line.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PlaylistCommand {
    /// List what is in a playlist.
    Show(String),
    /// Create an empty playlist.
    New(String),
    /// Rename one.
    Rename {
        /// The playlist as it is called now.
        from: String,
        /// What to call it.
        to: String,
    },
    /// Delete one. The tracks stay in the library.
    Delete {
        /// Which playlist.
        name: String,
        /// Set by `--yes`. Without it the command refuses.
        confirmed: bool,
    },
    /// Append a track from the library listing.
    Add {
        /// Which playlist.
        name: String,
        /// Position in the `tracks` listing, counting from one.
        track: usize,
    },
    /// Remove an entry by its position in the playlist.
    Remove {
        /// Which playlist.
        name: String,
        /// Position in the playlist, counting from one.
        entry: usize,
    },
    /// Move an entry to another position.
    Move {
        /// Which playlist.
        name: String,
        /// Where the entry is, counting from one.
        from: usize,
        /// Where it should go, counting from one.
        to: usize,
    },
}

/// What to print when asked, and when the arguments make no sense.
pub const USAGE: &str = "\
cadenza — local music player, entirely offline

USAGE:
    cadenza                          open the window
    cadenza status                   show the active profile and the others
    cadenza create <name>            create a profile
    cadenza switch <name>            switch to a profile
    cadenza delete <name> --yes      delete a profile and all of its data
    cadenza history <on|off>         set history for the active profile
    cadenza theme <dark|light>       set the theme for the active profile
    cadenza crossfade <on|off> [s]   fade ordinary tracks into each other, 3 to 5 seconds

    cadenza folders                  list the folders being scanned
    cadenza add-folder <path> [-r]   add a folder, -r to include subfolders
    cadenza scan                     scan every folder and import what is new
    cadenza watch                    keep watching for changes until Enter
    cadenza tracks                   list the library with genres
    cadenza genre <n> <genre>...     set the genres of track n, this profile only
    cadenza genre <n> --reset        restore the genres the file itself carries
    cadenza reviews                  list files waiting for a decision
    cadenza analyze [n]              work out tempo, key and energy for n files

    cadenza playlists                list the playlists
    cadenza playlist show <name>     list what is in one
    cadenza playlist new <name>      create an empty playlist
    cadenza playlist rename <a> <b>  rename it
    cadenza playlist delete <n> --yes  delete it, keeping its tracks
    cadenza playlist add <name> <n>  append track n from: cadenza tracks
    cadenza playlist remove <n> <e>  remove entry e from playlist n
    cadenza playlist move <n> <a> <b>  move entry a to position b

    cadenza play <file>              play a file: p pause, s <sec> seek,
                                     v <0-100> volume, q quit

    cadenza paths                    show where data is stored
    cadenza help                     this text

This command line is temporary scaffolding and is replaced by the interface in
milestone M6.";

/// Reads the arguments, excluding the program name.
pub fn parse<I: IntoIterator<Item = String>>(args: I) -> Result<Command, String> {
    let mut args = args.into_iter();

    // No arguments is the ordinary way to start a music player.
    let Some(verb) = args.next() else {
        return Ok(Command::Ui);
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
        "analyze" => Command::Analyze {
            limit: match args.next() {
                None => None,
                Some(value) => Some(
                    value
                        .parse()
                        .map_err(|_| format!("analyze needs a number of files, not {value:?}"))?,
                ),
            },
        },
        "playlists" => Command::Playlists,
        "playlist" => Command::Playlist(parse_playlist(&mut args)?),
        "watch" => Command::Watch,
        "play" => Command::Play(require_value(args.next(), "play", "a file to play")?),

        "add-folder" => {
            let path = require_value(args.next(), "add-folder", "a folder path")?;
            // Subfolders are opt-in. Pointing at a drive root and walking all of
            // it by default is a surprise nobody wants twice.
            let recursive = matches!(args.next().as_deref(), Some("-r" | "--recursive"));
            Command::AddFolder { path, recursive }
        }

        "genre" => {
            let index = require_value(args.next(), "genre", "a track number from: cadenza tracks")?
                .parse::<usize>()
                .map_err(|_| "genre takes a track number, for example: genre 3 Jazz".to_owned())?;

            let rest: Vec<String> = args.collect();
            let reset = rest.iter().any(|word| word == "--reset");
            let names = rest
                .into_iter()
                .filter(|word| word != "--reset")
                .collect::<Vec<_>>();

            if reset && !names.is_empty() {
                return Err("--reset takes no genres: it restores the file's own".to_owned());
            }

            Command::Genre {
                index,
                names,
                reset,
            }
        }

        "theme" => match require_value(args.next(), "theme", "dark or light")?.as_str() {
            "dark" => Command::Theme(true),
            "light" => Command::Theme(false),
            other => return Err(format!("theme takes dark or light, not {other:?}")),
        },

        "crossfade" => {
            let enabled = match require_value(args.next(), "crossfade", "on or off")?.as_str() {
                "on" => true,
                "off" => false,
                other => return Err(format!("crossfade takes on or off, not {other:?}")),
            };
            let seconds = match args.next() {
                Some(value) => Some(value.parse::<u64>().map_err(|_| {
                    format!("crossfade takes a length in whole seconds, not {value:?}")
                })?),
                None => None,
            };
            Command::Crossfade { enabled, seconds }
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

/// Reads what follows `playlist`.
///
/// Every action names one, so the verb comes first and the name after it —
/// `playlist show <name>` rather than `playlist <name>`, which would make a
/// playlist called "add" unreachable.
fn parse_playlist<I: Iterator<Item = String>>(args: &mut I) -> Result<PlaylistCommand, String> {
    let verb = require_value(
        args.next(),
        "playlist",
        "show, new, rename, delete, add, remove or move",
    )?;

    let command = match verb.as_str() {
        "show" => PlaylistCommand::Show(require_value(args.next(), "playlist show", "a name")?),
        "new" => PlaylistCommand::New(require_value(args.next(), "playlist new", "a name")?),

        "rename" => PlaylistCommand::Rename {
            from: require_value(args.next(), "playlist rename", "the playlist to rename")?,
            to: require_value(args.next(), "playlist rename", "the new name")?,
        },

        "delete" => PlaylistCommand::Delete {
            name: require_value(args.next(), "playlist delete", "a name")?,
            confirmed: args.next().as_deref() == Some("--yes"),
        },

        "add" => PlaylistCommand::Add {
            name: require_value(args.next(), "playlist add", "a playlist name")?,
            track: require_number(
                args.next(),
                "playlist add",
                "a track number from: cadenza tracks",
            )?,
        },

        "remove" => PlaylistCommand::Remove {
            name: require_value(args.next(), "playlist remove", "a playlist name")?,
            entry: require_number(args.next(), "playlist remove", "an entry number")?,
        },

        "move" => PlaylistCommand::Move {
            name: require_value(args.next(), "playlist move", "a playlist name")?,
            from: require_number(args.next(), "playlist move", "the entry to move")?,
            to: require_number(args.next(), "playlist move", "where to move it")?,
        },

        other => return Err(format!("unknown playlist command {other:?}")),
    };

    Ok(command)
}

fn require_number(value: Option<String>, verb: &str, expected: &str) -> Result<usize, String> {
    let text = require_value(value, verb, expected)?;
    match text.parse::<usize>() {
        // Counting from one everywhere it is typed, so zero is a mistake rather
        // than the first entry.
        Ok(number) if number > 0 => Ok(number),
        _ => Err(format!("{verb} needs {expected}, not {text:?}")),
    }
}

fn require_value(value: Option<String>, verb: &str, expected: &str) -> Result<String, String> {
    match value {
        Some(value) if !value.trim().is_empty() => Ok(value),
        _ => Err(format!("{verb} needs {expected}")),
    }
}

#[cfg(test)]
mod tests {
    use super::{Command, PlaylistCommand, parse};

    fn parse_args(args: &[&str]) -> Result<Command, String> {
        parse(args.iter().map(|arg| (*arg).to_owned()))
    }

    #[test]
    fn playlist_actions_name_the_playlist_and_number_the_rest() {
        assert_eq!(parse_args(&["playlists"]), Ok(Command::Playlists));

        assert_eq!(
            parse_args(&["playlist", "new", "Late night"]),
            Ok(Command::Playlist(PlaylistCommand::New(
                "Late night".to_owned()
            )))
        );
        assert_eq!(
            parse_args(&["playlist", "add", "Late night", "3"]),
            Ok(Command::Playlist(PlaylistCommand::Add {
                name: "Late night".to_owned(),
                track: 3,
            }))
        );
        assert_eq!(
            parse_args(&["playlist", "move", "Late night", "1", "4"]),
            Ok(Command::Playlist(PlaylistCommand::Move {
                name: "Late night".to_owned(),
                from: 1,
                to: 4,
            }))
        );
    }

    #[test]
    fn a_playlist_delete_needs_saying_twice() {
        assert_eq!(
            parse_args(&["playlist", "delete", "Late night"]),
            Ok(Command::Playlist(PlaylistCommand::Delete {
                name: "Late night".to_owned(),
                confirmed: false,
            })),
            "without --yes the command reports what it would do and stops"
        );
    }

    #[test]
    fn positions_are_counted_from_one_so_zero_is_a_mistake() {
        assert!(parse_args(&["playlist", "add", "Late night", "0"]).is_err());
        assert!(parse_args(&["playlist", "add", "Late night", "three"]).is_err());
        assert!(
            parse_args(&["playlist", "add", "Late night"]).is_err(),
            "and a missing number is not the first track"
        );
    }

    #[test]
    fn no_arguments_opens_the_window() {
        assert_eq!(parse_args(&[]), Ok(Command::Ui));
        assert_eq!(
            parse_args(&["status"]),
            Ok(Command::Status),
            "the old default is still reachable by name"
        );
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
        assert_eq!(
            parse_args(&["crossfade", "on", "4"]),
            Ok(Command::Crossfade {
                enabled: true,
                seconds: Some(4)
            })
        );
        assert_eq!(
            parse_args(&["crossfade", "off"]),
            Ok(Command::Crossfade {
                enabled: false,
                seconds: None
            })
        );
        assert!(parse_args(&["crossfade", "maybe"]).is_err());
        assert!(parse_args(&["crossfade", "on", "soon"]).is_err());

        assert_eq!(parse_args(&["history", "on"]), Ok(Command::History(true)));
        assert_eq!(parse_args(&["history", "off"]), Ok(Command::History(false)));
        assert_eq!(parse_args(&["paths"]), Ok(Command::Paths));
        assert_eq!(
            parse_args(&["play", "C:/music/a.flac"]),
            Ok(Command::Play("C:/music/a.flac".to_owned()))
        );
    }

    #[test]
    fn genres_are_taken_as_written_and_reset_is_recognised() {
        assert_eq!(
            parse_args(&["genre", "3", "Jazz", "Bebop"]),
            Ok(Command::Genre {
                index: 3,
                names: vec!["Jazz".to_owned(), "Bebop".to_owned()],
                reset: false,
            })
        );
        assert_eq!(
            parse_args(&["genre", "3", "--reset"]),
            Ok(Command::Genre {
                index: 3,
                names: Vec::new(),
                reset: true,
            })
        );
        assert_eq!(
            parse_args(&["genre", "3"]),
            Ok(Command::Genre {
                index: 3,
                names: Vec::new(),
                reset: false,
            }),
            "no genres at all is a decision the listener is allowed to make"
        );
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
        assert!(parse_args(&["play"]).is_err());
        assert!(parse_args(&["genre"]).is_err());
        assert!(parse_args(&["genre", "one"]).is_err(), "not a number");
        assert!(
            parse_args(&["genre", "1", "Jazz", "--reset"]).is_err(),
            "resetting and setting at once means nothing"
        );
    }

    #[test]
    fn an_unknown_verb_names_itself_in_the_error() {
        let err = parse_args(&["frobnicate"]).expect_err("not a command");
        assert!(err.contains("frobnicate"), "got {err}");
    }
}
