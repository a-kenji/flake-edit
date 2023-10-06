#![no_main]

use libfuzzer_sys::fuzz_target;
use nix_uri::{FlakeRef, NixUriResult};

// Check if the errors are the same.

fuzz_target!(|data: String| {
    let parsed: NixUriResult<FlakeRef> = data.parse();
    let nix_cmd = check_ref(&data);
    match parsed {
        Err(err) => {
            if let Ok(output) = nix_cmd {
                println!("Input: {data}");
                println!("Nix Uri Err: {err:?}");
                println!("Nix Cmd Output: {output:?}");
                panic!();
            }
        }
        Ok(parsed) => {
            if let Err(err) = nix_cmd {
                // Discard registry and file errors
                if (err.contains("error: cannot find flake")
                    && err.contains("in the flake registries"))
                    || err.contains("No such file or directory")
                    || err.contains("error: unable to download")
                    || err.contains("error: could not find a flake.nix file")
                // || err.contains("unrecognised flag")
                {
                } else {
                    println!("Input: {data}");
                    println!("Nix Cmd Err: {err}");
                    println!("Parsed Nix Uri: {parsed:#?}");
                    panic!();
                }
            }
        }
    }
});

fn check_ref(stream: &str) -> Result<(), String> {
    let cmd = "nix";
    let mut args = vec![
        "--extra-experimental-features",
        "nix-command flakes",
        "flake",
        "check",
    ];
    args.push(stream);
    let child = std::process::Command::new(cmd)
        .args(args)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .output();

    // Discard IO Errors
    match child {
        Ok(pipe) => {
            if !pipe.status.success() {
                let stderr = pipe.stderr;
                let stderr = std::str::from_utf8(&stderr).unwrap();
                return Err(stderr.into());
            }
        }
        Err(e) => {
            // println!("{e}");
            return Err(e.to_string());
        }
    }
    Ok(())
}
