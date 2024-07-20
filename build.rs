use clap::CommandFactory;
use clap::ValueEnum;
use clap_complete::{generate_to, Shell};
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
            // Explicily ignore patterns
            #[allow(clippy::wildcard_in_or_patterns)]
            match shell {
                Shell::Fish => {
                    let mut source = PathBuf::from(manifest_dir.clone());
                    source.push(COMPLETIONS_DIR);
                    source.push(FISH_COMPLETIONS);
                    let source = fs::read_to_string(source).expect("Could not read source file");
                    let path = out.join(format!("{NAME}.fish"));
                    let mut file = OpenOptions::new()
                        .append(true)
                        .open(path)
                        .expect("Could not create path.");
                    let _ = file.write_all(source.as_bytes());
                }
                Shell::Zsh => {}
                Shell::PowerShell | Shell::Bash | Shell::Elvish | _ => {}
            }
        });
        generate_to(Nushell, cmd, NAME.to_string(), out).unwrap();
    } else {
        eprintln!("ASSET_DIR environment variable not set");
        eprintln!("Not able to generate completion files");
        eprintln!("Not able to generate manpage files");
    }
}

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
    roff.text(["Edit your flake inputs with ease.".into()]);
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
    roff.to_writer(&mut buf).expect("Not able to write roff.");

    // Footer
    man.render_version_section(&mut buf)
        .expect("Not able to render subcommands section.");
    man.render_authors_section(&mut buf)
        .expect("Not able to render subcommands section.");

    write(path, buf).expect("Not able to write manpage");
}
