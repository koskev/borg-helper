use std::{
    error::Error,
    fs,
    os::unix::fs::MetadataExt,
    path::{Path, PathBuf},
    str::FromStr,
};

use serde::{Deserialize, Serialize};
use serde_with::{DisplayFromStr, PickFirst};
use void::Void;

use crate::utils::folder::{BackupType, Folder, FolderEntry};

#[derive(Serialize, Deserialize, Debug, Default, Clone)]
pub struct LocalFolder {
    pub(crate) path: PathBuf,
}

impl LocalFolder {
    pub fn new<P>(path: P) -> Self
    where
        P: AsRef<Path>,
    {
        Self {
            path: PathBuf::from(path.as_ref()),
        }
    }
}

fn get_path_size(path: PathBuf) -> Result<u64, Box<dyn Error>> {
    let mut total_size = 0;
    if path.is_dir() {
        for entry in fs::read_dir(path)? {
            let entry = entry?;
            let file_type = entry.file_type()?;

            if file_type.is_dir() {
                total_size += get_path_size(entry.path()).unwrap_or(0);
            } else {
                total_size += entry.metadata().map(|meta| meta.size()).unwrap_or(0);
            }
        }
    } else {
        total_size += path.metadata().map(|meta| meta.size()).unwrap_or(0);
    }
    Ok(total_size)
}

impl Folder for LocalFolder {
    fn get_size(&self) -> Result<u64, Box<dyn Error>> {
        get_path_size(self.get_path())
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
