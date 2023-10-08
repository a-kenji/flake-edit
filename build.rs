use clap::CommandFactory;
use clap::ValueEnum;
use clap_complete::{generate_to, Shell};
use clap_mangen::Man;

use std::{
    env,
    fs::{create_dir_all, File},
    path::Path,
};

include!("src/bin/fe/cli.rs");

fn main() {
    println!("cargo:rerun-if-env-changed=ASSET_DIR");

    // const NAME: &str = "flake-add";
    const NAME: &str = "fe";

    if let Some(dir) = env::var_os("ASSET_DIR") {
        let out = &Path::new(&dir);
        create_dir_all(out).unwrap();
        let cmd = &mut CliArgs::command();

        Man::new(cmd.clone())
            .render(&mut File::create(out.join(format!("{NAME}.1"))).unwrap())
            .unwrap();

        Shell::value_variants().iter().for_each(|shell| {
            generate_to(*shell, cmd, NAME.to_string(), out).unwrap();
        });
    } else {
        eprintln!("ASSET_DIR environment variable not set");
        eprintln!("Not able generate completion files");
        eprintln!("Not able generate manpage files");
    }
}
