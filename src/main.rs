use std::fmt::Debug;
use std::fs::File;
use std::io::BufReader;
use std::path::PathBuf;
use std::process::{Child, Command, Output, Stdio};
use std::str::FromStr;

use chrono::{DateTime, Local};
use clap::Parser;
use log::{debug, info, warn, LevelFilter};
use secstr::SecUtf8;
use serde::Deserialize;
use serde::Serialize;
use serde_with::{DisplayFromStr, PickFirst};
use simplelog::{ColorChoice, Config, TermLogger, TerminalMode};
use utils::folder::BackupGroup;
use void::Void;

mod sources;
mod utils;

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

impl From<SecUtf8> for PlainPassword {
    fn from(value: SecUtf8) -> Self {
        Self { value }
    }
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
struct Repository {
    path: String,
    #[serde(default)]
    tags: Vec<String>,

    #[serde(default, flatten)]
    options: RepositoryOptions,
}

impl Repository {
    fn merge_options(&mut self, options: &RepositoryOptions) {
        self.options = self.options.merge(options);
    }

    fn export_password(&self) {
        let options = &self.options;
        let mut password = None;
        match &options.password {
            Some(pwo) => password = pwo.get_password(),
            None => (),
        }
        if password.is_none() {
            // Ask for password
            // TODO: cache password?
            let pw = rpassword::prompt_password(format!("Enter Password for repo {}:", self.path))
                .unwrap();
            password = Some(SecUtf8::from_str(&pw).unwrap());
        }
        match &password {
            Some(pw) => std::env::set_var("BORG_PASSPHRASE", pw.unsecure()),
            None => (),
        }
    }

    fn is_valid(&self) -> bool {
        self.export_password();
        let cmd = format!("borg info {}", self.path);
        let output = run_cmd(&cmd);
        output.status.success()
    }

    fn backup_create(
        &self,
        backup_source_groups: &[BackupGroup],
        excludes: &[String],
        date: &DateTime<Local>,
    ) {
        self.export_password();
        if self.is_valid() {
            info!("Processing {}", self.path);
            for backup_source in backup_source_groups {
                info!("Processing source {}", backup_source.name);
                let mut folders = backup_source.r#type.get_folders();
                if self.tags.len() > 0 {
                    folders = folders
                        .into_iter()
                        .filter(|f| self.tags.iter().any(|item| f.tags.contains(item)))
                        .collect();
                }
                if backup_source.r#type.pre_backup() {
                    let paths: Vec<PathBuf> = folders.iter().map(|f| f.folder.get_path()).collect();
                    info!("Backing up folders {:?}", paths);
                    // Create Backup
                    if folders.len() > 0 {
                        Borg::_backup_create(
                            &format!(
                                "{} {}",
                                backup_source.r#type.get_additional_options(),
                                &self.options.cmdline.clone().unwrap_or_default()
                            ),
                            &self.path,
                            &format!(
                                "{}-{}",
                                backup_source.r#type.get_hostname(),
                                date.to_rfc3339()
                            ),
                            &paths,
                            excludes,
                        )
                    }
                }
                backup_source.r#type.post_backup();
            }
        } else {
            warn!("Skipping repo {}", self.path);
        }
    }

    fn backup_prune(&self, backup_groups: &[BackupGroup]) {
        let prefixes: Vec<String> = backup_groups
            .iter()
            .map(|b| b.r#type.get_hostname())
            .collect();
        prefixes.iter().for_each(|prefix| {
                if self.is_valid() {
                    //let mut keep_vec = vec![];
                    let prune_options = self.options.prune.clone().unwrap_or_default();
                    let cmd = format!("borg prune --list --stats -v --keep-daily={} --keep-weekly={} --keep-monthly={} --keep-yearly={} --glob-archives '{prefix}*' {}",
                                      prune_options.daily.unwrap_or_default(),
                                      prune_options.weekly.unwrap_or_default(),
                                      prune_options.monthly.unwrap_or_default(),
                                      prune_options.yearly.unwrap_or_default(), self.path
                                      );
                    run_cmd_piped(&cmd);
                }
        });
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

    #[serde(default)]
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

    #[serde(default)]
    excludes: Vec<String>,
    #[serde(default)]
    conditional_excludes: Vec<ConditionalExclude>,

    #[serde(skip)]
    date: DateTime<Local>,
}

impl Borg {
    fn from_file(config_path: &str) -> Self {
        let conf_reader = BufReader::new(File::open(config_path).unwrap());
        let mut obj: Borg = serde_yaml::from_reader(conf_reader).unwrap();
        obj.date = Local::now();
        obj.fill_parent_options();

        obj
    }

    fn from_str(config: &str) -> Self {
        let mut obj: Borg = serde_yaml::from_str(config).unwrap();
        obj.date = Local::now();
        obj.fill_parent_options();

        obj
    }

    fn fill_parent_options(&mut self) {
        for repo in &mut self.repository.repositories {
            repo.merge_options(&self.repository.options);
        }
    }

    fn backup_create(&self) {
        for repo in &self.repository.repositories {
            repo.backup_create(&self.backups, &self.excludes, &self.date);
        }
    }

    fn backup_prune(&self) {
        self.repository.repositories.iter().for_each(|repo| {
            repo.backup_prune(&self.backups);
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

    fn _backup_create(
        options: &str,
        repo: &str,
        name: &str,
        folders: &[PathBuf],
        excludes: &[String],
    ) {
        let folder_vec_str: Vec<&str> = folders.iter().filter_map(|f| f.to_str()).collect();
        let folders_str = folder_vec_str.join(" ");

        let folder_exclude_str: String = excludes
            .into_iter()
            .map(|val| format!(" --exclude {val}"))
            .collect();

        let cmd = format!("borg create {options} {repo}::{name} {folders_str} {folder_exclude_str} --exclude-if-present .nobackup --exclude-if-present CACHEDIR.TAG");
        let _res = run_cmd_piped(&cmd);
    }

    fn get_sizes(&self) {
        for backup_source in &self.backups {
            let folders = backup_source.r#type.get_folders();
            // TODO: fix multiple mount calls. Fix auto mount stuff
            for folder_entry in folders {
                let size = folder_entry.folder.get_size().unwrap_or_default();
                let size_str = byte_unit::Byte::from_u64(size)
                    .get_appropriate_unit(byte_unit::UnitType::Binary);
                info!(
                    "{}: {:.2}",
                    folder_entry.folder.get_path().to_str().unwrap_or_default(),
                    size_str
                );
            }

            backup_source.r#type.post_backup();
        }
    }
}

fn run_cmd_piped(cmd: &str) -> Output {
    info!("Calling piped \"{}\"", cmd);
    let output = Command::new("sh")
        .arg("-c")
        .arg(cmd)
        .stdout(Stdio::inherit())
        .stderr(Stdio::inherit())
        .output()
        .expect("failed to execute process");

    output
}

fn run_cmd_background(cmd: &str) -> Result<Child, std::io::Error> {
    Command::new("sh").arg("-c").arg(cmd).spawn()
}

fn run_cmd(cmd: &str) -> Output {
    info!("Calling \"{}\"", cmd);
    let output = Command::new("sh")
        .arg("-c")
        .arg(cmd)
        .output()
        .expect("failed to execute process");

    output
}

#[derive(Parser)]
#[command(version, about, long_about = None)]
struct Cli {
    #[arg(short, long)]
    show_size: bool,
}

fn main() {
    TermLogger::init(
        LevelFilter::Trace,
        Config::default(),
        TerminalMode::Stdout,
        ColorChoice::Auto,
    )
    .unwrap();
    let cli = Cli::parse();
    let borg = Borg::from_file("config.yaml");
    debug!("{:?}", borg);
    if cli.show_size {
        borg.get_sizes();
    } else {
        borg.backup_create();
        borg.backup_prune();
    }
}

#[cfg(test)]
mod test {
    use std::str::FromStr;

    use include_dir::{include_dir, Dir};

    use crate::{Borg, Password, PruneSettings, Repository};

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
    fn test_configs() {
        static DIR: Dir<'_> = include_dir!("test/configs");
        for config in DIR.files() {
            println!("Testing {:?}", config);
            let _borg = Borg::from_str(std::str::from_utf8(config.contents()).unwrap());
        }
    }

    #[test]
    fn test_full_config() {
        let config = include_str!("../test/configs/99-full.yaml");

        let borg = Borg::from_str(config);
        println!("{:#?}", borg);
        assert_eq!(borg.backups.len(), 2);
        // SSH
        assert_ne!(borg.backups[0].r#type.get_additional_options().len(), 0);

        // LOCAL
        assert_eq!(borg.backups[1].r#type.get_additional_options().len(), 0);
        let local_folders = borg.backups[1].r#type.get_folders();

        assert_eq!(local_folders.len(), 2);

        assert!(local_folders[1].tags.contains(&"important".to_string()));

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
                .unwrap(),
            repo_prune
        );

        assert!(borg.repository.options.password.is_some());

        let pw = borg.repository.repositories[1].options.password.clone();

        assert!(pw.is_some());
        assert!(pw.clone().unwrap().get_password().is_some());

        assert_eq!(pw.unwrap().get_password().unwrap().unsecure(), "mypassword");
    }
}
