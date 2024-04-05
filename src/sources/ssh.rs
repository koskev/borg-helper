use std::{error::Error, path::PathBuf, str::FromStr};

use log::info;
use serde::{Deserialize, Serialize};
use serde_with::{serde_as, DisplayFromStr, PickFirst};
use void::Void;

use crate::{
    run_cmd,
    utils::{
        folder::{BackupType, Folder, FolderEntry},
        mountable::Mountable,
    },
};

#[serde_with::serde_as]
#[derive(Serialize, Deserialize, Debug, Default)]
pub struct SSHBackup {
    pub target: String,
    #[serde_as(as = "Vec<PickFirst<(_, DisplayFromStr)>>")]
    pub folders: Vec<FolderEntry<SSHFolder>>,
}

impl Mountable for SSHBackup {
    fn mount(&self) -> bool {
        // TODO: use key
        let temp_dir = self.get_mount_path();
        std::fs::create_dir_all(&temp_dir).unwrap_or_default();
        let cmd = format!("sshfs {}:/ {temp_dir}", self.target);
        let output = run_cmd(&cmd);
        output.status.success()
    }

    fn unmount(&self) -> bool {
        let cmd = format!("fusermount -u {}", self.get_mount_path());
        let output = run_cmd(&cmd);
        output.status.success()
    }

    fn get_mount_path(&self) -> String {
        format!("/tmp/backup/{}", self.target)
    }
}

#[typetag::serde(name = "ssh")]
impl BackupType for SSHBackup {
    fn pre_backup(&self) -> bool {
        self.mount()
    }

    fn post_backup(&self) -> bool {
        self.unmount()
    }

    fn get_hostname(&self) -> String {
        let host = self.target.split_once('@');
        match host {
            Some((_user, hostname)) => hostname.to_string(),
            // Probably an ssh shortcut
            None => self.target.clone(),
        }
    }

    fn get_additional_options(&self) -> String {
        String::from("--files-cache ctime,size")
    }

    fn get_folders(&self) -> Vec<FolderEntry<Box<dyn Folder>>> {
        info!("Getting folders");
        let mut v: Vec<FolderEntry<Box<dyn Folder>>> = vec![];
        for f in &self.folders {
            let mut folder = f.folder.clone();
            folder.prefix = PathBuf::from(self.get_mount_path());
            folder.target = self.target.clone();
            let dyn_folder: Box<dyn Folder> = Box::new(folder);
            let fe = FolderEntry {
                tags: f.tags.clone(),
                folder: dyn_folder,
            };
            v.push(fe);
        }
        v
    }
}

#[derive(Serialize, Deserialize, Debug, Default, Clone)]
pub struct SSHFolder {
    pub path: PathBuf,
    #[serde(skip)]
    pub prefix: PathBuf,
    #[serde(skip)]
    pub target: String,
}

impl Folder for SSHFolder {
    fn get_size(&self) -> Result<u64, Box<dyn Error>> {
        // SSH to target and call "du <folder>". It is way faster than using the mounted fs
        let remote_cmd = format!(
            "du -s {} 2>/dev/null | cut -f1",
            self.path.to_str().unwrap_or_default()
        );

        let cmd = format!("ssh {} {}", self.target, remote_cmd);

        let output = run_cmd(&cmd);
        if output.status.success() {
            let output_str: String = std::str::from_utf8(&output.stdout)
                .unwrap()
                .chars()
                .filter(|c| !c.is_whitespace())
                .collect();
            info!("{}", output_str);

            let val = output_str.parse::<u64>().unwrap_or(0);
            return Ok(val);
        }
        Ok(0)
    }

    fn get_path(&self) -> PathBuf {
        // Strip leading / if it exists
        let strip = self.path.strip_prefix("/").unwrap_or(&self.path);
        self.prefix.join(strip)
    }
}

impl FromStr for SSHFolder {
    type Err = Void;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Ok(Self {
            path: PathBuf::from_str(s).unwrap(),
            ..Default::default()
        })
    }
}

impl FromStr for FolderEntry<SSHFolder> {
    type Err = Void;
    fn from_str(value: &str) -> Result<Self, Self::Err> {
        Ok(Self {
            folder: SSHFolder {
                path: PathBuf::from_str(value).unwrap(),
                ..Default::default()
            },
            ..Default::default()
        })
    }
}
