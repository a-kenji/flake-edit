use clap::CommandFactory;
use clap::ValueEnum;
use clap_complete::{generate_to, Shell};
use clap_complete_nushell::Nushell;
use clap_mangen::Man;

use std::path::PathBuf;
use std::{env, fs::create_dir_all, path::Path};

include!("src/bin/fe/cli.rs");

fn main() {
    println!("cargo:rerun-if-env-changed=ASSET_DIR");

    const NAME: &str = "fe";

    if let Some(dir) = env::var_os("ASSET_DIR") {
        let out = &Path::new(&dir);
        create_dir_all(out).unwrap();
        let cmd = &mut CliArgs::command();

        gen_man(NAME, out.to_path_buf());

        Shell::value_variants().iter().for_each(|shell| {
            generate_to(*shell, cmd, NAME.to_string(), out).unwrap();
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
