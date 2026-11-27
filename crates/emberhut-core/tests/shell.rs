//! The shell reader: simple commands with their words unquoted, the folder each runs in, and
//! what heredocs feed them.
#![expect(
    clippy::unwrap_used,
    reason = "a test helper: a line it can't read fails the test that gave it"
)]

use emberhut_core::guard::shell::{self, Run};

const HOME: &str = "/home/user";

fn runs(command: &str) -> Vec<Run> {
    shell::parse(command, Some("/work"), false, HOME).unwrap()
}

fn argv(run: &Run) -> Vec<&str> {
    run.argv.iter().map(String::as_str).collect()
}

#[test]
fn quotes_come_off_and_expansions_are_marked_unknown() {
    let all = runs(r#"echo "a $X b" 'c d' e\ f $'g\nh' "$HOME/x""#);
    assert_eq!(all.len(), 1);
    assert_eq!(
        argv(&all[0]),
        ["echo", "a $X b", "c d", "e f", "g\nh", "/home/user/x"]
    );
    assert_eq!(all[0].fixed, [true, false, true, true, true, true]);
}

#[test]
fn cd_moves_the_rest_of_the_list_and_a_subshell_keeps_its_own() {
    let all = runs("cd sub && make; (cd /tmp && ls); pwd; cd; ls; cd $D; ls; popd; ls");
    let dirs: Vec<Option<&str>> = all.iter().map(|r| r.dir.as_deref()).collect();
    assert_eq!(
        dirs,
        [
            Some("/work"),
            Some("/work/sub"),
            Some("/work/sub"),
            Some("/tmp"),
            Some("/work/sub"),
            Some("/work/sub"),
            Some("/home/user"),
            Some("/home/user"),
            None,
            None,
            None,
        ]
    );
}

#[test]
fn a_heredoc_feeds_its_command_and_runs_only_its_substitutions() {
    let all = runs("cat <<'EOF' > f\ngit push\nEOF\npython3 - <<EOF\n$(git push)\nEOF\nls");
    let verbs: Vec<&str> = all.iter().map(|r| r.argv[0].as_str()).collect();
    assert_eq!(verbs, ["cat", "python3", "git", "ls"]);
    assert_eq!(all[0].stdin, ["git push"]);
    assert_eq!(all[1].stdin, ["$(git push)"]);
    let here = runs("bash <<<'git push'");
    assert_eq!(here[0].stdin, ["git push"]);
}

#[test]
fn substitutions_and_pipes_are_commands_of_their_own() {
    let all = runs("x=`git push` | tee log && echo $(date) <(sort f)");
    let verbs: Vec<&str> = all.iter().map(|r| r.argv[0].as_str()).collect();
    assert_eq!(verbs, ["git", "x=`…`", "tee", "date", "sort", "echo"]);
}

#[test]
fn what_it_cannot_read_is_an_error() {
    for bad in [
        "echo 'open",
        "echo \"open",
        "(ls",
        "ls )",
        "(( x + 1 ))",
        "echo $(ls",
        "f( x",
    ] {
        assert!(shell::parse(bad, None, false, HOME).is_err(), "{bad}");
    }
    let far = shell::parse("cd /srv && ls", Some("/work"), true, HOME).unwrap();
    assert!(
        far.iter().all(|r| r.dir.is_none()),
        "another machine's folders can't be told"
    );
}

#[test]
fn a_folder_is_placed_as_the_shell_would() {
    assert_eq!(
        shell::place("~/a", None, HOME).as_deref(),
        Some("/home/user/a")
    );
    assert_eq!(
        shell::place("/a/../b/./c", None, HOME).as_deref(),
        Some("/b/c")
    );
    assert_eq!(
        shell::place("../x", Some("/w/y"), HOME).as_deref(),
        Some("/w/x")
    );
    assert_eq!(shell::place("x", None, HOME), None);
}
