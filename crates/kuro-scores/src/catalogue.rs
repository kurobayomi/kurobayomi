//! The titles a provider lists in bulk (`ScoreProvider::catalogue`), indexed by name. A Browse page shows titles the
//! user has not opened yet; asking the site for each cover would take a request and a wait per cover, so a cover is
//! first matched here, against lists fetched a few requests at a time. A title the lists do not hold is not found here,
//! and is asked by its own request.

use std::collections::HashMap;

use kuro_track::auto_bind::{loose, normalize};

use crate::provider::Hit;

/// The listed titles of one kind, ready to be found by name.
#[derive(Debug, Default)]
pub struct Catalogue {
    hits: Vec<Hit>,
    /// The titles of each name, by the name as the match compares it ([`normalize`]): a title's own name and its others.
    by_name: HashMap<String, Vec<usize>>,
    /// The same by the match's second reading ([`loose`]: a plural `s` aside).
    by_loose: HashMap<String, Vec<usize>>,
    by_id: HashMap<i64, usize>,
}

impl Catalogue {
    pub fn new(hits: Vec<Hit>) -> Self {
        let (mut by_name, mut by_loose): (HashMap<String, Vec<usize>>, HashMap<String, Vec<usize>>) = Default::default();
        for (index, hit) in hits.iter().enumerate() {
            for name in std::iter::once(&hit.title).chain(&hit.alt_titles) {
                for (map, key) in [(&mut by_name, normalize(name)), (&mut by_loose, loose(name))] {
                    if key.is_empty() {
                        continue;
                    }
                    let of_name = map.entry(key).or_default();
                    if !of_name.contains(&index) {
                        of_name.push(index);
                    }
                }
            }
        }
        let by_id = hits.iter().enumerate().map(|(index, hit)| (hit.remote_id, index)).collect();
        Self { hits, by_name, by_loose, by_id }
    }

    /// The listed title of this id on the site.
    pub fn by_id(&self, remote_id: i64) -> Option<&Hit> {
        self.hits.get(*self.by_id.get(&remote_id)?)
    }

    pub fn len(&self) -> usize {
        self.hits.len()
    }

    pub fn is_empty(&self) -> bool {
        self.hits.is_empty()
    }

    /// Every listed title, in the order the provider gave them.
    pub fn hits(&self) -> &[Hit] {
        &self.hits
    }

    /// The one listed title of this name; when none has it exactly, the one equal but for a plural `s`. `None` when no
    /// title matches, or more than one does: the rule of `kuro_track::auto_bind::pick`, so a match here is as exact as a
    /// match by search.
    pub fn find(&self, title: &str) -> Option<&Hit> {
        let index = match self.by_name.get(&normalize(title)).map(Vec::as_slice) {
            Some([index]) => *index,
            Some([_, _, ..]) => return None,
            _ => match self.by_loose.get(&loose(title))?.as_slice() {
                [index] => *index,
                _ => return None,
            },
        };
        self.hits.get(index)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn hit(id: i64, title: &str, others: &[&str], score: Option<f64>) -> Hit {
        Hit { remote_id: id, title: title.into(), alt_titles: others.iter().map(|name| (*name).to_owned()).collect(), score, slot: None }
    }

    #[test]
    fn a_title_is_found_by_its_name_or_one_of_its_other_names_as_the_match_compares_them() {
        let catalogue = Catalogue::new(vec![
            hit(52034, "[Oshi no Ko]", &["My Star"], Some(8.52)),
            hit(55791, "[Oshi no Ko] 2nd Season", &[], Some(8.49)),
            hit(1, "Cowboy Bebop", &["カウボーイビバップ"], Some(8.75)),
        ]);
        assert_eq!(catalogue.len(), 3);
        assert_eq!(catalogue.by_id(55791).map(|hit| hit.title.as_str()), Some("[Oshi no Ko] 2nd Season"));
        assert_eq!(catalogue.by_id(2).map(|hit| hit.remote_id), None, "an id the lists do not hold");
        assert_eq!(catalogue.find("\"Oshi no Ko\"").map(|hit| hit.remote_id), Some(52034), "the punctuation of a source's name does not count");
        assert_eq!(catalogue.find("Oshi no Ko Season 2").map(|hit| hit.remote_id), Some(55791), "\"2nd Season\" reads as \"Season 2\"");
        assert_eq!(catalogue.find("my star").map(|hit| hit.remote_id), Some(52034));
        assert_eq!(catalogue.find("カウボーイビバップ").and_then(|hit| hit.score), Some(8.75));
        assert_eq!(catalogue.find("Oshi no Ko 3rd Season").map(|hit| hit.remote_id), None, "a title the lists do not hold");
        assert_eq!(catalogue.find("My Stars").map(|hit| hit.remote_id), Some(52034), "a plural s aside, when nothing is exact");
        assert_eq!(catalogue.find("!!!").map(|hit| hit.remote_id), None, "a name with nothing to compare");
    }

    #[test]
    fn two_titles_of_one_name_are_no_match_and_a_title_named_twice_is_one() {
        let catalogue = Catalogue::new(vec![
            hit(1, "Hunter x Hunter", &[], Some(8.4)),
            hit(2, "Hunter x Hunter (2011)", &["Hunter x Hunter"], Some(9.0)),
            // its own name again among its other names: still one title
            hit(3, "Naruto", &["NARUTO", "Naruto"], Some(8.0)),
        ]);
        assert_eq!(catalogue.find("Hunter x Hunter").map(|hit| hit.remote_id), None, "two titles answer to it");
        assert_eq!(catalogue.find("Hunter x Hunter (2011)").map(|hit| hit.remote_id), Some(2));
        assert_eq!(catalogue.find("naruto").map(|hit| hit.remote_id), Some(3));
        assert!(Catalogue::default().is_empty());
        assert_eq!(Catalogue::default().find("Naruto").map(|hit| hit.remote_id), None);
    }
}
