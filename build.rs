#[cfg(feature = "assets")]
pub mod asset_build {
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

    pub mod cli {
        include!("src/cli.rs");
    }
    use cli::*;

    pub fn run() {
        println!("cargo:rerun-if-env-changed=ASSET_DIR");
        println!("cargo:rerun-if-changed=docs/man/flake-edit.md");

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
                if *shell == Shell::Fish {
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
            });
            generate_to(Nushell, cmd, NAME.to_string(), out).unwrap();
        } else {
            eprintln!("ASSET_DIR environment variable not set");
            eprintln!("Not able to generate completion files");
            eprintln!("Not able to generate manpage files");
        }
    }

    fn gen_man(name: &str, dir: PathBuf) {
        use std::fs::write;

        const PROSE_MD: &str = include_str!("docs/man/flake-edit.md");

        let path = dir.join(format!("{name}.1"));
        let mut buf: Vec<u8> = Vec::new();
        let man = Man::new(CliArgs::command());

        man.render_title(&mut buf)
            .expect("Not able to render title.");
        buf.extend_from_slice(b".nh\n.ad l\n");
        man.render_name_section(&mut buf)
            .expect("Not able to render name section.");
        man.render_synopsis_section(&mut buf)
            .expect("Not able to render synopsis section.");
        man.render_description_section(&mut buf)
            .expect("Not able to render description section.");
        man.render_options_section(&mut buf)
            .expect("Not able to render options section.");
        man.render_subcommands_section(&mut buf)
            .expect("Not able to render subcommands section.");

        let prose =
            manners::to_roff(PROSE_MD).expect("Not able to render docs/man/flake-edit.md to roff");
        prose
            .write_to(&mut buf)
            .expect("Not able to write prose roff fragment");

        man.render_authors_section(&mut buf)
            .expect("Not able to render authors section.");

        write(path, buf).expect("Not able to write manpage");
    }
}

#[cfg(feature = "assets")]
fn main() {
    asset_build::run();
}

#[cfg(not(feature = "assets"))]
fn main() {
    // Keep build.rs compiling when the assets feature (and build deps) are disabled.
}
