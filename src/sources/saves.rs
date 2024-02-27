use std::{collections::HashMap, error::Error, str::FromStr};

use serde::{Deserialize, Serialize};

use crate::utils::{
    cmd::run_cmd_checked,
    folder::{BackupType, Folder, FolderEntry},
};

use super::local::LocalFolder;

#[derive(Debug, Default, Deserialize)]
struct ApiFile {
    bytes: u64,
}

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

#[serde_with::serde_as]
#[derive(Serialize, Deserialize, Debug, Default)]
struct SaveBackup {}

#[typetag::serde(name = "saves")]
impl BackupType for SaveBackup {
    fn pre_backup(&self) -> bool {
        true
    }

    fn post_backup(&self) -> bool {
        true
    }

    fn get_hostname(&self) -> String {
        format!(
            "{}-games",
            hostname::get().unwrap().to_str().unwrap().to_string()
        )
    }

    fn get_folders(&self) -> Vec<FolderEntry<Box<dyn Folder>>> {
        // Call ludusavi
        let output = run_cmd_checked("ludusavi backup --preview --api").unwrap();
        let output_str = String::from_utf8(output.stdout).unwrap_or_default();

        let json_data: JsonOutput = serde_yaml::from_str(&output_str).unwrap();

        let files: Vec<FolderEntry<Box<dyn Folder>>> = json_data
            .games
            .iter()
            .filter_map(|(_name, data)| match data {
                ApiGame::Operative { files } => {
                    Some(files.keys().cloned().collect::<Vec<String>>())
                }
                _ => None,
            })
            .flatten()
            .map(|path| {
                let bf: Box<dyn Folder> = Box::new(LocalFolder::from_str(&path).unwrap());
                let fe = FolderEntry {
                    tags: vec![],
                    folder: bf,
                };
                fe
            })
            .collect();
        files
    }
}

#[cfg(test)]
mod test {
    use more_asserts::assert_ge;

    use super::SaveBackup;
    use crate::utils::folder::BackupType;

    #[test]
    fn test_ludusavi() {
        let back = SaveBackup {};
        let folders = back.get_folders();
        assert_ge!(folders.len(), 1);
    }
}
