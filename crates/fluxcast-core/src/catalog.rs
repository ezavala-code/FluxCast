use nucleo_matcher::{
    pattern::{AtomKind, CaseMatching, Normalization, Pattern},
    Config, Matcher, Utf32String,
};
use serde::{Deserialize, Serialize};
use std::{cmp::Reverse, path::PathBuf};

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Kind {
    App,
    Project,
}

pub struct Entry {
    pub name: String,
    pub kind: Kind,
    pub path: PathBuf,
    searchable: Utf32String,
}

impl Entry {
    pub fn new(name: String, kind: Kind, path: PathBuf) -> Self {
        let searchable = Utf32String::from(name.as_str());
        Self {
            name,
            kind,
            path,
            searchable,
        }
    }
}

pub struct Catalog(Vec<Entry>);

impl Catalog {
    pub fn new(mut entries: Vec<Entry>) -> Self {
        entries.sort_by_cached_key(|e| (e.name.to_lowercase(), e.path.clone()));
        Self(entries)
    }

    pub fn get(&self, id: usize) -> Option<&Entry> {
        self.0.get(id)
    }

    /// IDs permanecen estables durante la vida del catálogo.
    pub fn search(&self, query: &str, kind: Option<Kind>, limit: usize) -> Vec<usize> {
        let candidates = self
            .0
            .iter()
            .enumerate()
            .filter(|(_, entry)| kind.is_none_or(|k| k == entry.kind));
        if query.is_empty() {
            return candidates.map(|(id, _)| id).take(limit).collect();
        }
        let pattern = Pattern::new(
            query,
            CaseMatching::Ignore,
            Normalization::Smart,
            AtomKind::Fuzzy,
        );
        let mut matcher = Matcher::new(Config::DEFAULT);
        let mut matches: Vec<_> = candidates
            .filter_map(|(id, entry)| {
                let score = pattern.score(entry.searchable.slice(..), &mut matcher)?;
                Some((!entry.name.eq_ignore_ascii_case(query), Reverse(score), id))
            })
            .collect();
        if matches.len() > limit {
            matches.select_nth_unstable(limit);
            matches.truncate(limit);
        }
        matches.sort_unstable();
        matches.into_iter().map(|(_, _, id)| id).collect()
    }
}
