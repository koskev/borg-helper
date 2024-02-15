use std::{
    cell::RefCell,
    error::Error,
    fs::{self, File},
    io::Write,
    path::PathBuf,
    process::Child,
};

use netstat2::{get_sockets_info, AddressFamilyFlags, ProtocolFlags, ProtocolSocketInfo, TcpState};
use serde::{Deserialize, Serialize};

use crate::{
    run_cmd, run_cmd_background,
    utils::{
        folder::{BackupType, Folder, FolderEntry},
        mountable::Mountable,
    },
};

#[derive(Serialize, Deserialize, Debug, Default)]
struct PsqlBackup {
    user: String,
    password: String,
    port: u16,
    host: Option<String>,
    k8s_deployment: Option<String>,
    #[serde(default)]
    tags: VeMountablec<String>,

    #[serde(skip)]
    proxy_process: RefCell<Option<Child>>,
}

fn is_port_listening(port: u16) -> bool {
    let af_flags = AddressFamilyFlags::IPV4 | AddressFamilyFlags::IPV6;
    let proto_flags = ProtocolFlags::TCP;
    let sockets_info = get_sockets_info(af_flags, proto_flags);
    println!("Checking if port {port} is listening");
    match sockets_info {
        Ok(sockets_info) => {
            let sockets = sockets_info.iter().find(|s| match &s.protocol_socket_info {
                ProtocolSocketInfo::Tcp(tcp) => {
                    tcp.state == TcpState::Listen && tcp.local_port == port
                }
                _ => false,
            });
            sockets.is_some()
        }
        Err(_) => false,
    }
}

impl Mountable for PsqlBackup {
    fn mount(&self) -> bool {
        // If host is not set we assume localhost (or k8s)
        let host = self.host.clone().unwrap_or("127.0.0.1".to_string());
        // Create proxy connection
        match &self.k8s_deployment {
            Some(deployment) => {
                println!("Starting proxy...");
                let cmd = format!(
                    "kubectl port-forward {} {}:{}",
                    deployment, self.port, self.port
                );
                let child = run_cmd_background(&cmd);
                match child {
                    Ok(mut child) => {
                        // Wait for proxy to run
                        while !is_port_listening(self.port) {
                            // Check if child returned or threw an error. If not -> Program is
                            // still running and we can wait for the port
                            let child_ret = child.try_wait();
                            match child_ret {
                                Ok(ret) => {
                                    if ret.is_some() {
                                        return false;
                                    }
                                }
                                Err(_) => return false,
                            }
                        }
                        *self.proxy_process.borrow_mut() = Some(child)
                    }
                    Err(e) => {
                        println!("Failed to create k8s proxy: {}", e);
                        return false;
                    }
                }
            }
            None => (),
        }
        let cmd = format!(
            "pg_dumpall --dbname=postgresql://{}:{}@{}:{}",
            self.user, self.password, host, self.port
        );
        let output = run_cmd(&cmd);
        if !output.status.success() {
            println!(
                "Failed to dump database: {}",
                String::from_utf8_lossy(&output.stderr)
            );
            return false;
        }
        let output_file = File::create(self.get_mount_path());
        match output_file {
            Ok(mut f) => {
                f.write(&output.stdout).unwrap();
            }
            Err(e) => {
                println!("Failed to open {}: {}", self.get_mount_path(), e);
                return false;
            }
        }
        output.status.success()
    }

    fn unmount(&self) -> bool {
        println!("Unmounting psql");
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

#[typetag::serde(name = "psql")]
impl BackupType for PsqlBackup {
    fn pre_backup(&self) -> bool {
        self.mount()
    }

    fn post_backup(&self) -> bool {
        self.unmount()
    }

    fn get_hostname(&self) -> String {
        let name = self.host.clone().unwrap_or(
            self.k8s_deployment
                .clone()
                .unwrap_or("psql_back".to_string()),
        );
        name
    }

    fn get_folders(&self) -> Vec<FolderEntry<Box<dyn Folder>>> {
        println!("Getting folders");
        let mut v: Vec<FolderEntry<Box<dyn Folder>>> = vec![];
        let dyn_folder: Box<dyn Folder> = Box::new(PsqlFolder::new(&self.get_mount_path()));
        let fe = FolderEntry {
            tags: self.tags.clone(),
            folder: dyn_folder,
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
