use std::{error::Error, path::PathBuf, str::FromStr};

use serde::{Deserialize, Serialize};
use serde_with::{serde_as, DisplayFromStr, PickFirst};
use void::Void;

use crate::utils::folder::{BackupType, Folder, FolderEntry};

#[serde_with::serde_as]
#[derive(Serialize, Deserialize, Debug, Default)]
struct SSHBackup {
    target: String,
    #[serde_as(as = "Vec<PickFirst<(_, DisplayFromStr)>>")]
    folders: Vec<FolderEntry<SSHFolder>>,
}

#[typetag::serde(name = "ssh")]
impl BackupType for SSHBackup {
    fn pre_backup(&self) -> bool {
        // TODO: mount
        true
    }

    fn post_backup(&self) -> bool {
        // TODO: unmount
        true
    }

    fn get_hostname(&self) -> String {
        hostname::get().unwrap().to_str().unwrap().to_string()
    }

    fn get_additional_options(&self) -> String {
        String::from("--files-cache ctime,size")
    }

    fn get_folders(&self) -> Vec<FolderEntry<Box<dyn Folder>>> {
        let mut v: Vec<FolderEntry<Box<dyn Folder>>> = vec![];
        for f in &self.folders {
            let bf: Box<dyn Folder> = Box::new(f.folder.clone());
            let fe = FolderEntry {
                tags: f.tags.clone(),
                folder: bf,
            };
            v.push(fe);
        }
        v
    }
}

#[derive(Serialize, Deserialize, Debug, Default, Clone)]
struct SSHFolder {
    path: PathBuf,
}

impl Folder for SSHFolder {
    fn get_size(&self) -> Result<u64, Box<dyn Error>> {
        Ok(0)
    }

    fn get_path(&self) -> PathBuf {
        PathBuf::new()
    }
}

impl FromStr for SSHFolder {
    type Err = Void;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Ok(Self {
            path: PathBuf::from_str(s).unwrap(),
        })
    }
}

impl FromStr for FolderEntry<SSHFolder> {
    type Err = Void;
    fn from_str(value: &str) -> Result<Self, Self::Err> {
        Ok(Self {
            folder: SSHFolder {
                path: PathBuf::from_str(value).unwrap(),
            },
            ..Default::default()
        })
    }
}
