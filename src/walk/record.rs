//! Build the input model (`InputMap`) from a parsed `flake.nix`.
//!
//! Nix lets one declaration be split freely between a dotted attribute path
//! and nested attrsets. Recording therefore flattens each
//! binding under `inputs` into its full attribute path first and interprets
//! that path once, so every spelling yields the same model.

use std::collections::HashMap;

use rnix::{SyntaxKind, SyntaxNode};

use crate::follows::{AttrPath, Segment, strip_outer_quotes};
use crate::input::{Input, Range};

pub(crate) fn record_inputs(attr_set: &SyntaxNode) -> HashMap<String, Input> {
    let mut inputs = HashMap::new();
    collect_bindings(attr_set, &mut Vec::new(), &mut inputs);
    inputs
}

fn collect_bindings(
    attr_set: &SyntaxNode,
    path: &mut Vec<Segment>,
    inputs: &mut HashMap<String, Input>,
) {
    for entry in attr_set.children() {
        if entry.kind() != SyntaxKind::NODE_ATTRPATH_VALUE {
            continue;
        }
        let Some(attrpath) = entry
            .children()
            .find(|c| c.kind() == SyntaxKind::NODE_ATTRPATH)
        else {
            continue;
        };
        let Some(value) = attrpath.next_sibling() else {
            continue;
        };

        let depth = path.len();
        path.extend(
            attrpath
                .children()
                .map(|c| Segment::from_syntax_or_sentinel(&c)),
        );
        if path.first().is_some_and(|s| s.as_str() == "inputs") {
            if value.kind() == SyntaxKind::NODE_ATTR_SET {
                collect_bindings(&value, path, inputs);
            } else {
                record_binding(inputs, path, &value);
            }
        }
        path.truncate(depth);
    }
}

/// Interpret one flattened `inputs.<id>.<rest…> = value` binding.
fn record_binding(inputs: &mut HashMap<String, Input>, path: &[Segment], value: &SyntaxNode) {
    let [_inputs, id, rest @ ..] = path else {
        return;
    };
    let Some(attr) = rest.last() else {
        return;
    };
    let input = inputs
        .entry(id.as_str().to_string())
        .or_insert_with(|| Input::new(id.clone()));

    if rest.len() == 1 {
        match attr.as_str() {
            "url" | "follows" => {
                let text = value.to_string();
                let url = strip_outer_quotes(&text);
                if !url.is_empty() {
                    input.url = url.to_string();
                    input.range = Range::from_text_range(value.text_range());
                }
            }
            "flake" if value.to_string() == "false" => input.flake = false,
            _ => {}
        }
        return;
    }

    let Some((nested, attr)) = nested_attribute(rest) else {
        return;
    };
    match attr.as_str() {
        "follows" => {
            let target = AttrPath::parse_follows_target(&value.to_string(), nested.last());
            input.push_indirect_follows(nested, target, Range::from_text_range(value.text_range()));
        }
        "inputs" => {}
        _ => input.push_override(nested),
    }
}

fn nested_attribute(rest: &[Segment]) -> Option<(AttrPath, &Segment)> {
    let (attr, chain) = rest.split_last()?;
    if chain.is_empty() || chain.len() % 2 != 0 {
        return None;
    }
    let mut names = chain.chunks_exact(2).map(|pair| match pair {
        [keyword, name] if keyword.as_str() == "inputs" => Some(name.clone()),
        _ => None,
    });
    let mut nested = AttrPath::new(names.next()??);
    for name in names {
        nested.push(name?);
    }
    Some((nested, attr))
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    use crate::change::Change;
    use crate::input::{Follows, Input};
    use crate::walk::Walker;

    fn record(flake: &str) -> HashMap<String, Input> {
        let mut walker = Walker::new(flake);
        walker.walk(&Change::None).expect("walk succeeds");
        walker.inputs
    }

    /// `(nested path, target)` of every follows on `id`, `""` for an empty target.
    fn follows(inputs: &HashMap<String, Input>, id: &str) -> Vec<(String, String)> {
        let input = inputs
            .get(id)
            .unwrap_or_else(|| panic!("{id} missing from {inputs:?}"));
        input
            .follows()
            .iter()
            .map(|f| match f {
                Follows::Indirect { path, target, .. } => (
                    path.to_string(),
                    target.as_ref().map(|t| t.to_string()).unwrap_or_default(),
                ),
                Follows::Direct(name, _) => panic!("unexpected direct follows {name}"),
            })
            .collect()
    }

    fn pairs(expected: &[(&str, &str)]) -> Vec<(String, String)> {
        expected
            .iter()
            .map(|(p, t)| (p.to_string(), t.to_string()))
            .collect()
    }

    #[test]
    fn follows_nested_in_attrset_valued_inputs_entry() {
        let inputs = record(
            r#"{
  inputs = {
    flake-parts.url = "github:hercules-ci/flake-parts";
    nixpkgs-lib.url = "github:nix-community/nixpkgs.lib";
    atuin = {
      url = "github:atuinsh/atuin";
      inputs = {
        flake-parts = {
          follows = "flake-parts";
          inputs.nixpkgs-lib.follows = "nixpkgs-lib";
        };
      };
    };
  };
  outputs = { self, ... }: { };
}"#,
        );
        assert_eq!(
            follows(&inputs, "atuin"),
            pairs(&[
                ("flake-parts", "flake-parts"),
                ("flake-parts.nixpkgs-lib", "nixpkgs-lib"),
            ])
        );
    }

    #[test]
    fn follows_in_inputs_block_before_url() {
        let inputs = record(
            r#"{
  inputs = {
    a = {
      inputs = { t = { follows = "x"; }; };
      url = "github:o/a";
    };
    t.url = "github:o/t";
  };
  outputs = _: { };
}"#,
        );
        assert_eq!(follows(&inputs, "a"), pairs(&[("t", "x")]));
        assert_eq!(inputs["a"].url(), "github:o/a");
    }

    #[test]
    fn follows_in_nested_inputs_attrset_of_inputs_block() {
        let inputs = record(
            r#"{
  inputs = {
    a = {
      url = "github:o/a";
      inputs = {
        n.inputs = { t.follows = ""; };
      };
    };
    t.url = "github:o/t";
  };
  outputs = _: { };
}"#,
        );
        assert_eq!(follows(&inputs, "a"), pairs(&[("n.t", "")]));
    }

    #[test]
    fn input_declared_without_url_is_recorded() {
        let inputs = record(
            r#"{ inputs = { nixpkgs.url = "github:nixos/nixpkgs";
    disko = { type = "github"; owner = "nix-community"; repo = "disko"; };
    other.flake = false; }; outputs = _: { }; }"#,
        );
        let disko = inputs.get("disko").expect("disko is recorded");
        assert_eq!(disko.url(), "");
        assert!(disko.flake);
        assert!(!inputs["other"].flake);
    }

    #[test]
    fn depth_two_follows_in_inputs_block() {
        let inputs = record(
            r#"{ inputs = { g.url = "github:o/g";
    a = { url = "github:o/a"; inputs = { x.inputs.g.follows = "g"; }; };
    b.url = "github:o/b"; b.inputs.x.inputs.g.follows = "g"; };
  outputs = _: { }; }"#,
        );
        assert_eq!(follows(&inputs, "a"), pairs(&[("x.g", "g")]));
        assert_eq!(follows(&inputs, "b"), pairs(&[("x.g", "g")]));
    }

    #[test]
    fn flat_urls_and_depth_one_follows_in_inputs_block() {
        let inputs = record(
            r#"{
  inputs = {
    nixpkgs.url = "github:NixOS/nixpkgs/nixos-unstable";
    flake-edit.url = "github:a-kenji/flake-edit";
    flake-edit.inputs.nixpkgs.follows = "nixpkgs";
  };

  outputs = { self, ... }: { };
}"#,
        );
        assert_eq!(
            inputs["nixpkgs"].url(),
            "github:NixOS/nixpkgs/nixos-unstable"
        );
        assert_eq!(inputs["flake-edit"].url(), "github:a-kenji/flake-edit");
        assert_eq!(
            follows(&inputs, "flake-edit"),
            pairs(&[("nixpkgs", "nixpkgs")])
        );
    }

    #[test]
    fn nested_source_overrides_are_recorded_apart_from_follows() {
        let inputs = record(
            r#"{
  inputs = {
    atuin = {
      url = "github:atuinsh/atuin";
      inputs.flake-parts.flake = false;
      inputs = {
        crane.url = "github:ipetkov/crane";
        crane.inputs.nixpkgs.follows = "nixpkgs";
      };
    };
  };
  outputs = _: { };
}"#,
        );
        let overrides: Vec<String> = inputs["atuin"]
            .overrides
            .iter()
            .map(|p| p.to_string())
            .collect();
        assert_eq!(overrides, ["crane", "flake-parts"]);
        assert_eq!(
            follows(&inputs, "atuin"),
            pairs(&[("crane.nixpkgs", "nixpkgs")])
        );
    }

    #[test]
    fn follows_range_covers_the_declared_value_in_every_spelling() {
        let flake = r#"{
  inputs = {
    nixpkgs.url = "github:NixOS/nixpkgs/nixos-unstable";
    crane.url = "github:ipetkov/crane";
    crane.inputs.nixpkgs.follows = "nixpkgs";
    atuin = {
      url = "github:atuinsh/atuin";
      inputs.flake-utils.follows = "flake-utils";
      inputs = {
        rust-overlay.inputs.nixpkgs.follows = "nixpkgs";
      };
    };
  };
  outputs = _: { };
}"#;
        let inputs = record(flake);
        let values: Vec<&str> = ["crane", "atuin"]
            .iter()
            .flat_map(|id| inputs[*id].follows())
            .map(|f| match f {
                Follows::Indirect { range, .. } => &flake[range.start..range.end],
                Follows::Direct(name, _) => panic!("unexpected direct follows {name}"),
            })
            .collect();
        assert_eq!(values, [r#""nixpkgs""#, r#""flake-utils""#, r#""nixpkgs""#]);
    }
}
