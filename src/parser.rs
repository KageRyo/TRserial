use regex::Regex;
use serde::{Deserialize, Serialize};
use std::{collections::HashSet, sync::LazyLock};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ParsedCode {
    pub code: String,
    pub rewards: Vec<String>,
    /// Original text, not a guessed year or timezone.
    pub expiration: Option<String>,
    pub needs_review: bool,
}

static LABEL: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?i)^(?:(?:序號(?:碼)?|兑换码|兌換碼)\s*[:：=]?\s*|(?:serial(?:\s+code)?s?|codes?)(?:\s*[:：=]\s*|\s+|$))(.*)$")
        .unwrap()
});
static TOKEN: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"^[A-Za-z0-9]+(?:[-_][A-Za-z0-9]+)*$").unwrap());
static DATE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"^\d{4}[-_]\d{1,2}[-_]\d{1,2}$").unwrap());
static URL: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"(?i)(?:https?://|www\.)\S+").unwrap());

pub fn valid_code(value: &str) -> bool {
    !value.is_empty() && value.len() <= 256 && TOKEN.is_match(value)
}

pub fn parse(text: &str) -> Vec<ParsedCode> {
    let mut codes: Vec<ParsedCode> = Vec::new();
    let mut seen = HashSet::new();
    let mut labelled = false;
    for raw in text.lines() {
        let line = raw.trim().trim_matches(['`', '*', ' ', '\t']);
        if line.is_empty() {
            continue;
        }
        if line.contains("到期")
            || line.contains("有效期")
            || line.to_lowercase().starts_with("expir")
        {
            if let Some(last) = codes.last_mut() {
                last.expiration = Some(line.to_owned());
            }
            labelled = false;
            continue;
        }
        let capture = LABEL.captures(line);
        let explicit = capture.is_some() || labelled;
        let value = capture
            .as_ref()
            .map_or(line, |c| c.get(1).unwrap().as_str());
        if capture.is_some() && value.is_empty() {
            labelled = true;
            continue;
        }
        let stripped = URL.replace_all(value, " ");
        let mut found = false;
        for word in stripped
            .split(|c: char| c.is_whitespace() || matches!(c, ',' | '，' | ';' | '；' | '、' | '|'))
        {
            let word = word.trim_matches(['`', '*', '"', '\'', '(', ')', '（', '）']);
            let mixed = word.chars().any(|c| c.is_ascii_alphabetic())
                && word.chars().any(|c| c.is_ascii_digit());
            let quantity = word
                .strip_prefix(['x', 'X'])
                .is_some_and(|n| !n.is_empty() && n.chars().all(|c| c.is_ascii_digit()));
            // Bare prose is never scanned token-by-token. Only a standalone mixed code is a candidate.
            if valid_code(word)
                && !DATE.is_match(word)
                && (explicit || !quantity)
                && (explicit || (value == word && mixed))
                && (explicit || mixed)
            {
                found = true;
                if seen.insert(word.to_owned()) {
                    codes.push(ParsedCode {
                        code: word.to_owned(),
                        rewards: vec![],
                        expiration: None,
                        needs_review: !explicit,
                    });
                }
            }
        }
        labelled = false;
        if !found
            && capture.is_none()
            && let Some(last) = codes.last_mut()
        {
            last.rewards.push(raw.trim().to_owned());
        }
    }
    codes
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn supplied_message_retains_rewards_and_expiry() {
        let p = parse(
            "序號\nmalojam1106\n20周年TR箱子(小) 1個\n綿綿兔箱子 3個\n角色兌換券 1張\n序號到期日:11/15 23:59",
        );
        assert_eq!(p[0].code, "malojam1106");
        assert_eq!(
            p[0].rewards,
            ["20周年TR箱子(小) 1個", "綿綿兔箱子 3個", "角色兌換券 1張"]
        );
        assert_eq!(p[0].expiration.as_deref(), Some("序號到期日:11/15 23:59"));
        assert!(!p[0].needs_review);
    }
    #[test]
    fn multiple_variable_length_and_dedup() {
        let p = parse("序號：a1, verylong_CODE2026\n序號 a1\nCode: LETTERS\n序號: 1234");
        assert_eq!(
            p.iter().map(|p| p.code.as_str()).collect::<Vec<_>>(),
            ["a1", "verylong_CODE2026", "LETTERS", "1234"]
        );
    }
    #[test]
    fn excludes_urls_dates_rewards_and_prose() {
        for text in [
            "https://example.com/abc123",
            "11/15 23:59",
            "20周年TR箱子(小) 1個",
            "hello abc123 world",
            "序號: https://example.com/abc123",
            "序號: 2026-11-15",
            "Code: 2026_11_15",
            "x20",
            "",
            "\0💥",
            "序號到期日:11/15 23:59",
        ] {
            assert!(parse(text).is_empty(), "{text:?}");
        }
    }
    #[test]
    fn ambiguous_candidates_need_review_and_preserve_case() {
        let p = parse("`Abc1`\n不確定的獎勵說明");
        assert!(p[0].needs_review);
        assert_eq!(p[0].code, "Abc1");
        assert_eq!(p[0].rewards, ["不確定的獎勵說明"]);
    }
}

#[cfg(test)]
mod format_tests {
    use super::*;
    #[test]
    fn labelled_formats_and_short_codes_are_supported_without_prefix_confusion() {
        for message in [
            "Code: a1",
            "Serial code：a1",
            "序號碼=a1",
            "兌換碼 a1",
            "**序號：** `a1`",
            "序號\na1",
        ] {
            let p = parse(message);
            assert_eq!(p[0].code, "a1", "{message}");
            assert!(!p[0].needs_review);
        }
        assert!(parse("CodeBox123")[0].needs_review);
        assert_eq!(parse("Code: x1")[0].code, "x1");
    }
}
