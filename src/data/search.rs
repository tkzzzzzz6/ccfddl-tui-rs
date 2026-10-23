//! Ratcliff/Obershelp similarity, equivalent to Python's `difflib.SequenceMatcher.ratio()`
//! for short strings (no junk heuristics), so search ranking matches the Python version.

fn find_longest_match(
    a: &[char],
    b: &[char],
    (alo, ahi): (usize, usize),
    (blo, bhi): (usize, usize),
) -> (usize, usize, usize) {
    let mut best = (alo, blo, 0usize);
    let mut prev = vec![0usize; b.len() + 1];
    let mut cur = vec![0usize; b.len() + 1];

    for i in alo..ahi {
        for j in blo..bhi {
            if a[i] == b[j] {
                let k = prev[j] + 1;
                cur[j + 1] = k;
                if k > best.2 {
                    best = (i + 1 - k, j + 1 - k, k);
                }
            } else {
                cur[j + 1] = 0;
            }
        }
        std::mem::swap(&mut prev, &mut cur);
    }
    best
}

fn matching_chars(a: &[char], b: &[char]) -> usize {
    let mut total = 0;
    let mut queue = vec![((0, a.len()), (0, b.len()))];

    while let Some((a_range, b_range)) = queue.pop() {
        let (i, j, k) = find_longest_match(a, b, a_range, b_range);
        if k == 0 {
            continue;
        }
        total += k;
        if a_range.0 < i && b_range.0 < j {
            queue.push(((a_range.0, i), (b_range.0, j)));
        }
        if i + k < a_range.1 && j + k < b_range.1 {
            queue.push(((i + k, a_range.1), (j + k, b_range.1)));
        }
    }
    total
}

pub fn similarity_ratio(a: &str, b: &str) -> f64 {
    let a: Vec<char> = a.chars().collect();
    let b: Vec<char> = b.chars().collect();
    let length = a.len() + b.len();
    if length == 0 {
        return 1.0;
    }
    2.0 * matching_chars(&a, &b) as f64 / length as f64
}

#[cfg(test)]
mod tests {
    use super::*;

    fn close(a: f64, b: f64) -> bool {
        (a - b).abs() < 1e-12
    }

    #[test]
    fn identical_and_disjoint() {
        assert!(close(similarity_ratio("abc", "abc"), 1.0));
        assert!(close(similarity_ratio("abc", "xyz"), 0.0));
        assert!(close(similarity_ratio("", ""), 1.0));
        assert!(close(similarity_ratio("abc", ""), 0.0));
    }

    #[test]
    fn matches_python_difflib_reference_values() {
        // Reference values taken from difflib.SequenceMatcher(None, a, b).ratio().
        assert!(close(similarity_ratio("abcd", "bcde"), 0.75));
        assert!(close(similarity_ratio("cvpr", "cvpr 2025"), 2.0 * 4.0 / 13.0));
        assert!(close(similarity_ratio("abxcd", "abcd"), 2.0 * 4.0 / 9.0));
    }

    #[test]
    fn unicode_is_compared_per_char() {
        assert!(close(similarity_ratio("数据库", "数据库"), 1.0));
        assert!(close(similarity_ratio("数据", "数据库"), 0.8));
    }
}
