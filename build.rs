use clap::CommandFactory;
use clap::ValueEnum;
use clap_complete::{Shell, generate_to};
use clap_complete_nushell::Nushell;
use clap_mangen::Man;

use std::fs;

use std::fs::OpenOptions;
use std::io::Write;
use std::path::PathBuf;
use std::{env, fs::create_dir_all, path::Path};

include!("src/bin/flake-edit/cli.rs");

fn main() {
    println!("cargo:rerun-if-env-changed=ASSET_DIR");

    // Only run asset generation when the assets feature is enabled
    #[cfg(feature = "assets")]
    {
        const NAME: &str = "flake-edit";
        const COMPLETIONS_DIR: &str = "assets/completions";
        const FISH_COMPLETIONS: &str = "fish/completions.fish";
        let manifest_dir =
            env::var_os("CARGO_MANIFEST_DIR").expect("Could not find env CARGO_MANIFEST_DIR");

        if let Some(dir) = env::var_os("ASSET_DIR") {
            let out = &Path::new(&dir);
            create_dir_all(out).unwrap();
            let cmd = &mut CliArgs::command();

            gen_man(NAME, out.to_path_buf());

            Shell::value_variants().iter().for_each(|shell| {
                generate_to(*shell, cmd, NAME.to_string(), out).unwrap();
                // claps completions generation mechanisms are very immature,
                // include self adjusted ones
                // Explicitly ignore patterns
                #[allow(clippy::wildcard_in_or_patterns)]
                match shell {
                    Shell::Fish => {
                        let mut source = PathBuf::from(manifest_dir.clone());
                        source.push(COMPLETIONS_DIR);
                        source.push(FISH_COMPLETIONS);
                        let source =
                            fs::read_to_string(source).expect("Could not read source file");
                        let path = out.join(format!("{NAME}.fish"));
                        let mut file = OpenOptions::new()
                            .append(true)
                            .open(path)
                            .expect("Could not create path.");
                        let _ = file.write_all(source.as_bytes());
                    }
                    Shell::Zsh | Shell::PowerShell | Shell::Bash | Shell::Elvish | _ => {}
                }
            });
            generate_to(Nushell, cmd, NAME.to_string(), out).unwrap();
        } else {
            eprintln!("ASSET_DIR environment variable not set");
            eprintln!("Not able to generate completion files");
            eprintln!("Not able to generate manpage files");
        }
    }
}

#[cfg(feature = "assets")]
fn gen_man(name: &str, dir: PathBuf) {
    use roff::Roff;
    use std::fs::write;

    let path = dir.join(format!("{name}.1"));
    let mut buf: Vec<u8> = Vec::new();
    let man = Man::new(CliArgs::command());

    man.render_title(&mut buf)
        .expect("Not able to render title.");
    man.render_name_section(&mut buf)
        .expect("Not able to render name section.");
    man.render_synopsis_section(&mut buf)
        .expect("Not able to render synopsis section.");
    let mut roff = Roff::new();
    roff.control("SH", ["DESCRIPTION"]);
    roff.text(vec!["Edit your flake inputs with ease.".into()]);
    roff.to_writer(&mut buf)
        .expect("Not able to write description.");
    // man.render_description_section(&mut buf)
    //     .expect("Not able to render description section.");
    man.render_options_section(&mut buf)
        .expect("Not able to render options section.");
    man.render_subcommands_section(&mut buf)
        .expect("Not able to render subcommands section.");

    // Examples
    roff.control("SH", ["EXAMPLES"]);

    // Add a new flake input
    roff.text(vec![format!("Add a new flake input:").into()]);
    roff.control("RS", []);
    roff.text(vec![
        format!("{} add nixpkgs github:NixOS/nixpkgs", name).into(),
    ]);
    roff.control("RE", []);
    roff.text(vec!["".into()]);

    // Add with auto-inference
    roff.text(vec![
        format!("Add an input with automatic ID inference:").into(),
    ]);
    roff.control("RS", []);
    roff.text(vec![
        format!("{} add github:nix-community/home-manager", name).into(),
    ]);
    roff.control("RE", []);
    roff.text(vec!["".into()]);

    // Remove an input
    roff.text(vec![format!("Remove a flake input:").into()]);
    roff.control("RS", []);
    roff.text(vec![format!("{} remove nixpkgs", name).into()]);
    roff.control("RE", []);
    roff.text(vec!["".into()]);

    // List inputs
    roff.text(vec![format!("List all current inputs:").into()]);
    roff.control("RS", []);
    roff.text(vec![format!("{} list", name).into()]);
    roff.control("RE", []);
    roff.text(vec!["".into()]);

    // Update inputs
    roff.text(vec![
        format!("Update all inputs to latest versions:").into(),
    ]);
    roff.control("RS", []);
    roff.text(vec![format!("{} update", name).into()]);
    roff.control("RE", []);
    roff.text(vec!["".into()]);

    // Pin an input
    roff.text(vec![
        format!("Pin an input to its current revision:").into(),
    ]);
    roff.control("RS", []);
    roff.text(vec![format!("{} pin nixpkgs", name).into()]);
    roff.control("RE", []);
    roff.text(vec!["".into()]);

    // Show diff without applying changes
    roff.text(vec![
        format!("Preview changes without applying them:").into(),
    ]);
    roff.control("RS", []);
    roff.text(vec![
        format!(
            "{} --diff add home-manager github:nix-community/home-manager",
            name
        )
        .into(),
    ]);
    roff.control("RE", []);
    roff.text(vec!["".into()]);

    // Skip lockfile update
    roff.text(vec![format!("Add input without updating lockfile:").into()]);
    roff.control("RS", []);
    roff.text(vec![
        format!(
            "{} --no-lock add nixos-hardware github:NixOS/nixos-hardware",
            name
        )
        .into(),
    ]);
    roff.control("RE", []);

    roff.to_writer(&mut buf).expect("Not able to write roff.");

    // Footer
    man.render_version_section(&mut buf)
        .expect("Not able to render subcommands section.");
    man.render_authors_section(&mut buf)
        .expect("Not able to render subcommands section.");

    write(path, buf).expect("Not able to write manpage");
}
