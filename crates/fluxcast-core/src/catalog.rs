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
    Action,
}

pub struct Entry {
    pub name: String,
    pub kind: Kind,
    pub path: PathBuf,
    searchable: Utf32String,
    keywords: Option<Utf32String>,
}

impl Entry {
    pub fn new(name: String, kind: Kind, path: PathBuf) -> Self {
        let searchable = Utf32String::from(name.as_str());
        Self {
            name,
            kind,
            path,
            searchable,
            keywords: None,
        }
    }

    pub fn with_keywords(name: String, kind: Kind, path: PathBuf, keywords: String) -> Self {
        let mut entry = Self::new(name, kind, path);
        let trimmed = keywords.trim();
        if !trimmed.is_empty() {
            entry.keywords = Some(Utf32String::from(trimmed));
        }
        entry
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
        let query = query.trim();
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
                let (by_keyword_only, score) =
                    if let Some(score) = pattern.score(entry.searchable.slice(..), &mut matcher) {
                        (false, score)
                    } else if let Some(keywords) = &entry.keywords {
                        (true, pattern.score(keywords.slice(..), &mut matcher)?)
                    } else {
                        return None;
                    };
                Some((
                    !entry.name.eq_ignore_ascii_case(query),
                    by_keyword_only,
                    Reverse(score),
                    id,
                ))
            })
            .collect();
        if matches.len() > limit {
            matches.select_nth_unstable(limit);
            matches.truncate(limit);
        }
        matches.sort_unstable();
        matches.into_iter().map(|(_, _, _, id)| id).collect()
    }
}
