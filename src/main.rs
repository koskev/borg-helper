use std::collections::{HashMap, HashSet};
use std::fs::File;
use std::io::BufReader;
use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};
use std::{env, fs};

use chrono::{DateTime, Local};
use serde::Deserialize;
use serde::Serialize;

type RemoteFolders = HashMap<String, Vec<String>>;

#[derive(Serialize, Deserialize, Debug)]
struct PruneSettings {
    yearly: u32,
    monthly: u32,
    weekly: u32,
    daily: u32,
    hourly: u32,
}

#[derive(Serialize, Deserialize, Debug)]
struct PasswordStore {
    system: String,
    user: String,
}

#[derive(Serialize, Deserialize, Debug)]
struct ConditionalExclude {
    triggers: Vec<String>,
    excludes: Vec<String>,
}

#[derive(Serialize, Deserialize, Debug, Default)]
struct BackupTypeSSH {
    target: String,
    key: Option<String>,
}

impl BackupTypeSSH {
    fn get_hostname(&self) -> String {
        let host = self.target.split_once("@");
        match host {
            Some((_user, hostname)) => hostname.to_string(),
            // Probably an ssh shortcut
            None => self.target.clone(),
        }
    }
}

#[derive(Serialize, Deserialize, Debug, Default)]
enum BackupType {
    #[default]
    LOCAL,
    SSH(BackupTypeSSH),
}

impl BackupType {
    // TODO: this can probably be done with traits? I don't like it. Probably make config and
    // actual object separate?
    fn get_prefix(&self) -> String {
        match self {
            BackupType::LOCAL => hostname::get().unwrap().to_str().unwrap().to_string(),
            BackupType::SSH(ssh) => ssh.get_hostname(),
        }
    }

    fn pre_backup(&self) -> Option<String> {
        match self {
            BackupType::LOCAL => Some("".to_string()),
            BackupType::SSH(ssh) => {
                if ssh.mount() {
                    Some(ssh.get_mount_path())
                } else {
                    None
                }
            }
        }
    }

    fn post_backup(&self) -> bool {
        match self {
            BackupType::LOCAL => true,
            BackupType::SSH(ssh) => ssh.unmount(),
        }
    }

    fn get_additional_options(&self) -> Option<String> {
        match self {
            BackupType::SSH(_ssh) => Some(format!("--files-cache ctime,size")),
            _ => None,
        }
    }
}

#[derive(Serialize, Deserialize, Debug, Default)]
struct FolderEntryDetails {
    #[serde(default)]
    tags: Vec<String>,
    path: String,
}

impl From<String> for FolderEntryDetails {
    fn from(value: String) -> Self {
        Self {
            path: value,
            ..Default::default()
        }
    }
}

#[derive(Serialize, Deserialize, Debug)]
enum FolderEntry {
    Simple(String),
    Details(FolderEntryDetails),
}

impl FolderEntry {
    fn get_path(&self) -> String {
        match self {
            FolderEntry::Simple(e) => e.clone(),
            FolderEntry::Details(e) => e.path.clone(),
        }
    }
}

#[derive(Serialize, Deserialize, Debug)]
struct BackupGroup {
    name: String,
    #[serde(default)]
    r#type: BackupType,
    folders: Vec<FolderEntry>,
}

#[derive(Serialize, Deserialize, Debug)]
struct Borg {
    options: String,

    repositories: Vec<String>,
    backups: Vec<BackupGroup>,

    excludes: Vec<String>,
    conditional_excludes: Vec<ConditionalExclude>,
    password_store: PasswordStore,

    prune_settings: PruneSettings,

    #[serde(skip)]
    date: DateTime<Local>,
}

trait RemoteMount {
    fn mount(&self) -> bool;
    fn unmount(&self) -> bool;
    fn get_mount_path(&self) -> String;
}

impl RemoteMount for BackupTypeSSH {
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

impl Borg {
    fn new(config_path: &str) -> Self {
        let conf_reader = BufReader::new(File::open(config_path).unwrap());
        let mut obj: Borg = serde_yaml::from_reader(conf_reader).unwrap();
        obj.date = Local::now();

        obj
    }

    fn backup_create(&self) {
        for repo in &self.repositories {
            if Borg::is_repo(repo) {
                for backup_source in &self.backups {
                    let mount_point = backup_source.r#type.pre_backup();
                    match mount_point {
                        Some(mount_point) => {
                            let folders: Vec<String> = backup_source
                                .folders
                                .iter()
                                .map(|f| {
                                    PathBuf::from(mount_point.clone())
                                        .join(f.get_path())
                                        .to_str()
                                        .unwrap()
                                        .to_string()
                                })
                                .collect();
                            // Create Backup
                            Borg::_backup_create(
                                &format!(
                                    "{} {}",
                                    backup_source
                                        .r#type
                                        .get_additional_options()
                                        .unwrap_or("".to_string()),
                                    &self.options
                                ),
                                repo,
                                &format!(
                                    "{}-{}",
                                    backup_source.r#type.get_prefix(),
                                    self.date.to_rfc3339()
                                ),
                                &folders,
                                &self.excludes,
                            )
                        }
                        None => (),
                    }
                    backup_source.r#type.post_backup();
                }
            } else {
                println!("Skipping repo {}", repo);
            }
        }
    }

    fn backup_prune(&self) {
        let prefixes: Vec<String> = self.backups.iter().map(|b| b.r#type.get_prefix()).collect();
        prefixes.iter().for_each(|prefix| {
            let cmd = format!("prune --list --stats -v --keep-daily={} --keep-weekly={} --keep-monthly={} --keep-yearly={} --glob-archives '{prefix}*'",
                              self.prune_settings.daily,
                              self.prune_settings.weekly,
                              self.prune_settings.monthly,
                              self.prune_settings.yearly);
            self.run_every_repo(&cmd);
        });
    }

    fn run_every_repo(&self, command: &str) {
        for repo in &self.repositories {
            if Borg::is_repo(repo) {
                let cmd = format!("borg {} {}", command, repo);
                run_cmd_piped(&cmd);
            }
        }
    }

    fn compact(&self) {
        self.run_every_repo("compact");
    }

    fn is_repo(repo: &str) -> bool {
        let p = Path::new(repo);
        if p.exists() {
            let cmd = format!("borg info {repo}");
            let output = run_cmd(&cmd);
            return output.status.success();
        }
        false
    }

    // TODO: make filter a parameter
    fn _backup_create(
        options: &str,
        repo: &str,
        name: &str,
        folders: &Vec<String>,
        excludes: &Vec<String>,
    ) {
        let folder_list_str = folders.join(" ");
        let mut local_excludes = excludes.clone();
        //let mut target_paths = Vec::new();
        //let mut dirs_to_check = folders.clone();
        //let mut visited_dirs = HashSet::new();
        //while let Some(dir) = dirs_to_check.pop() {
        //    if let Ok(dir_contents) = fs::read_dir(dir) {
        //        for subentry in dir_contents {
        //            let subentry = subentry.unwrap();
        //            let subpath = subentry.path();
        //            let real_path = fs::read_link(&subpath).unwrap_or_else(|_| subpath.clone());
        //            if real_path.is_dir() && !visited_dirs.insert(real_path) {
        //                println!("Already visited {}", subpath.display());
        //                continue;
        //            }

        //            if subpath.is_dir() && !subpath.is_symlink() {
        //                dirs_to_check.push(subpath.to_string_lossy().to_string());
        //            } else if subpath.ends_with("Cargo.toml") {
        //                let target_path =
        //                    Path::new(&subpath).parent().unwrap().join("target").clone();
        //                if target_path.exists() {
        //                    println!("#### ignoring {target_path:?}");
        //                    target_paths.push(target_path);
        //                }
        //            }
        //        }
        //    }
        //}

        //local_excludes.extend(
        //    target_paths
        //        .into_iter()
        //        .map(|p| p.to_string_lossy().to_string()),
        //);

        let folder_exclude_str: String = local_excludes
            .into_iter()
            .map(|val| format!(" --exclude {val}"))
            .collect();

        let cmd = format!("borg create {options} {repo}::{name} {folder_list_str} {folder_exclude_str} --exclude-if-present .nobackup --exclude-if-present CACHEDIR.TAG");
        let _res = run_cmd_piped(&cmd);
    }
}

fn run_cmd_piped(cmd: &str) -> Output {
    println!("Calling piped \"{}\"", cmd);
    let output = Command::new("sh")
        .arg("-c")
        .arg(cmd)
        .stdout(Stdio::inherit())
        .stderr(Stdio::inherit())
        .output()
        .expect("failed to execute process");

    output
}

fn run_cmd(cmd: &str) -> Output {
    println!("Calling \"{}\"", cmd);
    let output = Command::new("sh")
        .arg("-c")
        .arg(cmd)
        .output()
        .expect("failed to execute process");

    output
}

fn get_password(service: &str, user: &str) -> Option<String> {
    let cmd = format!("secret-tool lookup {service} {user}");
    let output = run_cmd(&cmd);
    if output.status.success() {
        return Some(std::str::from_utf8(&output.stdout).unwrap().to_string());
    }
    None
}

fn main() {
    let borg = Borg::new("config.yaml");
    println!("{:?}", borg);
    let pw = get_password(&borg.password_store.system, &borg.password_store.user).unwrap();
    env::set_var("BORG_PASSPHRASE", pw);
    borg.backup_create();
    borg.backup_prune();
}
