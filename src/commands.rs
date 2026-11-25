#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn search_matches_whole_prefix_and_keeps_the_query() {
        let commands = Commands::default();
        assert_eq!(
            commands
                .resolve(" Search for C++ tips in 2026? ")
                .unwrap()
                .argument,
            "C++ tips in 2026?"
        );
        assert!(commands.resolve("search for").is_none());
        assert!(commands.resolve("search forecast").is_none());
    }

    #[test]
    fn rejects_duplicate_phrases_after_normalization() {
        let mut commands = Commands::default();
        let duplicate = Command {
            phrase: "  OPEN   CHROME ".into(),
            action: Action::MediaToggle,
        };
        assert!(commands.set(None, duplicate).is_err());
    }

    #[test]
    fn command_changes_survive_restart() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("commands.json");
        let mut commands = Commands::load(&file).unwrap();
        commands
            .set(
                None,
                Command {
                    phrase: "open my notes".into(),
                    action: Action::OpenWebsite("https://example.com/notes".into()),
                },
            )
            .unwrap();
        commands.save(&file).unwrap();
        let reloaded = Commands::load(&file).unwrap();
        assert_eq!(
            reloaded.resolve("Open my notes.").unwrap().command.action,
            Action::OpenWebsite("https://example.com/notes".into())
        );
    }
}
use std::{
    fs,
    io::{self, Write},
    path::Path,
};

use anyhow::{Context, Result, ensure};
use serde::{Deserialize, Serialize};
use url::Url;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", content = "target", rename_all = "snake_case")]
pub enum Action {
    OpenChrome,
    OpenApp(String),
    OpenWebsite(String),
    MediaToggle,
    SwitchApps,
    SearchChrome,
}

impl Action {
    pub fn label(&self) -> &'static str {
        match self {
            Self::OpenChrome | Self::OpenApp(_) => "Open app",
            Self::OpenWebsite(_) => "Open website",
            Self::MediaToggle => "Media play/pause",
            Self::SwitchApps => "Switch apps",
            Self::SearchChrome => "Search in Chrome",
        }
    }

    fn validate(&self) -> Result<()> {
        match self {
            Self::OpenApp(target) => {
                let path = Path::new(target);
                ensure!(
                    path.is_absolute()
                        && path
                            .extension()
                            .is_some_and(|ext| ext.eq_ignore_ascii_case("exe"))
                        && !target.contains('\0'),
                    "Choose an app's .exe file."
                );
            }
            Self::OpenWebsite(target) => {
                let url = Url::parse(target).context("Enter a full website URL.")?;
                ensure!(
                    matches!(url.scheme(), "http" | "https")
                        && url.host_str().is_some()
                        && url.username().is_empty()
                        && url.password().is_none(),
                    "Enter an HTTP or HTTPS website URL without sign-in details."
                );
            }
            _ => {}
        }
        Ok(())
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Command {
    pub phrase: String,
    pub action: Action,
}

#[derive(Clone, Serialize, Deserialize)]
pub struct Commands {
    items: Vec<Command>,
}

pub struct Match<'a> {
    pub command: &'a Command,
    pub argument: String,
}

impl Default for Commands {
    fn default() -> Self {
        Self {
            items: [
                ("Open Chrome", Action::OpenChrome),
                (
                    "Open ChatGPT",
                    Action::OpenWebsite("https://chatgpt.com/".into()),
                ),
                ("Play", Action::MediaToggle),
                ("Pause", Action::MediaToggle),
                ("Search for", Action::SearchChrome),
                ("Switch apps", Action::SwitchApps),
                ("Play or pause", Action::MediaToggle),
            ]
            .into_iter()
            .map(|(phrase, action)| Command {
                phrase: phrase.into(),
                action,
            })
            .collect(),
        }
    }
}

impl Commands {
    pub fn empty() -> Self {
        Self { items: Vec::new() }
    }

    pub fn items(&self) -> &[Command] {
        &self.items
    }

    pub fn set(&mut self, index: Option<usize>, mut command: Command) -> Result<()> {
        command.phrase = command
            .phrase
            .split_whitespace()
            .collect::<Vec<_>>()
            .join(" ");
        let phrase = normalize(&command.phrase);
        ensure!(!phrase.is_empty(), "Enter the phrase you want to say.");
        ensure!(phrase != "hey wiki", "“Hey Wiki” is the wake phrase.");
        ensure!(
            !self
                .items
                .iter()
                .enumerate()
                .any(|(i, item)| { Some(i) != index && normalize(&item.phrase) == phrase }),
            "That phrase is already assigned."
        );
        command.action.validate()?;
        if let Some(index) = index {
            self.items[index] = command;
        } else {
            self.items.push(command);
        }
        Ok(())
    }

    pub fn remove(&mut self, index: usize) {
        self.items.remove(index);
    }

    pub fn resolve(&self, spoken: &str) -> Option<Match<'_>> {
        let spoken_phrase = normalize(spoken);
        if let Some(command) = self.items.iter().find(|command| {
            command.action != Action::SearchChrome && normalize(&command.phrase) == spoken_phrase
        }) {
            return Some(Match {
                command,
                argument: String::new(),
            });
        }

        let words: Vec<_> = spoken.split_whitespace().collect();
        self.items
            .iter()
            .filter(|command| command.action == Action::SearchChrome)
            .filter_map(|command| {
                let prefix = normalize(&command.phrase);
                let length = prefix.split_whitespace().count();
                let query = words.get(length..)?;
                if query.is_empty() || normalize(&words[..length].join(" ")) != prefix {
                    return None;
                }
                Some((
                    length,
                    Match {
                        command,
                        argument: query.join(" "),
                    },
                ))
            })
            .max_by_key(|(length, _)| *length)
            .map(|(_, command)| command)
    }

    pub fn fixed_speech_phrases(&self) -> impl Iterator<Item = String> + '_ {
        self.items
            .iter()
            .filter(|command| command.action != Action::SearchChrome)
            .map(|command| match normalize(&command.phrase).as_str() {
                "open chatgpt" => "open chat g p t".into(),
                phrase => phrase.into(),
            })
    }

    pub fn load(path: &Path) -> Result<Self> {
        let bytes = match fs::read(path) {
            Ok(bytes) => bytes,
            Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(Self::default()),
            Err(error) => return Err(error).context("Could not read commands.json."),
        };
        let saved: Self =
            serde_json::from_slice(&bytes).context("Could not read commands.json.")?;
        let mut commands = Self::empty();
        for command in saved.items {
            commands
                .set(None, command)
                .context("Invalid command in commands.json.")?;
        }
        Ok(commands)
    }

    pub fn save(&self, path: &Path) -> Result<()> {
        let parent = path
            .parent()
            .context("The command file needs a parent folder.")?;
        fs::create_dir_all(parent)?;
        let mut file = tempfile::NamedTempFile::new_in(parent)?;
        serde_json::to_writer_pretty(&mut file, self)?;
        file.write_all(b"\n")?;
        file.as_file().sync_all()?;
        file.persist(path)?;
        Ok(())
    }
}

pub(crate) fn normalize(text: &str) -> String {
    let phrase = text
        .split_whitespace()
        .map(|word| word.trim_matches(['.', ',', '!', '?', ':', ';', '"', '“', '”']))
        .filter(|word| !word.is_empty())
        .collect::<Vec<_>>()
        .join(" ")
        .to_lowercase();
    match phrase.as_str() {
        "open chat gpt" | "open chat g p t" => "open chatgpt".into(),
        _ => phrase,
    }
}
