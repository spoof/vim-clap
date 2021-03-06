#![feature(pattern)]

use filter::matcher::{get_appropriate_matcher, Algo};
use printer::truncate_long_matched_lines;
use pyo3::prelude::*;
use pyo3::wrap_pyfunction;
use std::collections::HashMap;

use std::str::pattern::Pattern;

#[inline]
fn find_start_at<'a, P: Pattern<'a>>(slice: &'a str, at: usize, pat: P) -> Option<usize> {
    slice[at..].find(pat).map(|i| at + i)
}

fn substr_scorer(niddle: &str, haystack: &str) -> Option<(f64, Vec<usize>)> {
    let haystack = haystack.to_lowercase();
    let haystack = haystack.as_str();

    let mut offset = 0;
    let mut positions = Vec::new();
    for sub_niddle in niddle.split_whitespace() {
        let sub_niddle = sub_niddle.to_lowercase();

        match find_start_at(haystack, offset, &sub_niddle) {
            Some(idx) => {
                offset = idx + sub_niddle.len();
                // For build without overflow checks this could be written as
                // `let mut pos = idx - 1;` with `|| { pos += 1; pos }` closure.
                let mut pos = idx;
                positions.resize_with(
                    positions.len() + sub_niddle.len(),
                    // Simple endless iterator for `idx..` range. Even though it's endless,
                    // it will iterate only `sub_niddle.len()` times.
                    || {
                        pos += 1;
                        pos - 1
                    },
                );
            }
            None => return None,
        }
    }

    if positions.is_empty() {
        return Some((0f64, positions));
    }

    let last_pos = positions.last().unwrap();
    let match_len = (last_pos + 1 - positions[0]) as f64;

    Some((
        ((2f64 / (positions[0] + 1) as f64) + 1f64 / (last_pos + 1) as f64 - match_len),
        positions,
    ))
}

/// Use f64 here as substr_scorer returns f64;
type MatcherResult = Option<(f64, Vec<usize>)>;
/// Pass a Vector of lines to Vim for setting them in Vim with one single API call.
type LinesInBatch = Vec<String>;
/// Each line's matched indices of LinesInBatch.
type MatchedIndicesInBatch = Vec<Vec<usize>>;
/// NOTE: TruncatedMap is ought to be HashMap<usize, String>,
/// but there is an issue when converting to call result to Vim Dict in python dynamic call,
/// therefore hereby has to use HashMap<String, String> instead.
type TruncatedMapInfo = HashMap<String, String>;

/// Filter the candidates given query using the fzy algorithm
#[pyfunction]
fn fuzzy_match(
    query: &str,
    candidates: Vec<String>,
    winwidth: usize,
    enable_icon: bool,
    line_splitter: String,
) -> PyResult<(MatchedIndicesInBatch, LinesInBatch, TruncatedMapInfo)> {
    let fzy_matcher = get_appropriate_matcher(&Algo::Fzy, &line_splitter.into());
    let matcher: Box<dyn Fn(&str) -> MatcherResult> = if query.contains(' ') {
        Box::new(|line: &str| substr_scorer(query, line))
    } else {
        Box::new(|line: &str| {
            if enable_icon {
                // " " is 4 bytes, but the offset of highlight is 2.
                fzy_matcher(&line[4..], query).map(|(score, indices)| {
                    (score as f64, indices.into_iter().map(|x| x + 4).collect())
                })
            } else {
                fzy_matcher(line, query).map(|(score, indices)| (score as f64, indices))
            }
        })
    };

    let mut ranked = candidates
        .into_iter()
        .filter_map(|line| matcher(&line).map(|(score, indices)| (line, score, indices)))
        .collect::<Vec<_>>();

    ranked.sort_unstable_by(|(_, v1, _), (_, v2, _)| v2.partial_cmp(v1).unwrap());

    // 2 = chars(icon)
    let skipped = if enable_icon { Some(2) } else { None };
    let (lines, truncated_map) = truncate_long_matched_lines(ranked, winwidth, skipped);

    let mut indices = Vec::with_capacity(lines.len());
    let mut filtered = Vec::with_capacity(lines.len());
    for (text, _, ids) in lines.into_iter() {
        indices.push(ids);
        filtered.push(text);
    }

    Ok((
        indices,
        filtered,
        truncated_map
            .into_iter()
            .map(|(k, v)| (k.to_string(), v))
            .collect(),
    ))
}

/// This module is a python module implemented in Rust.
#[pymodule]
fn fuzzymatch_rs(_py: Python, m: &PyModule) -> PyResult<()> {
    m.add_wrapped(wrap_pyfunction!(fuzzy_match))?;

    Ok(())
}

#[test]
fn py_and_rs_subscore_should_work() {
    use pyo3::{prelude::*, types::PyModule};
    use std::fs;

    let cur_dir = std::env::current_dir().unwrap();
    let py_path = cur_dir.parent().unwrap().join("scorer.py");
    let py_source_code = fs::read_to_string(py_path).unwrap();

    let gil = Python::acquire_gil();
    let py = gil.python();
    let py_scorer = PyModule::from_code(py, &py_source_code, "scorer.py", "scorer").unwrap();

    let test_cases = vec![
        ("su ou", "substr_scorer_should_work"),
        ("su ork", "substr_scorer_should_work"),
    ];

    for (niddle, haystack) in test_cases.into_iter() {
        let py_result: (f64, Vec<usize>) = py_scorer
            .call1("substr_scorer", (niddle, haystack))
            .unwrap()
            .extract()
            .unwrap();
        let rs_result = substr_scorer(niddle, haystack).unwrap();
        assert_eq!(py_result, rs_result);
    }
}

#[test]
fn test_skip_icon() {
    let lines = vec![" .dependabot/config.yml".into(), " .editorconfig".into()];
    let query = "con";
    println!(
        "ret: {:#?}",
        fuzzy_match(query, lines, 62, true, "Full".to_string())
    );
}
