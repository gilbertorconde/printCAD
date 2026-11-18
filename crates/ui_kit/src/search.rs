//! Finding a typed query among labels, as the search fields rank them.

/// How well `haystack` matches `needle`, both already lowercase: 0 when
/// it holds `needle` whole, 1 when it holds its characters in order with
/// others between, `None` when it does not. An empty needle matches all.
pub fn match_rank(haystack: &str, needle: &str) -> Option<u8> {
    if needle.is_empty() || haystack.contains(needle) {
        return Some(0);
    }
    let mut chars = needle.chars();
    let mut want = chars.next();
    for c in haystack.chars() {
        if Some(c) == want {
            want = chars.next();
            if want.is_none() {
                return Some(1);
            }
        }
    }
    None
}

/// The places of `options` that match `query`, ignoring case and the
/// spaces round it: whole matches first, then spread ones, each in the
/// options' order.
pub fn ranked(options: &[String], query: &str) -> Vec<usize> {
    let needle = query.trim().to_lowercase();
    let mut shown: Vec<(u8, usize)> = options
        .iter()
        .enumerate()
        .filter_map(|(i, o)| match_rank(&o.to_lowercase(), &needle).map(|r| (r, i)))
        .collect();
    shown.sort_unstable();
    shown.into_iter().map(|(_, i)| i).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn whole_matches_come_before_spread_ones() {
        let fonts: Vec<String> = [
            "DejaVu Sans Mono",
            "Noto Sans",
            "Liberation Mono",
            "Fira Code",
        ]
        .map(String::from)
        .into();
        assert_eq!(ranked(&fonts, " MONO "), [0, 2]);
        // "nsn" is in none whole; spread over Noto Sans and DejaVu Sans Mono.
        assert_eq!(ranked(&fonts, "nsn"), [0, 1]);
        assert_eq!(ranked(&fonts, ""), [0, 1, 2, 3]);
        assert!(ranked(&fonts, "xyz").is_empty());
    }
}
