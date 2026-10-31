//! Find the children an edit spliced into a syntax tree.

use std::ops::Range;

use rnix::{SyntaxElement, SyntaxKind, SyntaxNode};
use rowan::NodeOrToken;

#[derive(Debug)]
pub(crate) struct Splice {
    parent: SyntaxNode,
    children: Range<usize>,
    before: SyntaxNode,
    before_children: Range<usize>,
}

impl Splice {
    pub(crate) fn find(before: &SyntaxNode, edited: &SyntaxNode) -> Option<Self> {
        if std::ptr::eq(before.green(), edited.green()) {
            return None;
        }
        let (mut before, mut edited) = (before.clone(), edited.clone());
        loop {
            let old: Vec<SyntaxElement> = before.children_with_tokens().collect();
            let new: Vec<SyntaxElement> = edited.children_with_tokens().collect();
            let prefix = old
                .iter()
                .zip(&new)
                .take_while(|(o, n)| same_green(o, n))
                .count();
            let suffix = old[prefix..]
                .iter()
                .rev()
                .zip(new[prefix..].iter().rev())
                .take_while(|(o, n)| same_green(o, n))
                .count();
            if old.len() == new.len()
                && new.len() - suffix == prefix + 1
                && let (NodeOrToken::Node(o), NodeOrToken::Node(n)) = (&old[prefix], &new[prefix])
                && o.kind() == n.kind()
            {
                before = o.clone();
                edited = n.clone();
                continue;
            }
            return Some(Self {
                parent: edited,
                children: prefix..new.len() - suffix,
                before,
                before_children: prefix..old.len() - suffix,
            });
        }
    }

    pub(crate) fn in_attr_set(&self) -> Option<(SyntaxNode, Range<usize>)> {
        if self.parent.kind() == SyntaxKind::NODE_ATTR_SET {
            return Some((self.parent.clone(), self.children.clone()));
        }
        let binding = self.parent.ancestors().find(|n| {
            n.parent()
                .is_some_and(|p| p.kind() == SyntaxKind::NODE_ATTR_SET)
        })?;
        let index = binding.index();
        Some((binding.parent()?, index..index + 1))
    }

    /// Offset of the first byte the edit changed, and the change in length.
    pub(crate) fn replaced_bytes(&self) -> (usize, isize) {
        let old = children_text(&self.before, &self.before_children);
        let new = children_text(&self.parent, &self.children);
        let common = old
            .bytes()
            .zip(new.bytes())
            .take_while(|(o, n)| o == n)
            .count();
        let start = match self.children.start {
            0 => self.parent.text_range().start(),
            i => self
                .parent
                .children_with_tokens()
                .nth(i - 1)
                .map_or(self.parent.text_range().start(), |e| e.text_range().end()),
        };
        let delta = new.len() as isize - old.len() as isize;
        (usize::from(start) + common, delta)
    }
}

fn same_green(a: &SyntaxElement, b: &SyntaxElement) -> bool {
    match (a, b) {
        (NodeOrToken::Node(a), NodeOrToken::Node(b)) => std::ptr::eq(a.green(), b.green()),
        (NodeOrToken::Token(a), NodeOrToken::Token(b)) => std::ptr::eq(a.green(), b.green()),
        _ => false,
    }
}

fn children_text(parent: &SyntaxNode, children: &Range<usize>) -> String {
    parent
        .children_with_tokens()
        .skip(children.start)
        .take(children.len())
        .map(|e| e.to_string())
        .collect()
}
