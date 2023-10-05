use nix_uri::FlakeRef;

fn main() {
    let uri = "github:nixos/nixpkgs";
    let rev = "nixos-unstable";
    let mut flake_ref: FlakeRef = uri.parse().unwrap();
    flake_ref.r#type.ref_or_rev(Some(rev.to_owned())).ok();

    println!("The uri is: {uri}");
    println!("The rev is: {rev}");
    println!("The changed uri is: {flake_ref}");
}
