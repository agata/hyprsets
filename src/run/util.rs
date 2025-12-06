use std::{collections::HashMap, path::Path};

pub(crate) fn build_exec_command<'a>(
    base_cmd: &str,
    cwd: Option<&Path>,
    env_layers: impl IntoIterator<Item = &'a HashMap<String, String>>,
) -> String {
    let mut exec = String::new();

    if let Some(dir) = cwd {
        exec.push_str("cd ");
        exec.push_str(&shell_escape(&dir.to_string_lossy()));
        exec.push_str(" && ");
    }

    let mut has_env = false;
    for env in env_layers {
        for (key, value) in env {
            if has_env {
                exec.push(' ');
            }
            exec.push_str(key);
            exec.push('=');
            exec.push_str(&shell_escape(value));
            has_env = true;
        }
    }

    if has_env {
        exec.push(' ');
    }

    exec.push_str(base_cmd);
    exec
}

pub(crate) fn shell_escape(raw: &str) -> String {
    let mut escaped = String::from("'");
    for ch in raw.chars() {
        if ch == '\'' {
            escaped.push_str("'\"'\"'");
        } else {
            escaped.push(ch);
        }
    }
    escaped.push('\'');
    escaped
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    #[test]
    fn shell_escape_handles_quotes() {
        let raw = "O'Reilly & \"Friends\"";
        let escaped = shell_escape(raw);
        assert_eq!(escaped, "'O'\"'\"'Reilly & \"Friends\"'");
    }

    #[test]
    fn build_exec_command_with_cwd_and_env() {
        let mut env1 = HashMap::new();
        env1.insert("FOO".to_string(), "bar".to_string());
        let mut env2 = HashMap::new();
        env2.insert("BAZ".to_string(), "qux".to_string());

        let cmd = build_exec_command(
            "echo $FOO $BAZ",
            Some(Path::new("/tmp/work")),
            [&env1, &env2],
        );
        assert!(
            cmd.starts_with("cd '/tmp/work' && "),
            "cmd should start with cwd prefix"
        );
        // 順序は未定だが両方の環境変数が含まれることを確認
        assert!(cmd.contains("FOO='bar'"));
        assert!(cmd.contains("BAZ='qux'"));
        assert!(
            cmd.ends_with("echo $FOO $BAZ"),
            "base command should be appended"
        );
    }

    #[test]
    fn build_exec_command_without_cwd() {
        let cmd = build_exec_command(
            "ls -la",
            None,
            std::iter::empty::<&HashMap<String, String>>(),
        );
        assert_eq!(cmd, "ls -la");
    }
}
