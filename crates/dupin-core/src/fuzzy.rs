//! Subsequence matching for short names (app names, file names), ranked the way launchers
//! are expected to rank: a prefix beats a word start, which beats a run in the middle, which
//! beats letters scattered across the name.

/// A match of a query against a name: higher `score` ranks first, and `positions` are the
/// character indices in the name that matched, in order.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Match {
    pub score: i32,
    pub positions: Vec<usize>,
}

const MATCH: i32 = 16;
const PREFIX: i32 = 64;
const WORD_START: i32 = 32;
const CONSECUTIVE: i32 = 24;
const GAP: i32 = 3;
const GAP_START: i32 = 6;
/// Shorter names win ties: "Safari" over "Safari Technology Preview" for "saf".
const LENGTH: i32 = 1;

/// Matches `query` against `name`, ignoring case and whitespace in the query.
/// None when the query's characters don't all appear in order.
pub fn score(query: &str, name: &str) -> Option<Match> {
    let query: Vec<char> = query
        .chars()
        .filter(|c| !c.is_whitespace())
        .flat_map(char::to_lowercase)
        .collect();
    if query.is_empty() {
        return None;
    }
    let chars: Vec<char> = name.chars().collect();
    let lower: Vec<char> = chars
        .iter()
        .map(|c| c.to_lowercase().next().unwrap_or(*c))
        .collect();
    if !is_subsequence(&query, &lower) {
        return None;
    }

    // best[j]: the best score with the query so far matched and its last character at name
    // index j; from[i][j] remembers the previous position for reading the path back. Names are
    // short, so the quadratic inner search is cheaper than being clever.
    let n = chars.len();
    let mut best: Vec<Option<i32>> = vec![None; n];
    let mut from: Vec<Vec<usize>> = Vec::with_capacity(query.len());
    for (i, &q) in query.iter().enumerate() {
        let mut next: Vec<Option<i32>> = vec![None; n];
        let mut back = vec![usize::MAX; n];
        for j in 0..n {
            if lower[j] != q {
                continue;
            }
            let bonus = MATCH + position_bonus(&chars, j);
            if i == 0 {
                let lead = if j == 0 {
                    PREFIX
                } else {
                    -GAP_START - (j as i32).min(12)
                };
                next[j] = Some(bonus + lead);
                continue;
            }
            let step = |k: usize| {
                if k + 1 == j {
                    CONSECUTIVE
                } else {
                    -GAP_START - GAP * (j - k - 1) as i32
                }
            };
            if let Some((s, k)) = (0..j)
                .filter_map(|k| best[k].map(|s| (s + step(k), k)))
                .max_by_key(|&(s, _)| s)
            {
                next[j] = Some(s + bonus);
                back[j] = k;
            }
        }
        best = next;
        from.push(back);
    }

    let (mut j, mut total) = best
        .iter()
        .enumerate()
        .filter_map(|(j, s)| s.map(|s| (j, s)))
        .max_by_key(|&(j, s)| (s, std::cmp::Reverse(j)))?;
    total -= LENGTH * n as i32;
    let mut positions = vec![0; query.len()];
    for i in (0..query.len()).rev() {
        positions[i] = j;
        if i > 0 {
            j = from[i][j];
        }
    }
    Some(Match {
        score: total,
        positions,
    })
}

fn is_subsequence(query: &[char], name: &[char]) -> bool {
    let mut it = name.iter();
    query.iter().all(|q| it.any(|c| c == q))
}

/// Word starts: after a separator, or a capital after a lower-case letter ("FaceTime").
fn position_bonus(chars: &[char], j: usize) -> i32 {
    if j == 0 {
        return WORD_START;
    }
    let (prev, here) = (chars[j - 1], chars[j]);
    if !prev.is_alphanumeric()
        || (prev.is_lowercase() && here.is_uppercase())
        || (!prev.is_numeric() && here.is_numeric())
    {
        WORD_START
    } else {
        0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rank<'a>(query: &str, names: &[&'a str]) -> Vec<&'a str> {
        let mut hits: Vec<(i32, &str)> = names
            .iter()
            .filter_map(|n| score(query, n).map(|m| (m.score, *n)))
            .collect();
        hits.sort_by_key(|a| std::cmp::Reverse(a.0));
        hits.into_iter().map(|(_, n)| n).collect()
    }

    #[test]
    fn requires_every_character_in_order() {
        assert!(score("saf", "Safari").is_some());
        assert!(score("fas", "Safari").is_none());
        assert!(score("", "Safari").is_none());
        assert!(score("xyz", "Safari").is_none());
    }

    #[test]
    fn ignores_case_and_query_whitespace() {
        assert_eq!(score("SAF", "safari").unwrap().positions, vec![0, 1, 2]);
        assert_eq!(
            score("sys set", "System Settings").unwrap().positions,
            vec![0, 1, 2, 7, 8, 9]
        );
    }

    #[test]
    fn prefix_beats_middle_and_shorter_wins_ties() {
        assert_eq!(
            rank("saf", &["Safari Technology Preview", "Safari", "Hush Safe"]),
            ["Safari", "Safari Technology Preview", "Hush Safe"]
        );
    }

    #[test]
    fn word_starts_beat_scattered_letters() {
        // "vsc": V(isual) S(tudio) C(ode) against letters buried inside words.
        assert_eq!(
            rank("vsc", &["Visual Studio Code", "Avs Cleaner"])[0],
            "Visual Studio Code"
        );
        assert_eq!(score("ft", "FaceTime").unwrap().positions, vec![0, 4]);
    }

    #[test]
    fn consecutive_run_beats_split_letters() {
        assert_eq!(score("term", "Terminal").unwrap().positions, vec![0, 1, 2, 3]);
        assert_eq!(rank("note", &["Notes", "No Tears Edition"])[0], "Notes");
    }

    #[test]
    fn positions_index_characters_not_bytes() {
        assert_eq!(score("ré", "Aperçu Résumé").unwrap().positions, vec![7, 8]);
    }
}
