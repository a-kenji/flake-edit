//! Git tag integration

use std::process::{Command, Stdio};
pub struct Tags {
    uri: String,
    tags: Vec<Tag>,
}

pub struct Tag {
    hash: String,
    tag: String,
}

impl Tags {
    fn from_uri(uri: &str) {
        let output = Command::new("git")
            .arg("ls-remote")
            .arg("--tags")
            .arg(uri)
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .output()
            .expect("Failed to execute command");

        if output.status.success() {
            let stdout = String::from_utf8_lossy(&output.stdout);
            println!("Command output:\n{}", stdout);
        } else {
            let stderr = String::from_utf8_lossy(&output.stderr);
            eprintln!("Error executing command:\n{}", stderr);
        }
    }
    /// Multiple lines, some without tags
    fn from_git(stream: &str) -> Result<(), Self> {
        let mut res = Vec::new();
        let lines = stream.lines();
        for line in lines {
            let mut line = line.split_whitespace();
            let hash = line.next().unwrap();
            let tag = line.next().unwrap();
            res.push(Tag {
                hash: hash.to_owned(),
                tag: tag.to_owned(),
            });
        }
        Ok(())
    }
}
