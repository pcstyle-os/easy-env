use anyhow::{Result, bail};

use crate::EnvKey;

pub fn parse_dotenv(contents: &str) -> Result<Vec<(EnvKey, String)>> {
    let mut entries = Vec::new();

    for (index, line) in contents.lines().enumerate() {
        let line_no = index + 1;
        let mut line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }

        if let Some(rest) = line.strip_prefix("export ") {
            line = rest.trim_start();
        }

        let Some((raw_key, raw_value)) = line.split_once('=') else {
            bail!("invalid dotenv line {line_no}: expected KEY=value")
        };

        let key = EnvKey::parse(raw_key.trim())?;
        let value = parse_value(raw_value.trim(), line_no)?;
        entries.push((key, value));
    }

    Ok(entries)
}

fn parse_value(raw: &str, line_no: usize) -> Result<String> {
    if raw.starts_with('"') {
        return parse_double_quoted(raw, line_no);
    }

    if raw.starts_with('\'') {
        return parse_single_quoted(raw, line_no);
    }

    let mut value = String::new();
    let mut previous_was_space = false;
    for ch in raw.chars() {
        if ch == '#' && previous_was_space {
            break;
        }
        previous_was_space = ch.is_whitespace();
        value.push(ch);
    }
    Ok(value.trim_end().to_string())
}

fn parse_double_quoted(raw: &str, line_no: usize) -> Result<String> {
    let mut value = String::new();
    let mut escaped = false;

    for ch in raw[1..].chars() {
        if escaped {
            value.push(match ch {
                'n' => '\n',
                'r' => '\r',
                't' => '\t',
                '"' => '"',
                '\\' => '\\',
                other => other,
            });
            escaped = false;
            continue;
        }

        match ch {
            '\\' => escaped = true,
            '"' => return Ok(value),
            other => value.push(other),
        }
    }

    bail!("invalid dotenv line {line_no}: unterminated double-quoted value")
}

fn parse_single_quoted(raw: &str, line_no: usize) -> Result<String> {
    let tail = &raw[1..];
    let Some(end) = tail.find('\'') else {
        bail!("invalid dotenv line {line_no}: unterminated single-quoted value")
    };
    Ok(tail[..end].to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_basic_dotenv_files() {
        let entries = parse_dotenv(
            "OPENAI_API_KEY=sk-test\nexport STRIPE_SECRET='abc'\nQUOTED=\"hello\\nworld\"\n",
        )
        .unwrap();
        assert_eq!(entries.len(), 3);
        assert_eq!(entries[1].1, "abc");
        assert_eq!(entries[2].1, "hello\nworld");
    }

    #[test]
    fn strips_comments_from_unquoted_values() {
        let entries = parse_dotenv("FOO=bar # comment\n").unwrap();
        assert_eq!(entries[0].1, "bar");
    }
}
