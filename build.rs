#[cfg(feature = "assets")]
pub mod asset_build {
    use clap::CommandFactory;
    use clap_complete::env::{Bash, EnvCompleter, Fish, Zsh};
    use clap_complete::generate_to;
    use clap_complete_nushell::Nushell;
    use clap_mangen::Man;
    use std::fs;
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

        if let Some(dir) = env::var_os("ASSET_DIR") {
            let out = &Path::new(&dir);
            create_dir_all(out).unwrap();
            let cmd = &mut CliArgs::command();

            gen_man(NAME, out.to_path_buf());

            gen_registration(&Bash, NAME, out, format!("{NAME}.bash"));
            gen_registration(&Zsh, NAME, out, format!("_{NAME}"));
            gen_registration(&Fish, NAME, out, format!("{NAME}.fish"));

            // Nushell has no dynamic env adapter in clap_complete
            generate_to(Nushell, cmd, NAME.to_string(), out).unwrap();
        } else {
            eprintln!("ASSET_DIR environment variable not set");
            eprintln!("Not able to generate completion files");
            eprintln!("Not able to generate manpage files");
        }
    }

    fn gen_registration<C: EnvCompleter>(shell: &C, name: &str, out: &Path, filename: String) {
        let mut buf: Vec<u8> = Vec::new();
        shell
            .write_registration("COMPLETE", name, name, name, &mut buf)
            .expect("Not able to render completion registration.");
        fs::write(out.join(filename), buf).expect("Not able to write completion registration");
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
