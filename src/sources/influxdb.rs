use std::{
    cell::RefCell,
    fs::{self},
    process::Child,
};

use log::{error, info};
use secstr::SecUtf8;
use serde::{Deserialize, Serialize};

use crate::sources::local::LocalFolder;
use crate::utils::{
    cmd::run_cmd,
    folder::{BackupType, Folder, FolderEntry},
    k8s::start_k8s_proxy,
    mountable::Mountable,
};

#[derive(Serialize, Deserialize, Debug)]
struct InfluxdbBackup {
    token: SecUtf8,
    port: Option<u16>,
    host: Option<String>,
    k8s_deployment: Option<String>,
    k8s_namespace: Option<String>,
    #[serde(default)]
    tags: Vec<String>,

    #[serde(skip)]
    proxy_process: RefCell<Option<Child>>,
}

impl Mountable for InfluxdbBackup {
    fn mount(&self) -> bool {
        // If host is not set we assume localhost (or k8s)
        let host = self.host.clone().unwrap_or("http://127.0.0.1".to_string());
        let port = self.port.unwrap_or(8086);
        // Create proxy connection
        match &self.k8s_deployment {
            Some(deployment) => {
                let namespace = self.k8s_namespace.clone().unwrap_or("default".to_string());
                *self.proxy_process.borrow_mut() =
                    start_k8s_proxy(&namespace, &deployment, port, port)
            }
            None => (),
        }
        let cmd = format!(
            "influx backup --host {}:{} --token {} {}",
            host,
            port,
            self.token.clone().into_unsecure(),
            self.get_mount_path()
        );
        let output = run_cmd(&cmd);
        if !output.status.success() {
            error!(
                "Failed to dump database: {}",
                String::from_utf8_lossy(&output.stderr)
            );
            return false;
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
        let name = self.get_hostname();
        format!("/tmp/backup/{}", name)
    }
}

#[typetag::serde(name = "influxdb")]
impl BackupType for InfluxdbBackup {
    fn pre_backup(&self) -> bool {
        self.mount()
    }

    fn post_backup(&self) -> bool {
        self.unmount()
    }

    fn get_hostname(&self) -> String {
        self.host.clone().unwrap_or(
            self.k8s_deployment
                .clone()
                .unwrap_or("psql_back".to_string()),
        )
    }

    fn get_folders(&self) -> Vec<FolderEntry<Box<dyn Folder>>> {
        info!("Getting folders");
        let mut v: Vec<FolderEntry<Box<dyn Folder>>> = vec![];
        let dyn_folder: Box<dyn Folder> = Box::new(LocalFolder::new(&self.get_mount_path()));
        let fe = FolderEntry {
            tags: self.tags.clone(),
            folder: dyn_folder,
            options: None,
        };
        v.push(fe);
        v
    }
}
