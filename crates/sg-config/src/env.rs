use crate::error::ConfigError;

/// Expands `${VAR_NAME}` references against the process environment.
/// Fails closed: an undefined variable is a config error, not a silent
/// empty string, since these typically carry HEC tokens.
pub fn expand_env(input: &str) -> Result<String, ConfigError> {
    let chars: Vec<char> = input.chars().collect();
    let mut out = String::with_capacity(input.len());
    let mut i = 0;
    while i < chars.len() {
        if chars[i] == '$' && chars.get(i + 1) == Some(&'{') {
            if let Some(rel_end) = chars[i + 2..].iter().position(|&c| c == '}') {
                let name: String = chars[i + 2..i + 2 + rel_end].iter().collect();
                if !name.is_empty()
                    && name.chars().all(|c| c.is_ascii_alphanumeric() || c == '_')
                {
                    let value = std::env::var(&name)
                        .map_err(|_| ConfigError::UndefinedEnvVar(name.clone()))?;
                    out.push_str(&value);
                    i += 2 + rel_end + 1;
                    continue;
                }
            }
        }
        out.push(chars[i]);
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
}
