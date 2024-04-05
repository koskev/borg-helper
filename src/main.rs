use std::collections::HashMap;
use std::fmt::{Debug, Display, Write};
use std::fs::File;
use std::io::BufReader;
use std::io::Write as ioWrite;
use std::path::PathBuf;
use std::str::FromStr;

use chrono::{DateTime, Local};
use clap::Parser;
use log::{debug, info, warn, LevelFilter};
use mktemp::Temp;
use secstr::SecUtf8;
use serde::Deserialize;
use serde::Serialize;
use serde_with::{DisplayFromStr, PickFirst};
use simplelog::{ColorChoice, Config, TermLogger, TerminalMode};
use utils::cmd::{run_cmd, run_cmd_inherit, run_cmd_piped};
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
                if !self.tags.is_empty() {
                    folders.retain(|f| self.tags.iter().any(|item| f.tags.contains(item)));
                }
                if backup_source.r#type.pre_backup() {
                    let paths: Vec<PathBuf> = folders.iter().map(|f| f.folder.get_path()).collect();
                    info!("Backing up folders {:?}", paths);
                    // Create Backup
                    if !folders.is_empty() {
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
#[derive(Serialize, Deserialize, Debug, Default)]
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
#[derive(Serialize, Deserialize, Debug, Default)]
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

    #[allow(dead_code)]
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

    #[allow(dead_code)]
    fn run_every_repo(&self, command: &str) {
        for repo in &self.repository.repositories {
            if repo.is_valid() {
                let cmd = format!("borg {} {}", command, repo.path);
                run_cmd_piped(&cmd);
            }
        }
    }

    #[allow(dead_code)]
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
        let folder_vec_str: Vec<String> = folders
            .iter()
            .filter_map(|f| f.to_str().map(|path| format!("R {}", path)))
            .collect();
        let folders_str = folder_vec_str.join("\n");

        let folder_exclude_str = excludes.iter().fold(String::new(), |mut output, val| {
            let _ = write!(output, " --exclude {val}");
            output
        });

        let folder_file = Temp::new_file().unwrap();
        let mut f = File::create(&folder_file).unwrap();
        f.write_all(folders_str.as_bytes()).unwrap();
        drop(f);

        let cmd = format!("borg create {options} {repo}::{name} {folder_exclude_str} --exclude-if-present .nobackup --exclude-if-present CACHEDIR.TAG --patterns-from {}", folder_file.to_str().unwrap());
        run_cmd_inherit(&cmd);
    }

    fn get_sizes(&self) -> BackupSize {
        let mut sizes = BackupSize::default();
        for backup_source in &self.backups {
            let folders = backup_source.r#type.get_folders();
            // TODO: fix multiple mount calls. Fix auto mount stuff
            for folder_entry in folders {
                let skip_folder = folder_entry.options.unwrap_or_default().skip_size;
                if !skip_folder {
                    let size = folder_entry.folder.get_size().unwrap_or_default();
                    sizes.add_size(
                        &backup_source.name,
                        folder_entry.folder.get_path().to_str().unwrap(),
                        size as usize,
                    );
                }
            }

            backup_source.r#type.post_backup();
        }
        sizes
    }
}

#[derive(Debug, Default, Clone)]
struct BackupSize {
    pub sizes: HashMap<String, HashMap<String, usize>>,
}

impl BackupSize {
    fn add_size(&mut self, backup_name: &str, path: &str, size: usize) {
        self.sizes
            .entry(backup_name.to_string())
            .or_default()
            .insert(path.to_string(), size);
    }
}

impl Display for BackupSize {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        for (backup_name, path_sizes) in &self.sizes {
            let total_size: usize = path_sizes
                .values()
                .cloned()
                .reduce(|acc, size| acc + size)
                .unwrap_or(0);

            let total_size_str = byte_unit::Byte::from_u64(total_size as u64)
                .get_appropriate_unit(byte_unit::UnitType::Binary);

            write!(f, "Backup \"{}\" ({}):", backup_name, total_size_str)?;
            for (path, path_size) in path_sizes {
                let size_str = byte_unit::Byte::from_u64(*path_size as u64)
                    .get_appropriate_unit(byte_unit::UnitType::Binary);
                write!(f, "\n\t {}: {}", path, size_str)?;
            }
            write!(f, "\n")?;
        }
        Ok(())
    }
}

#[derive(Parser)]
#[command(version, about, long_about = None)]
struct Cli {
    #[arg(short, long)]
    show_size: bool,

    #[arg(short, long, default_value = "config.yaml")]
    config: String,
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
    let borg = Borg::from_file(&cli.config);
    debug!("{:?}", borg);
    if cli.show_size {
        let sizes = borg.get_sizes();
        println!("{}", sizes);
    } else {
        borg.backup_create();
        borg.backup_prune();
    }
}

#[cfg(test)]
mod test {
    use std::{
        fs::{self},
        io,
        path::{Path, PathBuf},
        str::FromStr,
    };

    use include_dir::{include_dir, Dir};
    use log::LevelFilter;
    use mktemp::Temp;
    use secstr::SecUtf8;
    use simplelog::{ColorChoice, Config, TermLogger, TerminalMode};

    use crate::{
        run_cmd,
        sources::{
            local::{LocalBackup, LocalFolder},
            ssh::{SSHBackup, SSHFolder},
        },
        utils::folder::{BackupGroup, BackupType, FolderEntry},
        Borg, Password, PasswordOptions, PlainPassword, PruneSettings, Repositories, Repository,
        RepositoryOptions,
    };

    fn create_repo() -> Temp {
        let _ = TermLogger::init(
            LevelFilter::Trace,
            Config::default(),
            TerminalMode::Stdout,
            ColorChoice::Auto,
        );
        let repo_path = Temp::new_dir().unwrap();
        let output = run_cmd(&format!(
            "borg init --encryption none {}",
            repo_path.to_str().unwrap()
        ));
        assert!(output.status.success());
        repo_path
    }

    fn get_files(dir: &Path) -> io::Result<Vec<PathBuf>> {
        let mut dir_vec = vec![];
        if dir.is_dir() {
            for entry in fs::read_dir(dir)? {
                let entry = entry?;
                let path = entry.path();
                if path.is_dir() {
                    let mut child_dir_vec = get_files(&path)?;
                    dir_vec.append(&mut child_dir_vec);
                } else {
                    dir_vec.push(entry.path());
                }
            }
        }
        Ok(dir_vec)
    }

    #[test]
    fn test_repo_ssh() {
        let repo_individual_files: Vec<PathBuf> = get_files(Path::new("./src"))
            .unwrap()
            .iter()
            .map(|f| f.canonicalize().unwrap())
            .collect();
        let repo_files = vec![PathBuf::from_str("./src").unwrap().canonicalize().unwrap()];

        let folders = repo_files
            .iter()
            .map(|f| FolderEntry {
                folder: SSHFolder {
                    path: f.as_path().to_path_buf(),
                    ..Default::default()
                },
                ..Default::default()
            })
            .collect();

        test_repo(
            Box::new(SSHBackup {
                folders,
                target: "localhost".to_string(),
            }),
            repo_individual_files,
        );
    }

    #[test]
    fn test_repo_local() {
        let repo_files = get_files(Path::new("./src")).unwrap();

        let folders = repo_files
            .iter()
            .map(|f| FolderEntry {
                folder: LocalFolder {
                    path: f.as_path().to_path_buf(),
                },
                ..Default::default()
            })
            .collect();
        test_repo(Box::new(LocalBackup { folders }), repo_files);
    }

    fn test_repo(backup_type: Box<dyn BackupType>, repo_files: Vec<PathBuf>) {
        let repo_path = create_repo();

        let borg = Borg {
            repository: Repositories {
                repositories: vec![Repository {
                    options: RepositoryOptions {
                        password: Some(PasswordOptions::Plain(PlainPassword {
                            value: SecUtf8::from_str("").unwrap(),
                        })),
                        ..Default::default()
                    },
                    path: repo_path.as_path().to_str().unwrap().to_string(),
                    ..Default::default()
                }],
                ..Default::default()
            },
            backups: vec![BackupGroup {
                name: "test".to_string(),
                r#type: backup_type,
            }],
            ..Default::default()
        };

        borg.backup_create();

        let mount_path = Temp::new_dir().unwrap();
        let _output = run_cmd(&format!(
            "borg mount {} {}",
            repo_path.as_path().to_str().unwrap(),
            mount_path.to_str().unwrap()
        ));

        let mount_files = get_files(Path::new(mount_path.as_path())).unwrap();
        run_cmd(&format!("fusermount -u {}", mount_path.to_str().unwrap()));

        // O(n^2) but we only have a low amount of files. O(n) would be using a hashset
        for mount_file in mount_files {
            assert!(repo_files.iter().any(|repo_file| {
                let mut end_path = repo_file.to_str().unwrap().to_string();
                if end_path.chars().nth(0).unwrap() == '.' {
                    end_path = end_path[1..].to_string();
                }
                mount_file.to_str().unwrap().ends_with(&end_path)
            }));
        }
    }

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
