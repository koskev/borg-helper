use std::{error::Error, path::PathBuf, str::FromStr};

use serde::{Deserialize, Serialize};
use serde_with::{DisplayFromStr, PickFirst};
use void::Void;

use crate::utils::folder::{BackupType, Folder, FolderEntry};

#[derive(Serialize, Deserialize, Debug, Default, Clone)]
pub struct LocalFolder {
    pub(crate) path: PathBuf,
}

impl Folder for LocalFolder {
    fn get_size(&self) -> Result<u64, Box<dyn Error>> {
        Ok(0)
    }

    fn get_path(&self) -> PathBuf {
        self.path.clone()
    }
}

impl FromStr for LocalFolder {
    type Err = Void;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Ok(Self {
            path: PathBuf::from_str(s).unwrap(),
        })
    }
}

impl FromStr for FolderEntry<LocalFolder> {
    type Err = Void;
    fn from_str(value: &str) -> Result<Self, Self::Err> {
        Ok(Self {
            folder: LocalFolder {
                path: PathBuf::from_str(value).unwrap(),
            },
            ..Default::default()
        })
    }
}

#[serde_with::serde_as]
#[derive(Serialize, Deserialize, Debug, Default)]
pub(crate) struct LocalBackup {
    #[serde_as(as = "Vec<PickFirst<(_, DisplayFromStr)>>")]
    pub(crate) folders: Vec<FolderEntry<LocalFolder>>,
}

#[typetag::serde(name = "local")]
impl BackupType for LocalBackup {
    fn pre_backup(&self) -> bool {
        true
    }

    fn post_backup(&self) -> bool {
        true
    }

    fn get_hostname(&self) -> String {
        hostname::get().unwrap().to_str().unwrap().to_string()
    }

    fn get_folders(&self) -> Vec<FolderEntry<Box<dyn Folder>>> {
        let mut v: Vec<FolderEntry<Box<dyn Folder>>> = vec![];
        for f in &self.folders {
            let bf: Box<dyn Folder> = Box::new(f.folder.clone());
            let fe = FolderEntry {
                tags: f.tags.clone(),
                folder: bf,
                options: f.options.clone(),
            };
            v.push(fe);
        }
        v
    }
}
