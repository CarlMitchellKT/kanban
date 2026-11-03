//! Splits free text into plain and link segments so links can be rendered clickable.

#[derive(Debug, PartialEq, Eq, Clone, Copy)]
pub enum Segment<'a> {
    Text(&'a str),
    Link(&'a str),
}

const SCHEMES: [&str; 2] = ["https://", "http://"];

/// Characters that, directly before a URL, still count as the start of a word.
fn is_boundary(c: char) -> bool {
    c.is_whitespace() || matches!(c, '(' | '[' | '<' | '"' | '\'' | '{')
}

/// Strips trailing punctuation that is almost certainly part of the sentence, not the URL.
/// A closing bracket is kept only if the URL contains its opening partner (Wikipedia style).
fn trim_url(url: &str) -> &str {
    let mut end = url.len();
    loop {
        let s = &url[..end];
        let Some(last) = s.chars().next_back() else {
            break;
        };
        let strip = match last {
            '.' | ',' | ';' | ':' | '!' | '?' | '\'' | '"' | '>' => true,
            ')' => s.matches(')').count() > s.matches('(').count(),
            ']' => s.matches(']').count() > s.matches('[').count(),
            '}' => s.matches('}').count() > s.matches('{').count(),
            _ => false,
        };
        if !strip {
            break;
        }
        end -= last.len_utf8();
    }
    &url[..end]
}

pub fn segments(text: &str) -> Vec<Segment<'_>> {
    let mut out = Vec::new();
    let mut plain_start = 0;
    let mut i = 0;
    while i < text.len() {
        let rest = &text[i..];
        let at_boundary = text[..i].chars().next_back().is_none_or(is_boundary);
        let scheme = SCHEMES.iter().find(|s| {
            rest.get(..s.len())
                .is_some_and(|p| p.eq_ignore_ascii_case(s))
        });
        if let (true, Some(scheme)) = (at_boundary, scheme) {
            let len = rest
                .find(|c: char| c.is_whitespace() || matches!(c, '<' | '>' | '"'))
                .unwrap_or(rest.len());
            let url = trim_url(&rest[..len]);
            if url.len() > scheme.len() {
                if plain_start < i {
                    out.push(Segment::Text(&text[plain_start..i]));
                }
                out.push(Segment::Link(url));
                i += url.len();
                plain_start = i;
                continue;
            }
        }
        i += rest.chars().next().map_or(1, char::len_utf8);
    }
    if plain_start < text.len() {
        out.push(Segment::Text(&text[plain_start..]));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::Segment::{Link, Text};
    use super::*;

    #[test]
    fn plain_text_is_one_segment() {
        assert_eq!(segments("nothing to see"), [Text("nothing to see")]);
        assert!(segments("").is_empty());
    }

    #[test]
    fn finds_link_in_the_middle() {
        assert_eq!(
            segments("see https://example.com/a?b=1 now"),
            [
                Text("see "),
                Link("https://example.com/a?b=1"),
                Text(" now")
            ]
        );
    }

    #[test]
    fn link_only_and_multiple_links() {
        assert_eq!(segments("http://a.io"), [Link("http://a.io")]);
        assert_eq!(
            segments("https://a.io https://b.io"),
            [Link("https://a.io"), Text(" "), Link("https://b.io")]
        );
    }

    #[test]
    fn strips_trailing_punctuation() {
        assert_eq!(
            segments("go to https://example.com."),
            [Text("go to "), Link("https://example.com"), Text(".")]
        );
        assert_eq!(
            segments("(see https://example.com/x), ok"),
            [Text("(see "), Link("https://example.com/x"), Text("), ok")]
        );
    }

    #[test]
    fn keeps_balanced_parentheses() {
        assert_eq!(
            segments("https://en.wikipedia.org/wiki/Rust_(language)"),
            [Link("https://en.wikipedia.org/wiki/Rust_(language)")]
        );
    }

    #[test]
    fn angle_brackets_and_quotes_delimit() {
        assert_eq!(
            segments("<https://example.com/a>"),
            [Text("<"), Link("https://example.com/a"), Text(">")]
        );
        assert_eq!(
            segments("\"https://example.com\""),
            [Text("\""), Link("https://example.com"), Text("\"")]
        );
    }

    #[test]
    fn scheme_must_start_a_word_and_have_a_host() {
        assert_eq!(segments("nothttps://x.io"), [Text("nothttps://x.io")]);
        assert_eq!(segments("https://"), [Text("https://")]);
        assert_eq!(segments("https://."), [Text("https://."),]);
    }

    #[test]
    fn scheme_is_case_insensitive() {
        assert_eq!(segments("HTTPS://X.IO"), [Link("HTTPS://X.IO")]);
    }

    #[test]
    fn handles_multibyte_text() {
        assert_eq!(
            segments("héllo → https://x.io/é ✓"),
            [Text("héllo → "), Link("https://x.io/é"), Text(" ✓")]
        );
    }

    #[test]
    fn segments_reassemble_to_input() {
        let s = "a https://x.io/(b). c, http://y.io? é";
        let joined: String = segments(s)
            .iter()
            .map(|seg| match seg {
                Text(t) | Link(t) => *t,
            })
            .collect();
        assert_eq!(joined, s);
    }
}
