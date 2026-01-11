use crate::cli::ListFormat;
use crate::edit::InputMap;
use flake_edit::input::Follows;
use std::collections::BTreeMap;

pub fn list_inputs(inputs: &InputMap, format: &ListFormat) {
    match format {
        ListFormat::Simple => list_simple(inputs),
        ListFormat::Json => list_json(inputs),
        ListFormat::Detailed => list_detailed(inputs),
        ListFormat::Raw => list_raw(inputs),
        ListFormat::Toplevel => list_toplevel(inputs),
        ListFormat::None => unreachable!("Should not be possible"),
    }
}

fn list_simple(inputs: &InputMap) {
    let mut buf = String::new();
    let mut keys: Vec<_> = inputs.keys().collect();
    keys.sort();
    for key in keys {
        let input = &inputs[key];
        if !buf.is_empty() {
            buf.push('\n');
        }
        buf.push_str(input.id());
        for follows in input.follows() {
            if let Follows::Indirect(id, _) = follows {
                let id = format!("{}.{}", input.id(), id);
                if !buf.is_empty() {
                    buf.push('\n');
                }
                buf.push_str(&id);
            }
        }
    }
    println!("{buf}");
}
fn list_json(inputs: &InputMap) {
    // Sort by key for deterministic output
    let sorted: BTreeMap<_, _> = inputs.iter().collect();
    let json = serde_json::to_string(&sorted).unwrap();
    println!("{json}");
}

fn list_toplevel(inputs: &InputMap) {
    let mut buf = String::new();
    let mut keys: Vec<_> = inputs.keys().collect();
    keys.sort();
    for key in keys {
        if !buf.is_empty() {
            buf.push('\n');
        }
        buf.push_str(&key.to_string());
    }
    println!("{buf}");
}

fn list_raw(inputs: &InputMap) {
    // Sort by key for deterministic output
    let sorted: BTreeMap<_, _> = inputs.iter().collect();
    println!("{:#?}", sorted);
}

fn list_detailed(inputs: &InputMap) {
    let mut buf = String::new();
    let mut keys: Vec<_> = inputs.keys().collect();
    keys.sort();
    for key in keys {
        let input = &inputs[key];
        if !buf.is_empty() {
            buf.push('\n');
        }
        let id = format!("· {} - {}", input.id(), input.url());
        buf.push_str(&id);
        for follows in input.follows() {
            if let Follows::Indirect(id, follow_id) = follows {
                let id = format!("{}{} => {}", " ".repeat(5), id, follow_id);
                if !buf.is_empty() {
                    buf.push('\n');
                }
                buf.push_str(&id);
            }
        }
    }
    println!("{buf}");
}
