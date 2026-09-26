//! What a program people run on their own machine does with the text it
//! shows, sent beside the file's path and language like the web framework
//! roles: a game client that hands the server's error text to its own
//! window over a channel of `Response` messages read as a server answering
//! a remote client, in fifteen reviews for sending internal error details.
//! Facts come from the package's dependencies on a desktop, game or
//! terminal interface toolkit.
use crate::packages::Package;

const CLIENT: &str = "Client application: this package is a desktop, game or terminal program that runs on its user's own machine, so the errors and messages it shows or passes to its own screens go to that user, not in a response to a remote client.";

/// Dependencies that make a package a program people run on their own machine.
const INTERFACES: [&str; 16] = [
    "ratatui",
    "cursive",
    "egui",
    "eframe",
    "iced",
    "bevy",
    "macroquad",
    "ggez",
    "slint",
    "druid",
    "fltk",
    "gtk4",
    "relm4",
    "tauri",
    "spacetimedb-sdk",
    "electron",
];

/// The client facts of a file whose package depends on an interface toolkit.
pub(super) fn describe(package: Option<&Package>) -> Option<&'static str> {
    package
        .filter(|p| INTERFACES.iter().any(|d| p.dependencies.contains(*d)))
        .map(|_| CLIENT)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn package(dependencies: &[&str]) -> Package {
        Package {
            dir: PathBuf::from("client"),
            name: None,
            dependencies: dependencies.iter().map(|d| d.to_string()).collect(),
        }
    }

    #[test]
    fn packages_with_an_interface_toolkit_are_client_applications() {
        assert_eq!(
            describe(Some(&package(&["ratatui", "spacetimedb-sdk"]))),
            Some(CLIENT)
        );
        assert!(describe(Some(&package(&["tauri", "serde"]))).is_some());
        assert!(describe(Some(&package(&["axum", "tokio"]))).is_none());
        assert!(describe(None).is_none());
    }
}
