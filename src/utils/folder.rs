use std::{error::Error, fmt::Debug, path::PathBuf};

use serde::{Deserialize, Serialize};

#[typetag::serde(tag = "type")]
pub trait BackupType: Debug {
    fn pre_backup(&self) -> bool;
    fn post_backup(&self) -> bool;
    fn get_hostname(&self) -> String;
    // TODO: I don't like this. Just returning a Vec<impl Folder> would be nice
    // Vec<Box<dyn Folder>> won't work as well :/
    fn get_folders(&self) -> Vec<FolderEntry<Box<dyn Folder>>>;
    fn get_additional_options(&self) -> String {
        String::new()
    }
}

pub trait Folder {
    fn get_size(&self) -> Result<u64, Box<dyn Error>>;
    fn get_path(&self) -> PathBuf;
}

impl<F: Folder + ?Sized> Folder for Box<F> {
    fn get_size(&self) -> Result<u64, Box<dyn Error>> {
        (**self).get_size()
    }

    fn get_path(&self) -> PathBuf {
        (**self).get_path()
    }
}

#[serde_with::serde_as]
#[derive(Serialize, Deserialize, Debug, Default, Clone)]
pub struct FolderEntry<T>
where
    T: Folder,
{
    #[serde(default)]
    pub tags: Vec<String>,
    #[serde(flatten)]
    pub folder: T,
}

#[serde_with::serde_as]
#[derive(Serialize, Deserialize, Debug)]
pub struct BackupGroup {
    pub name: String,
    #[serde(default, flatten)]
    pub r#type: Box<dyn BackupType>,
}
