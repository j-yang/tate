//! Version control kernel for structured data.
//!
//! A [`Repo`] stores versions (as [`Section`](crate::section::Section)s) in a
//! content-addressed map and tracks their lineage in a commit DAG. Merge is
//! computed via [`patch::merge_sections`] — the field-wise pushout.
//!
//! ```
//! use tate::tree::TreeNode;
//! use tate::repo::Repo;
//!
//! let mut repo = Repo::new();
//!
//! let v0 = repo.commit("initial", &[], &TreeNode::new("root")
//!     .with_child(TreeNode::new("server").with_identity("s1")
//!         .with_attr("port", "8080"))
//!     .with_child(TreeNode::new("db").with_identity("d1")
//!         .with_attr("url", "localhost")));
//!
//! let v1 = repo.commit("port -> 9090", &[v0], &TreeNode::new("root")
//!     .with_child(TreeNode::new("server").with_identity("s1")
//!         .with_attr("port", "9090"))
//!     .with_child(TreeNode::new("db").with_identity("d1")
//!         .with_attr("url", "localhost")));
//!
//! let v2 = repo.commit("url -> prod", &[v0], &TreeNode::new("root")
//!     .with_child(TreeNode::new("server").with_identity("s1")
//!         .with_attr("port", "8080"))
//!     .with_child(TreeNode::new("db").with_identity("d1")
//!         .with_attr("url", "prod.example.com")));
//!
//! let result = repo.merge(v1, v2);
//! assert!(result.is_clean());
//! ```

use crate::patch::{self, merge_sections, ApplyError, Patch, SectionConflict};
use crate::section::Section;
use crate::tree::TreeNode;
use std::collections::{HashMap, HashSet};

pub type Hash = u64;

#[derive(Debug, Clone)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct Commit {
    pub parents: Vec<Hash>,
    pub version: Hash,
    pub message: String,
    pub timestamp: u64,
}

pub struct Repo {
    sections: HashMap<Hash, Section>,
    commits: HashMap<Hash, Commit>,
    branches: HashMap<String, Hash>,
    head: Option<Hash>,
}

pub struct MergeResult {
    pub merged_section: Hash,
    pub conflicts: Vec<SectionConflict>,
    pub base: Hash,
    pub ours: Hash,
    pub theirs: Hash,
}

impl MergeResult {
    pub fn is_clean(&self) -> bool { self.conflicts.is_empty() }
}

impl Repo {
    pub fn new() -> Self {
        Repo { sections: HashMap::new(), commits: HashMap::new(), branches: HashMap::new(), head: None }
    }

    fn store_section(&mut self, section: Section) -> Hash {
        let h = hash_section(&section);
        self.sections.entry(h).or_insert(section);
        h
    }

    pub fn commit(&mut self, message: &str, parents: &[Hash], tree: &TreeNode) -> Hash {
        self.commit_section(message, parents, tree.to_section())
    }

    pub fn commit_section(&mut self, message: &str, parents: &[Hash], section: Section) -> Hash {
        let version = self.store_section(section);
        let commit = Commit {
            parents: parents.to_vec(), version,
            message: message.to_string(), timestamp: now_unix(),
        };
        let ch = hash_commit(&commit);
        self.commits.insert(ch, commit);
        self.head = Some(ch);
        ch
    }

    pub fn section(&self, commit: Hash) -> &Section {
        &self.sections[&self.commits[&commit].version]
    }

    pub fn tree(&self, commit: Hash) -> TreeNode {
        self.section(commit).to_tree().unwrap_or_default()
    }

    pub fn section_as_tree(&self, section_hash: Hash) -> TreeNode {
        self.sections.get(&section_hash)
            .and_then(|s| s.to_tree())
            .unwrap_or_default()
    }

    pub fn diff(&self, a: Hash, b: Hash) -> Patch {
        patch::diff_sections(self.section(a), self.section(b))
    }

    pub fn merge(&mut self, ours: Hash, theirs: Hash) -> MergeResult {
        let base = self.merge_base(ours, theirs);
        let result = merge_sections(self.section(base), self.section(ours), self.section(theirs));
        let merged_section = self.store_section(result.merged);
        MergeResult { merged_section, conflicts: result.conflicts, base, ours, theirs }
    }

    pub fn cherry_pick(&mut self, src: Hash, dst: Hash) -> Result<Hash, ApplyError> {
        let parent = self.commits[&src].parents.first().copied();
        let p = match parent {
            Some(p) => self.diff(p, src),
            None => patch::diff_sections(&Section::new(), self.section(src)),
        };
        match patch::apply_to_section(&p, self.section(dst)) {
            Ok(merged) => Ok(self.store_section(merged)),
            Err(e) => Err(*e),
        }
    }

    pub fn revert(&mut self, target: Hash, dst: Hash) -> Result<Hash, ApplyError> {
        let parent = self.commits[&target].parents.first().copied();
        let p = match parent {
            Some(p) => self.diff(p, target),
            None => patch::diff_sections(self.section(target), &Section::new()),
        };
        let inv = patch::invert(&p);
        match patch::apply_to_section(&inv, self.section(dst)) {
            Ok(merged) => Ok(self.store_section(merged)),
            Err(e) => Err(*e),
        }
    }

    pub fn merge_base(&self, a: Hash, b: Hash) -> Hash {
        if a == b { return a; }
        let anc_a = self.all_ancestors(a);
        let mut queue = vec![b];
        let mut visited = HashSet::new();
        let mut best: Option<(Hash, usize)> = None;
        while let Some(c) = queue.pop() {
            if !visited.insert(c) { continue; }
            if c != a && c != b && anc_a.contains(&c) {
                let depth = self.depth(c);
                match best {
                    None => best = Some((c, depth)),
                    Some((_, bd)) if depth < bd => best = Some((c, depth)),
                    _ => {}
                }
            }
            if let Some(commit) = self.commits.get(&c) {
                for &p in &commit.parents { queue.push(p); }
            }
        }
        best.map(|(h, _)| h)
            .unwrap_or_else(|| if anc_a.contains(&b) { b } else { a })
    }

    fn all_ancestors(&self, commit: Hash) -> HashSet<Hash> {
        let mut set = HashSet::new();
        let mut stack = vec![commit];
        while let Some(c) = stack.pop() {
            if set.insert(c) {
                if let Some(commit) = self.commits.get(&c) {
                    stack.extend(&commit.parents);
                }
            }
        }
        set
    }

    fn depth(&self, commit: Hash) -> usize {
        let mut max = 0;
        let mut visited = HashSet::new();
        let mut stack = vec![(commit, 0usize)];
        while let Some((c, d)) = stack.pop() {
            if !visited.insert(c) { continue; }
            max = max.max(d);
            if let Some(commit) = self.commits.get(&c) {
                for &p in &commit.parents { stack.push((p, d + 1)); }
            }
        }
        max
    }

    pub fn log(&self, from: Option<Hash>) -> Vec<(Hash, &Commit)> {
        let start = match from.or(self.head) { Some(h) => h, None => return Vec::new() };
        let mut visited = HashSet::new();
        let mut result = Vec::new();
        let mut stack = vec![start];
        while let Some(c) = stack.pop() {
            if !visited.insert(c) { continue; }
            if let Some(commit) = self.commits.get(&c) {
                result.push((c, commit));
                stack.extend(&commit.parents);
            }
        }
        result
    }

    pub fn branch(&mut self, name: &str, commit: Hash) { self.branches.insert(name.to_string(), commit); }
    pub fn branch_head(&self, name: &str) -> Option<Hash> { self.branches.get(name).copied() }
    pub fn branches(&self) -> &HashMap<String, Hash> { &self.branches }
    pub fn head(&self) -> Option<Hash> { self.head }
    pub fn set_head(&mut self, commit: Hash) { self.head = Some(commit); }
    pub fn len(&self) -> usize { self.commits.len() }
    pub fn is_empty(&self) -> bool { self.commits.is_empty() }
}

impl Default for Repo {
    fn default() -> Self { Self::new() }
}

fn fnv1a(data: &[u8]) -> u64 {
    let mut h = 0xcbf29ce484222325u64;
    for &b in data { h ^= b as u64; h = h.wrapping_mul(0x100000001b3); }
    h
}

fn hash_section(s: &Section) -> Hash {
    let mut buf = Vec::new();
    for (id, node) in &s.nodes {
        buf.extend_from_slice(id.as_bytes());
        buf.push(b'|');
        buf.extend(format!("{:?}", node).as_bytes());
        buf.push(b'\n');
    }
    fnv1a(&buf)
}

fn hash_commit(c: &Commit) -> Hash {
    let mut buf = Vec::new();
    for &p in &c.parents { buf.extend_from_slice(&p.to_le_bytes()); }
    buf.extend_from_slice(&c.version.to_le_bytes());
    buf.extend_from_slice(c.message.as_bytes());
    buf.extend_from_slice(&c.timestamp.to_le_bytes());
    fnv1a(&buf)
}

fn now_unix() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample() -> TreeNode {
        TreeNode::new("root")
            .with_child(TreeNode::new("server").with_identity("s1").with_attr("port", "8080"))
            .with_child(TreeNode::new("db").with_identity("d1").with_attr("url", "localhost"))
    }

    #[test]
    fn commit_and_log() {
        let mut repo = Repo::new();
        let v0 = repo.commit("initial", &[], &sample());
        assert_eq!(repo.len(), 1);
        assert_eq!(repo.log(None).len(), 1);
    }

    #[test]
    fn diff_two_commits() {
        let mut repo = Repo::new();
        let v0 = repo.commit("v0", &[], &sample());
        let mut modified = sample();
        modified.children[0].attributes[0].1 = "9090".into();
        let v1 = repo.commit("port change", &[v0], &modified);
        assert!(!repo.diff(v0, v1).is_empty());
    }

    #[test]
    fn clean_merge_disjoint_nodes() {
        let mut repo = Repo::new();
        let v0 = repo.commit("base", &[], &sample());

        let mut ours = sample();
        ours.children[0].attributes[0].1 = "9090".into();
        let v1 = repo.commit("port", &[v0], &ours);

        let mut theirs = sample();
        theirs.children[1].attributes[0].1 = "prod".into();
        let v2 = repo.commit("url", &[v0], &theirs);

        let result = repo.merge(v1, v2);
        assert!(result.is_clean());

        let section = &repo.sections[&result.merged_section];
        let server = section.nodes.get("s1").unwrap();
        assert_eq!(server.attrs.iter().find(|(k, _)| k == "port").unwrap().1, "9090");
        let db = section.nodes.get("d1").unwrap();
        assert_eq!(db.attrs.iter().find(|(k, _)| k == "url").unwrap().1, "prod");
    }

    #[test]
    fn conflict_merge_same_field() {
        let mut repo = Repo::new();
        let v0 = repo.commit("base", &[], &sample());

        let mut ours = sample();
        ours.children[0].attributes[0].1 = "9090".into();
        let v1 = repo.commit("9090", &[v0], &ours);

        let mut theirs = sample();
        theirs.children[0].attributes[0].1 = "3000".into();
        let v2 = repo.commit("3000", &[v0], &theirs);

        let result = repo.merge(v1, v2);
        assert!(!result.is_clean());
        assert_eq!(result.conflicts.len(), 1);
    }

    #[test]
    fn move_plus_modify_merges_clean() {
        let mut repo = Repo::new();
        let v0 = repo.commit("base", &[], &sample());

        // Alice: move s1 under d1.
        let mut moved = sample();
        let s1 = moved.children.remove(0);
        moved.children[0].children.push(s1);
        let v1 = repo.commit("move s1 -> d1", &[v0], &moved);

        // Bob: modify s1's port.
        let mut modified = sample();
        modified.children[0].attributes[0].1 = "9090".into();
        let v2 = repo.commit("port -> 9090", &[v0], &modified);

        let result = repo.merge(v1, v2);
        assert!(result.is_clean(), "move + modify must merge cleanly");

        let section = &repo.sections[&result.merged_section];
        let s1 = section.nodes.get("s1").unwrap();
        assert_eq!(s1.parent.as_deref(), Some("d1"), "s1 moved to d1");
        assert_eq!(s1.attrs.iter().find(|(k, _)| k == "port").unwrap().1, "9090", "port modified");
    }

    #[test]
    fn cherry_pick_works() {
        let mut repo = Repo::new();
        let v0 = repo.commit("base", &[], &sample());
        let mut changed = sample();
        changed.children[0].attributes[0].1 = "9090".into();
        let v1 = repo.commit("port", &[v0], &changed);
        let mut other = sample();
        other.children[1].attributes[0].1 = "staging".into();
        let v2 = repo.commit("staging", &[v0], &other);
        let picked = repo.cherry_pick(v1, v2).expect("cherry-pick should apply");
        let section = &repo.sections[&picked];
        assert_eq!(section.nodes.get("s1").unwrap().attrs.iter().find(|(k, _)| k == "port").unwrap().1, "9090");
        assert_eq!(section.nodes.get("d1").unwrap().attrs.iter().find(|(k, _)| k == "url").unwrap().1, "staging");
    }

    #[test]
    fn revert_works() {
        let mut repo = Repo::new();
        let v0 = repo.commit("base", &[], &sample());
        let mut changed = sample();
        changed.children[0].attributes[0].1 = "9090".into();
        let v1 = repo.commit("port change", &[v0], &changed);
        let reverted = repo.revert(v1, v1).expect("revert should apply");
        let section = &repo.sections[&reverted];
        assert_eq!(section.nodes.get("s1").unwrap().attrs.iter().find(|(k, _)| k == "port").unwrap().1, "8080");
    }

    #[test]
    fn branches() {
        let mut repo = Repo::new();
        let v0 = repo.commit("initial", &[], &sample());
        let v1 = repo.commit("second", &[v0], &sample());
        repo.branch("main", v0);
        repo.branch("dev", v1);
        assert_eq!(repo.branch_head("main"), Some(v0));
        assert_eq!(repo.branch_head("dev"), Some(v1));
    }

    #[test]
    fn merge_base_is_common_ancestor() {
        let mut repo = Repo::new();
        let v0 = repo.commit("root", &[], &sample());
        let v1 = repo.commit("a", &[v0], &sample());
        let v2 = repo.commit("b", &[v0], &sample());
        let v3 = repo.commit("c", &[v1], &sample());
        assert_eq!(repo.merge_base(v3, v2), v0);
    }

    #[test]
    fn identical_branches_merge_clean() {
        let mut repo = Repo::new();
        let v0 = repo.commit("base", &[], &sample());
        let v1 = repo.commit("same", &[v0], &sample());
        let v2 = repo.commit("same", &[v0], &sample());
        assert!(repo.merge(v1, v2).is_clean());
    }
}
