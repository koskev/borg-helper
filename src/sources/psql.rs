use std::{
    cell::RefCell,
    error::Error,
    fs::{self, File},
    io::Write,
    path::PathBuf,
    process::Child,
};

use log::{error, info};
use secstr::SecUtf8;
use serde::{Deserialize, Serialize};

use crate::utils::{
    cmd::run_cmd,
    folder::{BackupType, Folder, FolderEntry},
    k8s::start_k8s_proxy,
    mountable::Mountable,
};

#[derive(Serialize, Deserialize, Debug)]
struct PsqlBackup {
    user: String,
    password: SecUtf8,
    port: u16,
    host: Option<String>,
    k8s_deployment: Option<String>,
    #[serde(default)]
    tags: Vec<String>,

    #[serde(skip)]
    proxy_process: RefCell<Option<Child>>,
}

impl Mountable for PsqlBackup {
    fn mount(&self) -> bool {
        // If host is not set we assume localhost (or k8s)
        let host = self.host.clone().unwrap_or("127.0.0.1".to_string());
        // Create proxy connection
        match &self.k8s_deployment {
            Some(deployment) => {
                *self.proxy_process.borrow_mut() =
                    start_k8s_proxy("default", &deployment, self.port, self.port)
            }
            None => (),
        }
        let cmd = format!(
            "pg_dumpall --dbname=postgresql://{}:{}@{}:{}",
            self.user, self.password, host, self.port
        );
        let output = run_cmd(&cmd);
        if !output.status.success() {
            error!(
                "Failed to dump database: {}",
                String::from_utf8_lossy(&output.stderr)
            );
            return false;
        }
        let output_file = File::create(self.get_mount_path());
        match output_file {
            Ok(mut f) => {
                f.write_all(&output.stdout).unwrap();
            }
            Err(e) => {
                error!("Failed to open {}: {}", self.get_mount_path(), e);
                return false;
            }
        }
        output.status.success()
    }

    fn unmount(&self) -> bool {
        info!("Unmounting psql");
        if let Some(ref mut child) = *self.proxy_process.borrow_mut() {
            child.kill().unwrap();
        }
        let res = fs::remove_file(self.get_mount_path());
        res.is_ok()
    }

    fn get_mount_path(&self) -> String {
        format!(
            "/tmp/backup/postgresql-{}",
            self.host.clone().unwrap_or("nohost".into())
        )
    }
}

#[typetag::serde(name = "psql")]
impl BackupType for PsqlBackup {
    fn pre_backup(&self) -> bool {
        self.mount()
    }

    fn post_backup(&self) -> bool {
        self.unmount()
    }

    fn get_folders(&self) -> Vec<FolderEntry<Box<dyn Folder>>> {
        info!("Getting folders");
        let mut v: Vec<FolderEntry<Box<dyn Folder>>> = vec![];
        let dyn_folder: Box<dyn Folder> = Box::new(PsqlFolder::new(&self.get_mount_path()));
        let fe = FolderEntry {
            tags: self.tags.clone(),
            folder: dyn_folder,
            options: None,
        };
        v.push(fe);
        v
    }
}

struct PsqlFolder {
    path: String,
}

impl PsqlFolder {
    pub fn new(path: &str) -> Self {
        Self {
            path: path.to_string(),
        }
    }
}

impl Folder for PsqlFolder {
    fn get_size(&self) -> Result<u64, Box<dyn Error>> {
        Ok(0)
    }

    fn get_path(&self) -> PathBuf {
        PathBuf::from(self.path.clone())
    }
}
