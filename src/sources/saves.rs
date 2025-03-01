use std::{collections::HashMap, str::FromStr};

use serde::{Deserialize, Serialize};

use crate::utils::{
    cmd::run_cmd_checked,
    folder::{BackupType, Folder, FolderEntry},
};

use super::local::LocalFolder;

#[derive(Debug, Default, Deserialize)]
struct ApiFile {}

#[derive(Debug, Deserialize)]
#[serde(untagged)]
enum ApiGame {
    Operative { files: HashMap<String, ApiFile> },
    Stored {},
    Found {},
}

#[derive(Debug, Default, Deserialize)]
pub struct JsonOutput {
    games: HashMap<String, ApiGame>,
}

#[derive(Serialize, Deserialize, Debug, Default)]
struct GameSettings {
    name: String,
    #[serde(default)]
    tags: Vec<String>,
}

#[serde_with::serde_as]
#[derive(Serialize, Deserialize, Debug, Default)]
struct SaveBackup {
    #[serde(default)]
    pub games: Vec<GameSettings>,
    #[serde(default)]
    tags: Vec<String>,

    /// Custom path to ludusavi binary
    binary: Option<String>,
}

#[typetag::serde(name = "saves")]
impl BackupType for SaveBackup {
    fn pre_backup(&self) -> bool {
        true
    }

    fn post_backup(&self) -> bool {
        true
    }

    fn get_folders(&self) -> Vec<FolderEntry<Box<dyn Folder>>> {
        let binary = self.binary.clone().unwrap_or("ludusavi".to_string());

        let output = run_cmd_checked(&format!("{} backup --preview --api", binary)).unwrap();
        let output_str = String::from_utf8(output.stdout).unwrap_or_default();

        let json_data: JsonOutput = serde_yaml::from_str(&output_str).unwrap();

        let files: Vec<FolderEntry<Box<dyn Folder>>> = json_data
            .games
            .iter()
            .filter_map(|(name, data)| match data {
                ApiGame::Operative { files } => {
                    let mut entries = vec![];
                    let paths = files.keys().cloned().collect::<Vec<String>>();
                    let mut tags = self.tags.clone();
                    // search for custom game options
                    // TODO: allow for regex/wildcard
                    if let Some(game_settings) = self.games.iter().find(|f| f.name == *name) {
                        // we got an entry for the specified game
                        tags.append(&mut game_settings.tags.clone());
                    }

                    for path in paths {
                        let bf: Box<dyn Folder> = Box::new(LocalFolder::from_str(&path).unwrap());
                        let fe = FolderEntry {
                            tags: tags.clone(),
                            folder: bf,
                            options: None,
                        };
                        entries.push(fe);
                    }
                    Some(entries)
                }
                _ => None,
            })
            .flatten()
            .collect();
        files
    }
}

#[cfg(test)]
mod test {
    use more_asserts::assert_ge;

    use super::SaveBackup;
    use crate::{sources::saves::GameSettings, utils::folder::BackupType};

    #[test]
    fn test_ludusavi() {
        let back = SaveBackup::default();
        let folders = back.get_folders();
        assert_ge!(folders.len(), 1);
    }

    #[test]
    // FIXME: Requires stardew valley for now
    fn test_tags() {
        let back = SaveBackup {
            games: vec![GameSettings {
                name: "Stardew Valley".to_string(),
                tags: vec!["local_tag".to_string()],
            }],
            tags: vec!["global_tag".to_string()],
            binary: None,
        };
        let folders = back.get_folders();
        assert_ge!(folders.len(), 1);

        assert!(folders
            .iter()
            .all(|entry| entry.tags.contains(&"global_tag".to_string())));
        assert!(folders.iter().any(|entry| entry
            .folder
            .get_path()
            .to_str()
            .unwrap()
            .contains("StardewValley")
            && entry.tags.contains(&"local_tag".to_string())));
    }
}
