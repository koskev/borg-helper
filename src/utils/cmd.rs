use std::process::{Command, Output, Stdio};

pub fn run_cmd(cmd: &str) -> Output {
    println!("Calling \"{}\"", cmd);
    let output = Command::new("sh")
        .arg("-c")
        .arg(cmd)
        .output()
        .expect("failed to execute process");

    output
}

pub fn run_cmd_piped(cmd: &str) -> Output {
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
