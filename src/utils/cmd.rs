use std::process::{Child, Command, Output, Stdio};

use log::info;

pub fn run_cmd(cmd: &str) -> Output {
    info!("Calling \"{}\"", cmd);
    let output = Command::new("sh")
        .arg("-c")
        .arg(cmd)
        .output()
        .expect("failed to execute process");

    output
}

pub fn run_cmd_checked(cmd: &str) -> Result<Output, std::io::Error> {
    info!("Calling \"{}\"", cmd);
    let output = Command::new("sh").arg("-c").arg(cmd).output();

    output
}

pub fn run_cmd_inherit(cmd: &str) -> Output {
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

#[allow(dead_code)]
pub fn spawn_cmd_inherit(cmd: &str) -> Result<std::process::Child, std::io::Error> {
    info!("Spawning piped \"{}\"", cmd);
    let output = Command::new("sh")
        .arg("-c")
        .arg(cmd)
        .stdout(Stdio::inherit())
        .stderr(Stdio::inherit())
        .stdin(Stdio::piped())
        .spawn();

    output
}

pub fn run_cmd_piped(cmd: &str) -> Output {
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

pub fn run_cmd_background(cmd: &str) -> Result<Child, std::io::Error> {
    Command::new("sh").arg("-c").arg(cmd).spawn()
}
