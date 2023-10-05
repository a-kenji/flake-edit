use nix_uri::{FlakeRef, NixUriError};

fn main() {
    let maybe_input = std::env::args().nth(1);

    if let Some(input) = maybe_input {
        // let flake_ref: Result<FlakeRef, NixUriError> = input.parse();
        let flake_ref: Result<FlakeRef, NixUriError> =
            nix_uri::urls::UrlWrapper::convert_or_parse(&input);

        match flake_ref {
            Ok(flake_ref) => {
                println!(
                    "The parsed representation of the uri is the following:\n{:#?}",
                    flake_ref
                );
                println!("This is the flake_ref:\n{}", flake_ref);
            }
            Err(e) => {
                println!("There was an error parsing the uri: {input}\nError: {e}");
            }
        }
    } else {
        println!("Error: Please provide a uri.");
    }
}
