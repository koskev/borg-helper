use std::fmt::Debug;
use std::fs::File;
use std::io::{stdin, BufReader};
use std::ops::Deref;
use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};
use std::str::FromStr;

use chrono::{DateTime, Local};
use secstr::SecUtf8;
use serde::Deserialize;
use serde::Serialize;
use serde_with::{DisplayFromStr, PickFirst};
use void::Void;

#[derive(Serialize, Deserialize, Debug, Clone, Default, PartialEq)]
struct PruneSettings {
    yearly: Option<u32>,
    monthly: Option<u32>,
    weekly: Option<u32>,
    daily: Option<u32>,
    hourly: Option<u32>,
}

impl PruneSettings {
    fn merge(&self, parent: &PruneSettings) -> Self {
        Self {
            yearly: self.yearly.or(parent.yearly),
            monthly: self.monthly.or(parent.monthly),
            weekly: self.weekly.or(parent.weekly),
            daily: self.daily.or(parent.daily),
            hourly: self.hourly.or(parent.hourly),
        }
    }
}

#[derive(Serialize, Deserialize, Debug, Clone)]
struct PasswordStore {
    system: String,
    user: String,
}

trait Password {
    fn get_password(&self) -> Option<SecUtf8>;
}

impl Password for PasswordStore {
    fn get_password(&self) -> Option<SecUtf8> {
        let cmd = format!("secret-tool lookup {} {}", self.system, self.user);
        let output = run_cmd(&cmd);
        if output.status.success() {
            return Some(SecUtf8::from_str(std::str::from_utf8(&output.stdout).unwrap()).unwrap());
        }
        None
    }
}

#[derive(Serialize, Deserialize, Debug, Clone)]
struct PlainPassword {
    value: SecUtf8,
}

impl Password for PlainPassword {
    fn get_password(&self) -> Option<SecUtf8> {
        Some(self.value.clone())
    }
}

#[derive(Serialize, Deserialize, Debug, Clone)]
#[serde(tag = "type")]
enum PasswordOptions {
    #[serde(rename = "plain")]
    Plain(PlainPassword),
    #[serde(rename = "password_store")]
    PasswordStore(PasswordStore),
}

impl Password for PasswordOptions {
    fn get_password(&self) -> Option<SecUtf8> {
        match self {
            Self::Plain(p) => p.get_password(),
            Self::PasswordStore(p) => p.get_password(),
        }
    }
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
#[serde(tag = "type")]
enum BackupType {
    #[default]
    #[serde(rename = "local")]
    LOCAL,
    #[serde(rename = "ssh")]
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
struct FolderEntry {
    #[serde(default)]
    tags: Vec<String>,
    path: String,
}

impl FromStr for FolderEntry {
    type Err = Void;
    fn from_str(value: &str) -> Result<Self, Self::Err> {
        Ok(Self {
            path: value.to_string(),
            ..Default::default()
        })
    }
}

#[serde_with::serde_as]
#[derive(Serialize, Deserialize, Debug)]
struct BackupGroup {
    name: String,
    #[serde(default, flatten)]
    r#type: BackupType,
    //#[serde(deserialize_with = "string_or_struct_vec")]
    #[serde_as(as = "Vec<PickFirst<(_, DisplayFromStr)>>")]
    folders: Vec<FolderEntry>,
}

struct SecrectString(SecUtf8);

impl Deref for SecrectString {
    type Target = SecUtf8;
    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl Debug for SecrectString {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Password: ***SECRET***").finish()
    }
}

impl Default for SecrectString {
    fn default() -> Self {
        Self(SecUtf8::from_str("").unwrap())
    }
}

#[derive(Serialize, Deserialize, Debug, Default)]
struct Repository {
    path: String,
    #[serde(default)]
    tags: Vec<String>,

    #[serde(default, flatten)]
    options: RepositoryOptions,
}

impl Repository {
    fn is_valid(&self) -> bool {
        let cmd = format!("borg info {}", self.path);
        let output = run_cmd(&cmd);
        return output.status.success();
    }
}

impl FromStr for Repository {
    type Err = Void;
    fn from_str(value: &str) -> Result<Self, Self::Err> {
        Ok(Self {
            path: value.to_string(),
            ..Default::default()
        })
    }
}

#[serde_with::serde_as]
#[derive(Serialize, Deserialize, Debug)]
struct Repositories {
    #[serde_as(as = "Vec<PickFirst<(_, DisplayFromStr)>>")]
    repositories: Vec<Repository>,

    options: RepositoryOptions,
}

#[derive(Serialize, Deserialize, Debug, Default, Clone)]
struct RepositoryOptions {
    prune: Option<PruneSettings>,
    password: Option<PasswordOptions>,
    cmdline: Option<String>,
}

impl RepositoryOptions {
    fn merge(&self, parent: &RepositoryOptions) -> Self {
        let prune = match self.prune.clone() {
            Some(p) => match parent.prune.clone() {
                Some(pp) => Some(p.merge(&pp)),
                None => Some(p),
            },
            None => parent.prune.clone(),
        };

        Self {
            prune,
            password: self.password.clone().or(parent.password.clone()),
            cmdline: self.cmdline.clone().or(parent.cmdline.clone()),
        }
    }
}

#[serde_with::serde_as]
#[derive(Serialize, Deserialize, Debug)]
struct Borg {
    repository: Repositories,
    backups: Vec<BackupGroup>,

    excludes: Vec<String>,
    conditional_excludes: Vec<ConditionalExclude>,

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
    fn from_file(config_path: &str) -> Self {
        let conf_reader = BufReader::new(File::open(config_path).unwrap());
        let mut obj: Borg = serde_yaml::from_reader(conf_reader).unwrap();
        obj.date = Local::now();

        obj
    }

    fn from_str(config: &str) -> Self {
        let mut obj: Borg = serde_yaml::from_str(config).unwrap();
        obj.date = Local::now();

        obj
    }

    fn backup_create(&self) {
        for repo in &self.repository.repositories {
            let options = &repo.options.merge(&self.repository.options);
            let mut password = None;
            match &options.password {
                Some(pwo) => password = pwo.get_password(),
                None => (),
            }
            if password.is_none() {
                // Ask for password
                // XXX: any way to prevent this?
                let pw =
                    rpassword::prompt_password(format!("Enter Password for repo {}:", repo.path))
                        .unwrap();
                password = Some(SecUtf8::from_str(&pw).unwrap());
            }
            match &password {
                Some(pw) => std::env::set_var("BORG_PASSPHRASE", pw.unsecure()),
                None => (),
            }
            drop(password);
            if repo.is_valid() {
                for backup_source in &self.backups {
                    let mount_point = backup_source.r#type.pre_backup();
                    match mount_point {
                        Some(mount_point) => {
                            let folders: Vec<String> = backup_source
                                .folders
                                .iter()
                                // Filter tags
                                .filter(|f| repo.tags.iter().any(|item| f.tags.contains(item)))
                                .map(|f| {
                                    PathBuf::from(mount_point.clone())
                                        .join(f.path.clone())
                                        .to_str()
                                        .unwrap()
                                        .to_string()
                                })
                                .collect();
                            // Create Backup
                            if folders.len() > 0 {
                                Borg::_backup_create(
                                    &format!(
                                        "{} {}",
                                        backup_source
                                            .r#type
                                            .get_additional_options()
                                            .unwrap_or("".to_string()),
                                        &options.cmdline.clone().unwrap_or_default()
                                    ),
                                    &repo.path,
                                    &format!(
                                        "{}-{}",
                                        backup_source.r#type.get_prefix(),
                                        self.date.to_rfc3339()
                                    ),
                                    &folders,
                                    &self.excludes,
                                )
                            }
                        }
                        None => (),
                    }
                    backup_source.r#type.post_backup();
                }
            } else {
                println!("Skipping repo {}", repo.path);
            }
        }
    }

    fn backup_prune(&self) {
        let prefixes: Vec<String> = self.backups.iter().map(|b| b.r#type.get_prefix()).collect();
        prefixes.iter().for_each(|prefix| {
            //let mut keep_vec = vec![];
            //let cmd = format!("prune --list --stats -v --keep-daily={} --keep-weekly={} --keep-monthly={} --keep-yearly={} --glob-archives '{prefix}*'",
            //                  self.prune_settings.daily,
            //                  self.prune_settings.weekly,
            //                  self.prune_settings.monthly,
            //                  self.prune_settings.yearly);
            //self.run_every_repo(&cmd);
        });
    }

    fn run_every_repo(&self, command: &str) {
        for repo in &self.repository.repositories {
            if repo.is_valid() {
                let cmd = format!("borg {} {}", command, repo.path);
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

fn main() {
    let borg = Borg::from_file("config.yaml");
    println!("{:?}", borg);
    //let pw = get_password(&borg.password_store.system, &borg.password_store.user).unwrap();
    //env::set_var("BORG_PASSPHRASE", pw);
    borg.backup_create();
    borg.backup_prune();
}

#[cfg(test)]
mod test {
    use std::str::FromStr;

    use secstr::SecUtf8;

    use crate::{BackupType, Borg, Password, PruneSettings, Repository};

    #[test]
    fn test_from_str() {
        let repo1 = Repository {
            path: "a".to_string(),
            tags: vec![],
            ..Default::default()
        };
        let repo2 = Repository::from_str("a").unwrap();

        assert_eq!(repo1.path, repo2.path);
        assert_eq!(repo1.tags, repo2.tags);
    }

    #[test]
    fn test_config() {
        let config = include_str!("../config.yaml.example");

        let borg = Borg::from_str(config);
        assert_eq!(borg.backups.len(), 2);
        match &borg.backups[0].r#type {
            BackupType::SSH(ssh) => {
                assert_eq!(ssh.target, "user@server.de");
            }
            _ => assert!(false),
        }

        match &borg.backups[1].r#type {
            BackupType::LOCAL => {
                assert!(true);
            }
            _ => assert!(false),
        }

        assert_eq!(borg.backups[1].folders.len(), 2);

        assert!(borg.backups[1].folders[1]
            .tags
            .contains(&"important".to_string()));

        assert_eq!(borg.repository.repositories.len(), 3);
        assert!(borg.repository.options.prune.is_some());
        assert!(borg.repository.options.cmdline.is_some());
        let expeted_prune = PruneSettings {
            daily: Some(7),
            monthly: Some(6),
            weekly: Some(4),
            ..Default::default()
        };
        assert_eq!(
            borg.repository.options.prune.clone().unwrap(),
            expeted_prune
        );
        assert!(borg.repository.repositories[1].options.prune.is_some());

        // check overwrite prune
        let mut repo_prune = expeted_prune.clone();
        repo_prune.weekly = Some(2);

        assert_eq!(
            borg.repository.repositories[1]
                .options
                .prune
                .clone()
                .unwrap()
                .merge(&expeted_prune),
            repo_prune
        );

        assert!(borg.repository.options.password.is_some());

        let pw = borg.repository.repositories[1].options.password.clone();

        assert!(pw.is_some());
        assert!(pw.clone().unwrap().get_password().is_some());

        assert_eq!(pw.unwrap().get_password().unwrap().unsecure(), "mypassword");
    }
}
