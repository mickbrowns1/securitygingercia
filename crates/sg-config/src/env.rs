use crate::error::ConfigError;

/// Expands `${VAR_NAME}` references against the process environment.
/// Fails closed: an undefined variable is a config error, not a silent
/// empty string, since these typically carry HEC tokens.
///
/// Comment- and quote-aware (though not a full YAML parser): text inside
/// a `#` comment is left completely untouched -- including anything that
/// looks like `${...}` -- so authors can write literal dollar-brace text
/// in documentation/comments without it being mistaken for a secret
/// reference. A `#` only starts a comment where YAML says it can (at the
/// start of a line, or preceded by whitespace, and not inside a quoted
/// scalar); substitution still happens normally inside single- and
/// double-quoted strings, since `token: "${VAR}"` is a completely normal
/// way to write a reference.
pub fn expand_env(input: &str) -> Result<String, ConfigError> {
    let chars: Vec<char> = input.chars().collect();
    let mut out = String::with_capacity(input.len());
    let mut i = 0;
    let mut in_single_quote = false;
    let mut in_double_quote = false;
    let mut in_comment = false;
    // True at the start of input and right after whitespace/newline --
    // YAML only treats `#` as a comment start there, not when it's glued
    // to preceding non-space text (e.g. `foo#bar` is a plain scalar
    // containing `#`, not `foo` followed by a comment).
    let mut at_comment_boundary = true;

    while i < chars.len() {
        let c = chars[i];

        if in_comment {
            out.push(c);
            i += 1;
            if c == '\n' {
                in_comment = false;
                at_comment_boundary = true;
            }
            continue;
        }

        if in_double_quote {
            if c == '\\' && i + 1 < chars.len() {
                // Copy the escaped character verbatim without
                // interpreting it, so `\"` doesn't end the string.
                out.push(c);
                out.push(chars[i + 1]);
                i += 2;
                at_comment_boundary = false;
                continue;
            }
            if c == '"' {
                in_double_quote = false;
            }
        } else if in_single_quote {
            if c == '\'' {
                // YAML escapes a literal `'` inside a single-quoted
                // scalar by doubling it (`''`); a doubled quote doesn't
                // end the string.
                if chars.get(i + 1) == Some(&'\'') {
                    out.push('\'');
                    out.push('\'');
                    i += 2;
                    at_comment_boundary = false;
                    continue;
                }
                in_single_quote = false;
            }
        } else {
            // Not inside any quote or comment right now.
            if c == '#' && at_comment_boundary {
                in_comment = true;
                out.push(c);
                i += 1;
                continue;
            }
            if c == '"' {
                in_double_quote = true;
            } else if c == '\'' {
                in_single_quote = true;
            }
        }

        if c == '$' && chars.get(i + 1) == Some(&'{') {
            if let Some(rel_end) = chars[i + 2..].iter().position(|&x| x == '}') {
                let name: String = chars[i + 2..i + 2 + rel_end].iter().collect();
                if !name.is_empty()
                    && name.chars().all(|ch| ch.is_ascii_alphanumeric() || ch == '_')
                {
                    let value = std::env::var(&name)
                        .map_err(|_| ConfigError::UndefinedEnvVar(name.clone()))?;
                    out.push_str(&value);
                    i += 2 + rel_end + 1;
                    at_comment_boundary = false;
                    continue;
                }
            }
        }

        out.push(c);
        at_comment_boundary = c == '\n' || c == ' ' || c == '\t';
        i += 1;
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn expands_defined_var() {
        std::env::set_var("SG_TEST_VAR", "secret-token");
        let out = expand_env("token: ${SG_TEST_VAR}").unwrap();
        assert_eq!(out, "token: secret-token");
    }

    #[test]
    fn errors_on_undefined_var() {
        std::env::remove_var("SG_TEST_VAR_UNDEFINED");
        let err = expand_env("token: ${SG_TEST_VAR_UNDEFINED}").unwrap_err();
        assert!(matches!(err, ConfigError::UndefinedEnvVar(_)));
    }

    #[test]
    fn leaves_plain_text_untouched() {
        let out = expand_env("plain: value, no interpolation here").unwrap();
        assert_eq!(out, "plain: value, no interpolation here");
    }

    #[test]
    fn full_line_comment_with_dollar_brace_text_is_never_substituted() {
        std::env::set_var("SG_TEST_VAR", "secret-token");
        std::env::remove_var("SG_TEST_COMMENT_ONLY_VAR");
        let input = "# see ${SG_TEST_COMMENT_ONLY_VAR} for details\ntoken: ${SG_TEST_VAR}\n";
        // Must not error even though SG_TEST_COMMENT_ONLY_VAR is undefined,
        // since it only ever appears inside a comment.
        let out = expand_env(input).unwrap();
        assert_eq!(
            out,
            "# see ${SG_TEST_COMMENT_ONLY_VAR} for details\ntoken: secret-token\n"
        );
    }

    #[test]
    fn trailing_comment_after_a_value_is_not_substituted() {
        std::env::remove_var("SG_TEST_TRAILING_VAR");
        let input = "key: value # trailing ${SG_TEST_TRAILING_VAR} note\n";
        let out = expand_env(input).unwrap();
        assert_eq!(out, input, "trailing comment must survive byte-for-byte");
    }

    #[test]
    fn hash_inside_double_quoted_string_is_not_a_comment() {
        let out = expand_env(r#"pattern: "foo#bar""#).unwrap();
        assert_eq!(out, r#"pattern: "foo#bar""#);
    }

    #[test]
    fn hash_glued_to_a_word_does_not_start_a_comment() {
        // If `#` here were (incorrectly) treated as starting a comment,
        // the ${SG_TEST_VAR} that follows on the same line would be left
        // unsubstituted instead of resolved.
        std::env::set_var("SG_TEST_VAR", "secret-token");
        let out = expand_env("value: foo#bar ${SG_TEST_VAR}\n").unwrap();
        assert_eq!(out, "value: foo#bar secret-token\n");
    }

    #[test]
    fn dollar_brace_inside_double_quoted_string_is_still_substituted() {
        std::env::set_var("SG_TEST_VAR", "secret-token");
        let out = expand_env(r#"token: "${SG_TEST_VAR}""#).unwrap();
        assert_eq!(out, r#"token: "secret-token""#);
    }

    #[test]
    fn dollar_brace_inside_single_quoted_string_is_still_substituted() {
        std::env::set_var("SG_TEST_VAR", "secret-token");
        let out = expand_env("token: '${SG_TEST_VAR}'").unwrap();
        assert_eq!(out, "token: 'secret-token'");
    }

    #[test]
    fn doubled_single_quote_escape_does_not_end_the_string_early() {
        std::env::set_var("SG_TEST_VAR", "secret-token");
        let out = expand_env("note: 'it''s ${SG_TEST_VAR} value'").unwrap();
        assert_eq!(out, "note: 'it''s secret-token value'");
    }
}
