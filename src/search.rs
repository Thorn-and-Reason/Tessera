/// Query-sized KMP state, built once per picker frame. Names are never copied.
/// The 8192-character editor limit bounds lowercase construction. Pattern, prefix
/// table, boundary ring and temporary lowercase string share a 512 KiB sub-budget
/// of the existing 4 MiB UI reservation (no new reservation or persistent cache).
pub(crate) struct NameQuery {
    pattern: Vec<char>,
    prefix: Vec<usize>,
}
pub(crate) const SEARCH_SCRATCH_BYTES: usize = 512 * 1024;
impl NameQuery {
    pub(crate) fn new(query: &str) -> crate::error::AppResult<Self> {
        if query.chars().take(8192 + 1).count() > 8192 {
            return Err(crate::error::AppError::invalid(
                "Variable search",
                "Search is limited to 8192 characters.",
            ));
        }
        // Use the same scalar folding as the streamed name, including Greek sigma.
        // Its source is at most 32 KiB; Unicode lowercase expands a scalar at most 3x.
        let count = query.chars().flat_map(char::to_lowercase).count();
        let predicted =
            count * (4 + std::mem::size_of::<char>() + std::mem::size_of::<usize>() + 1);
        if predicted > SEARCH_SCRATCH_BYTES {
            return Err(crate::error::AppError::SizeLimit {
                limit: SEARCH_SCRATCH_BYTES,
            });
        }
        let mut folded = String::with_capacity(count * 4);
        folded.extend(query.chars().flat_map(char::to_lowercase));
        let count = folded.chars().count();
        let scratch = folded.capacity()
            + count * (std::mem::size_of::<char>() + std::mem::size_of::<usize>() + 1);
        if scratch > SEARCH_SCRATCH_BYTES {
            return Err(crate::error::AppError::SizeLimit {
                limit: SEARCH_SCRATCH_BYTES,
            });
        }
        let mut pattern = Vec::with_capacity(count);
        pattern.extend(folded.chars());
        let mut prefix = vec![0; count];
        for index in 1..count {
            let mut matched = prefix[index - 1];
            while matched > 0 && pattern[index] != pattern[matched] {
                matched = prefix[matched - 1];
            }
            if pattern[index] == pattern[matched] {
                matched += 1;
            }
            prefix[index] = matched;
        }
        Ok(Self { pattern, prefix })
    }
    #[cfg(any(feature = "desktop", test))]
    pub(crate) fn matches(&self, name: &str) -> bool {
        self.matches_checked(name, || Ok(()))
            .expect("infallible UI match")
    }
    #[cfg(test)]
    fn matches_counted(&self, name: &str, mut compared: impl FnMut()) -> bool {
        self.matches_checked(name, || {
            compared();
            Ok(())
        })
        .unwrap()
    }
    pub(crate) fn matches_worker(
        &self,
        name: &str,
        control: &crate::operation::JobControl,
    ) -> crate::error::AppResult<bool> {
        control.check_cancelled()?;
        let mut count = 0usize;
        self.matches_checked(name, || {
            count += 1;
            if count.is_multiple_of(4096) {
                control.check_cancelled()?;
            }
            Ok(())
        })
    }
    fn matches_checked(
        &self,
        name: &str,
        mut compared: impl FnMut() -> crate::error::AppResult<()>,
    ) -> crate::error::AppResult<bool> {
        let length = self.pattern.len();
        if length == 0 {
            return Ok(true);
        }
        // Matches may end inside a lowercase expansion, but must start at an
        // original-name scalar boundary, as in the previous borrowed matcher.
        let mut boundaries = vec![0u8; length];
        let mut cursor = 0;
        let mut matched = 0;
        for character in name.chars() {
            for (part, folded) in character.to_lowercase().enumerate() {
                boundaries[cursor] = u8::from(part == 0);
                cursor = (cursor + 1) % length;
                loop {
                    compared()?;
                    if folded == self.pattern[matched] {
                        matched += 1;
                        break;
                    }
                    if matched == 0 {
                        break;
                    }
                    matched = self.prefix[matched - 1];
                }
                if matched == length {
                    if boundaries[cursor] != 0 {
                        return Ok(true);
                    }
                    matched = self.prefix[matched - 1];
                }
            }
        }
        Ok(false)
    }
}
#[cfg(test)]
mod search_review_tests {
    use super::*;
    #[test]
    fn repeated_prefix_search_has_linear_comparison_bound() {
        for length in [10_000, 20_000, 40_000] {
            let name = "a".repeat(length);
            for query_length in [128, 1024, 8192] {
                let query = format!("{}b", "a".repeat(query_length - 1));
                let mut comparisons = 0usize;
                let matcher = NameQuery::new(&query).unwrap();
                assert!(!matcher.matches_counted(&name, || comparisons += 1));
                assert!(comparisons <= 2 * length,
                    "{length} source chars, {query_length} pattern chars: {comparisons} comparisons");
            }
        }
    }
    #[test]
    fn unicode_expansions_and_overlapping_prefixes_keep_matching_semantics() {
        for (name, query, expected) in [
            ("Sgôr ŵ", "GÔR Ŵ", true),
            ("xxİstanbul", "İST", true),
            ("İ", "i", true),
            ("İ", "\u{307}", false),
            ("İ\u{307}", "\u{307}", true),
            ("abababac", "ababac", true),
            ("aaaaaab", "aaaab", true),
            ("aaaaaab", "aaaac", false),
            ("Σ", "Σ", true),
            ("ΟΣ", "ΟΣ", true),
            ("", "", true),
            ("", "a", false),
        ] {
            assert_eq!(
                NameQuery::new(query).unwrap().matches(name),
                expected,
                "{name:?}, {query:?}"
            );
        }
    }
}

#[cfg(test)]
mod search_limits_tests {
    use super::*;
    #[test]
    fn maximal_query_scratch_is_bounded_and_oversized_queries_are_refused() {
        for character in ['a', 'İ', '𐐀'] {
            let query = character.to_string().repeat(8192);
            let matcher = NameQuery::new(&query).unwrap();
            let folded: String = query.chars().flat_map(char::to_lowercase).collect();
            let peak = folded.capacity()
                + matcher.pattern.capacity() * std::mem::size_of::<char>()
                + matcher.prefix.capacity() * std::mem::size_of::<usize>()
                + matcher.pattern.len();
            assert!(peak <= SEARCH_SCRATCH_BYTES);
            assert!(matcher.matches(&query));
            assert!(NameQuery::new(&(query + "a")).is_err());
        }
        const {
            assert!(SEARCH_SCRATCH_BYTES <= 4 * 1024 * 1024 / 8);
        }
    }
    #[test]
    fn streaming_matches_agree_with_identical_scalar_folding_and_boundary_rules() {
        let alphabet = ['a', 'b', 'İ', '\u{307}', 'Σ', 'ς'];
        let words: Vec<String> = alphabet
            .iter()
            .flat_map(|a| {
                alphabet
                    .iter()
                    .flat_map(move |b| alphabet.into_iter().map(move |c| format!("{a}{b}{c}")))
            })
            .collect();
        for query in &words {
            let matcher = NameQuery::new(query).unwrap();
            let folded: String = query.chars().flat_map(char::to_lowercase).collect();
            for name in &words {
                let reference = name.char_indices().any(|(start, _)| {
                    let mut candidate = name[start..].chars().flat_map(char::to_lowercase);
                    folded
                        .chars()
                        .all(|character| candidate.next() == Some(character))
                });
                assert_eq!(matcher.matches(name), reference, "{name:?}, {query:?}");
            }
        }
    }
}

#[cfg(test)]
mod cancellation_tests {
    use super::*;
    #[test]
    fn inner_comparisons_cancel_without_publishing_progress() {
        let control = crate::operation::JobControl::default();
        let matcher = NameQuery::new("aaaaab").unwrap();
        let mut comparisons = 0usize;
        let result = matcher.matches_checked(&"a".repeat(100_000), || {
            comparisons += 1;
            if comparisons == 4096 {
                control.cancel();
            }
            if comparisons.is_multiple_of(4096) {
                control.check_cancelled()?;
            }
            Ok(())
        });
        assert!(matches!(result, Err(crate::error::AppError::Cancelled)));
        assert_eq!(comparisons, 4096);
        assert!(matches!(
            matcher.matches_worker("aaa", &control),
            Err(crate::error::AppError::Cancelled)
        ));
    }
}
