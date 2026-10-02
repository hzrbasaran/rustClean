//! Finding entries by name below a directory.

use crate::tree::{NodeId, Tree};

/// A case-insensitive name pattern. `*` matches any run of characters and
/// `?` a single character; without wildcards the whole name must match.
pub struct Pattern {
    chars: Vec<char>,
}

impl Pattern {
    pub fn new(pattern: &str) -> Self {
        Self {
            chars: pattern.trim().to_lowercase().chars().collect(),
        }
    }

    pub fn is_empty(&self) -> bool {
        self.chars.is_empty()
    }

    pub fn matches(&self, name: &str) -> bool {
        let name: Vec<char> = name.to_lowercase().chars().collect();
        glob(&self.chars, &name)
    }
}

/// Iterative wildcard match with single-star backtracking: linear in practice.
fn glob(pat: &[char], s: &[char]) -> bool {
    let (mut p, mut i) = (0, 0);
    let mut star: Option<(usize, usize)> = None;
    while i < s.len() {
        if p < pat.len() && (pat[p] == '?' || pat[p] == s[i]) {
            p += 1;
            i += 1;
        } else if p < pat.len() && pat[p] == '*' {
            star = Some((p, i));
            p += 1;
        } else if let Some((sp, si)) = star {
            p = sp + 1;
            i = si + 1;
            star = Some((sp, si + 1));
        } else {
            return false;
        }
    }
    pat[p..].iter().all(|&c| c == '*')
}

/// Entries below `base` whose name matches. A matching directory's contents
/// are not searched: removing it removes them too, and listing both would
/// count their size twice.
pub fn find(tree: &Tree, base: NodeId, pattern: &Pattern) -> Vec<NodeId> {
    let mut found = Vec::new();
    if pattern.is_empty() {
        return found;
    }
    let mut stack: Vec<NodeId> = tree.children(base).collect();
    while let Some(id) = stack.pop() {
        if pattern.matches(tree.name(id)) {
            found.push(id);
        } else if tree.node(id).is_dir {
            stack.extend(tree.children(id));
        }
    }
    found
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tree::{Size, ROOT};
    use std::path::Path;

    fn m(pattern: &str, name: &str) -> bool {
        Pattern::new(pattern).matches(name)
    }

    #[test]
    fn plain_text_matches_whole_name() {
        assert!(m("deneme", "deneme"));
        assert!(m("deneme", "Deneme"));
        assert!(m("DENEME", "deneme"));
        assert!(!m("deneme", "deneme2"));
        assert!(!m("deneme", "eski-deneme"));
    }

    #[test]
    fn wildcards() {
        assert!(m("deneme*", "deneme"));
        assert!(m("deneme*", "denemeler"));
        assert!(m("*.log", "app.log"));
        assert!(!m("*.log", "app.log.gz"));
        assert!(m("*den*", "eski-deneme"));
        assert!(m("test?", "test1"));
        assert!(!m("test?", "test"));
        assert!(m("a*b*c", "a-x-b-y-c"));
        assert!(!m("a*b*c", "a-x-c-y-b"));
        assert!(m("*", "anything"));
        assert!(m("ğüş*", "ĞÜŞçö"));
    }

    #[test]
    fn find_skips_inside_matches() {
        let mut t = Tree::new(Path::new("/p"));
        let s = Size::default();
        let a = t.push(ROOT, "proje-a", true, s);
        let d1 = t.push(a, "deneme", true, s);
        t.push(d1, "deneme", true, s); // nested, covered by d1
        let b = t.push(ROOT, "proje-b", true, s);
        let src = t.push(b, "src", true, s);
        let d2 = t.push(src, "Deneme", true, s);
        t.push(b, "deneme.txt", false, s);

        let mut found = find(&t, ROOT, &Pattern::new("deneme"));
        found.sort();
        assert_eq!(found, vec![d1, d2]);

        // Searching only below proje-b.
        assert_eq!(find(&t, b, &Pattern::new("deneme")), vec![d2]);
        assert_eq!(find(&t, ROOT, &Pattern::new("   ")), Vec::<NodeId>::new());
    }
}
