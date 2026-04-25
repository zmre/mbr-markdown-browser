//! Readability scoring utilities (Flesch Reading Ease and Flesch-Kincaid Grade Level).
//!
//! This module provides a small, dependency-free implementation of two widely
//! used readability metrics that are computed server-side during markdown
//! rendering:
//!
//! * **Flesch Reading Ease (FRE)** — higher values mean easier to read
//!   (0 = extremely difficult, 100 = extremely easy).
//! * **Flesch-Kincaid Grade Level (FKGL)** — expressed as a US school grade.
//!
//! The heart of both formulas is the syllable count. We use a simple
//! Aydin/sylco-style heuristic that is correct on the overwhelming majority of
//! English words and degrades gracefully on edge cases. This matches the
//! project's "zero run-time dependencies" principle — the formulas themselves
//! are trivial arithmetic.
//!
//! # Example
//!
//! ```
//! use mbr::readability::{ReadabilityCounts, scores};
//!
//! let counts = ReadabilityCounts {
//!     words: 10,
//!     sentences: 2,
//!     syllables: 14,
//! };
//! let s = scores(&counts);
//! assert!(s.flesch_reading_ease.is_some());
//! assert!(s.flesch_kincaid_grade.is_some());
//! ```

/// Accumulated counts needed to compute readability scores.
///
/// All fields are tallied during the single-pass markdown event loop so the
/// computation piggybacks on work we are already doing.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct ReadabilityCounts {
    /// Total words outside code/metadata blocks.
    pub words: usize,
    /// Total sentence-terminating events.
    pub sentences: usize,
    /// Total syllables across counted words.
    pub syllables: usize,
}

/// Computed readability scores.
///
/// Each score is `None` when the input counts are insufficient to produce a
/// meaningful value (for example, empty documents or code-only files).
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct ReadabilityScores {
    /// Flesch Reading Ease. Typical range 0–100 (can exceed either bound
    /// slightly for degenerate inputs).
    pub flesch_reading_ease: Option<f32>,
    /// Flesch-Kincaid Grade Level (US school grade).
    pub flesch_kincaid_grade: Option<f32>,
}

/// Count syllables in a word using a simple heuristic.
///
/// The algorithm is a pared-down version of the well-known Aydin/sylco
/// approach used by libraries like Wooorm's `syllable`:
///
/// 1. lowercase and keep only ASCII letters
/// 2. count transitions from non-vowel to vowel (collapse vowel runs)
/// 3. subtract 1 for a silent trailing `e` (but not `le` after a consonant,
///    which is its own syllable in words like `table`, `simple`)
/// 4. ensure a minimum of 1 syllable for any non-empty word
///
/// The heuristic treats `y` as a vowel for counting purposes, which is a
/// reasonable approximation for English. Accuracy is roughly 80–90% on common
/// English corpora — more than good enough for readability band estimation.
///
/// Returns `0` for inputs that contain no ASCII letters.
///
/// # Examples
///
/// ```
/// use mbr::readability::count_syllables;
/// assert_eq!(count_syllables("the"), 1);
/// assert_eq!(count_syllables("cat"), 1);
/// assert_eq!(count_syllables("apple"), 2);
/// ```
pub fn count_syllables(word: &str) -> usize {
    // Normalize: keep only ASCII letters, lowercase them.
    let normalized: Vec<u8> = word
        .bytes()
        .filter(|b| b.is_ascii_alphabetic())
        .map(|b| b.to_ascii_lowercase())
        .collect();

    if normalized.is_empty() {
        return 0;
    }

    let is_vowel = |b: u8| matches!(b, b'a' | b'e' | b'i' | b'o' | b'u' | b'y');

    // Count vowel groups: each transition from non-vowel to vowel is +1.
    let mut count: usize = 0;
    let mut prev_was_vowel = false;
    for &b in &normalized {
        let v = is_vowel(b);
        if v && !prev_was_vowel {
            count += 1;
        }
        prev_was_vowel = v;
    }

    // Silent trailing `e`: subtract 1 unless the word ends in `le` preceded by
    // a consonant (e.g. `table` — that `le` is its own syllable).
    let len = normalized.len();
    if len >= 2 && normalized[len - 1] == b'e' {
        let ends_in_le_after_consonant =
            len >= 3 && normalized[len - 2] == b'l' && !is_vowel(normalized[len - 3]);
        if !ends_in_le_after_consonant {
            count = count.saturating_sub(1);
        }
    }

    // Non-empty words always get at least one syllable.
    count.max(1)
}

/// Compute readability scores from accumulated counts.
///
/// Returns `None` for either score when `words == 0` or `sentences == 0`, which
/// would otherwise produce a division by zero.
///
/// Formulas from Wikipedia
/// (<https://en.wikipedia.org/wiki/Flesch%E2%80%93Kincaid_readability_tests>):
///
/// ```text
/// FRE  = 206.835 - 1.015 * (words / sentences) - 84.6  * (syllables / words)
/// FKGL =   0.39  * (words / sentences) + 11.8  * (syllables / words) - 15.59
/// ```
#[must_use]
pub fn scores(counts: &ReadabilityCounts) -> ReadabilityScores {
    if counts.words == 0 || counts.sentences == 0 {
        return ReadabilityScores::default();
    }

    let words = counts.words as f32;
    let sentences = counts.sentences as f32;
    let syllables = counts.syllables as f32;

    let words_per_sentence = words / sentences;
    let syllables_per_word = syllables / words;

    let fre = 206.835 - 1.015 * words_per_sentence - 84.6 * syllables_per_word;
    let fkgl = 0.39 * words_per_sentence + 11.8 * syllables_per_word - 15.59;

    ReadabilityScores {
        flesch_reading_ease: Some(fre),
        flesch_kincaid_grade: Some(fkgl),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Helper: assert a float is within `tolerance` of `expected`.
    fn assert_close(actual: f32, expected: f32, tolerance: f32, label: &str) {
        assert!(
            (actual - expected).abs() < tolerance,
            "{label}: expected {expected} ± {tolerance}, got {actual}"
        );
    }

    #[test]
    fn syllables_empty_word_is_zero() {
        assert_eq!(count_syllables(""), 0);
        assert_eq!(count_syllables("   "), 0);
        assert_eq!(count_syllables("!!!"), 0);
    }

    #[test]
    fn syllables_single_syllable_words() {
        assert_eq!(count_syllables("the"), 1);
        assert_eq!(count_syllables("cat"), 1);
        assert_eq!(count_syllables("dog"), 1);
        assert_eq!(count_syllables("run"), 1);
        assert_eq!(count_syllables("eight"), 1);
    }

    #[test]
    fn syllables_two_syllable_words() {
        // Words where our heuristic agrees with natural syllable count.
        assert_eq!(count_syllables("apple"), 2);
        assert_eq!(count_syllables("table"), 2);
        assert_eq!(count_syllables("simple"), 2);
    }

    #[test]
    fn syllables_longer_words() {
        // "readability" → read-a-bil-i-ty → 5
        assert_eq!(count_syllables("readability"), 5);
    }

    #[test]
    fn syllables_edge_cases_documented() {
        // The Aydin heuristic is approximate — these are the values our
        // implementation produces. Documented here so regressions are
        // obvious and so the heuristic's behaviour is stable.
        //
        // "every" is pronounced as 2 (ev-ry) or 3 (ev-er-y). Our heuristic
        // sees three vowel groups (e, e, y) and no trailing silent `e`,
        // so it produces 3.
        assert_eq!(count_syllables("every"), 3);
        // "shoreline" has a silent medial `e` our heuristic cannot detect,
        // so we count 3 syllables (sho-re-line) instead of the true 2.
        assert_eq!(count_syllables("shoreline"), 3);
        // "simile" (si-mi-le) — the final `e` in -ile is dropped by the
        // silent-e rule because the preceding `l` is itself preceded by a
        // vowel (i), so our heuristic produces 2 instead of 3.
        assert_eq!(count_syllables("simile"), 2);
        // "queue" has 2 vowel groups collapsed into 1 by the silent-e rule,
        // giving 1. (True English count is 1.)
        assert_eq!(count_syllables("queue"), 1);
    }

    #[test]
    fn syllables_ignores_non_alpha_characters() {
        assert_eq!(count_syllables("don't"), count_syllables("dont"));
        assert_eq!(count_syllables("cat!"), 1);
        assert_eq!(count_syllables("   hello   "), count_syllables("hello"));
    }

    #[test]
    fn syllables_handles_all_vowels_word() {
        assert_eq!(count_syllables("aeiou"), 1); // single vowel run
    }

    #[test]
    fn scores_returns_none_for_empty_input() {
        let empty = ReadabilityCounts::default();
        let s = scores(&empty);
        assert!(s.flesch_reading_ease.is_none());
        assert!(s.flesch_kincaid_grade.is_none());
    }

    #[test]
    fn scores_returns_none_when_sentences_is_zero() {
        let counts = ReadabilityCounts {
            words: 10,
            sentences: 0,
            syllables: 15,
        };
        let s = scores(&counts);
        assert!(s.flesch_reading_ease.is_none());
        assert!(s.flesch_kincaid_grade.is_none());
    }

    #[test]
    fn scores_returns_none_when_words_is_zero() {
        let counts = ReadabilityCounts {
            words: 0,
            sentences: 1,
            syllables: 0,
        };
        let s = scores(&counts);
        assert!(s.flesch_reading_ease.is_none());
        assert!(s.flesch_kincaid_grade.is_none());
    }

    #[test]
    fn scores_known_simple_sentence() {
        // "The cat sat on the mat." — 6 words, 1 sentence, all one-syllable.
        // FRE = 206.835 - 1.015 * 6 - 84.6 * 1 = 116.145
        // FKGL = 0.39 * 6 + 11.8 * 1 - 15.59 = -1.45
        let counts = ReadabilityCounts {
            words: 6,
            sentences: 1,
            syllables: 6,
        };
        let s = scores(&counts);
        assert_close(s.flesch_reading_ease.unwrap(), 116.145, 0.01, "FRE");
        assert_close(s.flesch_kincaid_grade.unwrap(), -1.45, 0.01, "FKGL");
    }

    #[test]
    fn scores_typical_prose() {
        // Roughly: average English ~15 words/sentence, ~1.5 syllables/word.
        // FRE = 206.835 - 1.015 * 15 - 84.6 * 1.5 = 64.785 - 126.9 = 64.785
        //     = 206.835 - 15.225 - 126.9 = 64.71
        // FKGL = 0.39 * 15 + 11.8 * 1.5 - 15.59 = 5.85 + 17.7 - 15.59 = 7.96
        let counts = ReadabilityCounts {
            words: 150,
            sentences: 10,
            syllables: 225,
        };
        let s = scores(&counts);
        let fre = s.flesch_reading_ease.unwrap();
        let fkgl = s.flesch_kincaid_grade.unwrap();
        assert!(
            (55.0..=75.0).contains(&fre),
            "FRE for typical prose should be ~65, got {fre}"
        );
        assert!(
            (6.0..=10.0).contains(&fkgl),
            "FKGL for typical prose should be ~8, got {fkgl}"
        );
    }
}
