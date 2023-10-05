#![no_main]

use libfuzzer_sys::fuzz_target;
use nix_uri::FlakeRef;

fuzz_target!(|data: String| {
    // if let Ok(s) = std::str::from_utf8(data) {
    let _parsed: Option<FlakeRef> = data.parse().ok();
    // }
});
