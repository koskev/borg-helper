use std::collections::{HashMap, HashSet};
use std::fs::File;
use std::io::BufReader;
use std::path::Path;
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
struct Borg {
    options: String,

    repositories: Vec<String>,
    backup_folders: Vec<String>,

    remote_folders: RemoteFolders,
    excludes: Vec<String>,
    password_store: PasswordStore,

    prune_settings: PruneSettings,

    #[serde(skip)]
    date: DateTime<Local>,
}

impl Borg {
    fn new(config_path: &str) -> Self {
        let conf_reader = BufReader::new(File::open(config_path).unwrap());
        let mut obj: Borg = serde_yaml::from_reader(conf_reader).unwrap();
        obj.date = Local::now();

        obj
    }

    fn get_local_prefix(&self) -> String {
        let hostname_os = hostname::get().unwrap();
        hostname_os.to_str().unwrap().to_string()
    }

    fn get_remote_prefixes(&self) -> Vec<String> {
        let hostnames = self
            .remote_folders
            .keys()
            .map(|host| {
                let (_user, hostname) = host.split_once('@').unwrap();
                hostname.to_string()
            })
            .collect();

        hostnames
    }

    fn backup_create(&self) {
        // Test repos
        for repo in &self.repositories {
            if Borg::is_repo(repo) {
                Borg::_backup_create(
                    &self.options,
                    repo,
                    &format!("{}-{}", self.get_local_prefix(), self.date.to_rfc3339()),
                    &self.backup_folders,
                    &self.excludes,
                )
            } else {
                println!("Skipping repo {}", repo);
            }
        }
    }

    fn backup_create_remote(&self) {
        for repo in &self.repositories {
            if Borg::is_repo(repo) {
                Borg::_backup_create_remote(
                    &self.options,
                    repo,
                    &self.date.to_rfc3339(),
                    &self.remote_folders,
                    &self.excludes,
                );
            }
        }
    }
    //    def backup_prune(repo: str, keep_daily: int = 7, keep_weekly:int = 4, keep_monthly: int = 6, keep_yearly: int = 0, prefix = ""):
    //    cmd = "borg prune --list --stats -v {} --keep-daily={} --keep-weekly={} --keep-monthly={} --keep-yearly={} --prefix \"{}\"".format(repo, keep_daily, keep_weekly, keep_monthly, keep_yearly, prefix)
    //run_cmd(cmd)

    fn backup_prune(&self) {
        for repo in &self.repositories {
            if Borg::is_repo(repo) {
                let mut prefixes = self.get_remote_prefixes();
                prefixes.push(self.get_local_prefix());
                prefixes.iter().for_each(|prefix| {
                    let cmd = format!("borg prune --list --stats -v {repo} --keep-daily={} --keep-weekly={} --keep-monthly={} --keep-yearly={} --prefix={prefix}",
                                  self.prune_settings.daily,
                                  self.prune_settings.weekly,
                                  self.prune_settings.monthly,
                                  self.prune_settings.yearly);
                    run_cmd_piped(&cmd);
                });
            }
        }
    }

    fn is_repo(repo: &str) -> bool {
        //return true;
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
        let mut target_paths = Vec::new();
        let mut dirs_to_check = folders.clone();
        let mut visited_dirs = HashSet::new();
        while let Some(dir) = dirs_to_check.pop() {
            if let Ok(dir_contents) = fs::read_dir(dir) {
                for subentry in dir_contents {
                    let subentry = subentry.unwrap();
                    let subpath = subentry.path();
                    let real_path = fs::read_link(&subpath).unwrap_or_else(|_| subpath.clone());
                    if real_path.is_dir() && !visited_dirs.insert(real_path) {
                        println!("Already visited {}", subpath.display());
                        continue;
                    }

                    if subpath.is_dir() && !subpath.is_symlink() {
                        dirs_to_check.push(subpath.to_string_lossy().to_string());
                    } else if subpath.ends_with("Cargo.toml") {
                        let target_path =
                            Path::new(&subpath).parent().unwrap().join("target").clone();
                        if target_path.exists() {
                            println!("#### ignoring {target_path:?}");
                            target_paths.push(target_path);
                        }
                    }
                }
            }
        }

        local_excludes.extend(
            target_paths
                .into_iter()
                .map(|p| p.to_string_lossy().to_string()),
        );

        let folder_exclude_str: String = local_excludes
            .into_iter()
            .map(|val| format!(" --exclude {val}"))
            .collect();

        let cmd = format!("borg create {options} {repo}::{name} {folder_list_str} {folder_exclude_str} --exclude-if-present .nobackup");
        let _res = run_cmd_piped(&cmd);
    }

    fn _backup_create_remote(
        options: &str,
        repo: &str,
        name: &str,
        remote_folders: &RemoteFolders,
        excludes: &Vec<String>,
    ) {
        for (host, folders) in remote_folders.iter() {
            let temp_dir = format!("/tmp/backup/{host}");
            let (_user, hostname) = host.split_once('@').unwrap();
            std::fs::create_dir_all(&temp_dir).unwrap_or_default();
            let cmd = format!("sshfs {host}:/ {temp_dir}");
            let output = run_cmd(&cmd);
            let backup_dirs = folders
                .iter()
                .map(|folder| {
                    let new_path = temp_dir.clone() + folder;
                    Path::new(&new_path).to_str().unwrap().to_string()
                })
                .collect();
            println!("Output dirs: {:?}", backup_dirs);
            if output.status.success() {
                Borg::_backup_create(
                    &format!("--files-cache ctime,size {options}"),
                    repo,
                    &format!("{hostname}-{name}"),
                    &backup_dirs,
                    excludes,
                );
                let cmd = format!("fusermount -u {temp_dir}");
                run_cmd(&cmd);
            }
        }
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
    borg.backup_create_remote();
    borg.backup_prune();
}
