//! The outward rule: a command line's outward steps read from its verbs, never from a quoted
//! string or a heredoc's body.
#![expect(
    clippy::needless_raw_string_hashes,
    reason = "the cases are written one way, whatever each holds"
)]

use emberhut_core::guard::outward::{self, NoRemotes, Remotes};

const CASES: &[(&str, bool, &str)] = &[
    (
        r###"cd /tmp/study/parts && python3 -I - <<'EOF'
p='part1.md'
s=open(p).read()
def rep(old,new,count=1):
    global s
    assert s.count(old)>=1, old[:60]
    s=s.replace(old,new,count)
rep("the lead runs `git push`, then `gh pr merge 5`","the lead prepares the push and asks")
rep("| Push, merge, deploy | `npm publish` and `fly deploy` |","| Push, merge, deploy | held for the OK |")
rep("systemctl --user restart app; git push","(held)")
open(p,'w').write(s)
EOF
wc -w part1.md; grep -n "merge" part1.md | cut -c1-160"###,
        true,
        r###"python's code isn't read through: an interpreter's inline code is held, whatever its words"###,
    ),
    (
        r###"git log --grep push && npm test"###,
        false,
        r###"push is a grep pattern"###,
    ),
    (
        r###"echo "git push origin main""###,
        false,
        r###"a quoted string"###,
    ),
    (
        r###"printf '%s\n' 'gh pr merge 5 --squash'"###,
        false,
        r###"a single-quoted string"###,
    ),
    (
        r###"cat <<'EOF' > notes.md
Run `git push`, then `gh pr merge 5 --squash` and `npm publish`.
EOF"###,
        false,
        r###"a quoted heredoc's body"###,
    ),
    (
        r###"cat > notes.md <<EOF
- git push origin main
- systemctl --user restart app
EOF
wc -l notes.md"###,
        false,
        r###"an unquoted heredoc's text (no $(...))"###,
    ),
    (
        r###"grep -rn "git push\|gh pr merge" docs/"###,
        false,
        r###"a grep pattern"###,
    ),
    (
        r###"python3 -I - <<'EOF'
print('deploy: systemctl --user restart app; git push')
EOF"###,
        true,
        r###"python reading a heredoc runs code the reader doesn't read"###,
    ),
    (
        r###"git commit -m "push the merge, then deploy""###,
        false,
        r###"a commit message"###,
    ),
    (
        r###"gh api repos/owner/repo/pulls/5"###,
        false,
        r###"gh api read (GET)"###,
    ),
    (
        r###"gh api repos/o/r/pulls --jq '.[].number'"###,
        false,
        r###"gh api read with --jq"###,
    ),
    (
        r###"gh api graphql -f query='query { viewer { login } }'"###,
        false,
        r###"a graphql query, not a mutation"###,
    ),
    (
        r###"gh pr view 5 --json state,mergedAt"###,
        false,
        r###"gh pr view"###,
    ),
    (
        r###"gh pr list --search 'merge deploy'"###,
        false,
        r###"gh pr list"###,
    ),
    (r###"gh release list"###, false, r###"gh release list"###),
    (
        r###"systemctl --user status app"###,
        false,
        r###"a unit's status"###,
    ),
    (
        r###"systemctl --user daemon-reload"###,
        false,
        r###"daemon-reload restarts nothing"###,
    ),
    (
        r###"journalctl --user -u app -n 50 | grep restart"###,
        false,
        r###"logs"###,
    ),
    (
        r###"cat ops/deploy.cs && rg 'fly deploy|railway up' docs"###,
        false,
        r###"reading the deploy script"###,
    ),
    (
        r###"jq '.push' f.json; echo "deploy $(date)""###,
        false,
        r###"$(date) runs, but it isn't outward"###,
    ),
    (
        r###"git pull --rebase && git status"###,
        false,
        r###"git pull"###,
    ),
    (
        r###"node -e "console.log('git push')""###,
        true,
        r###"node -e runs code the reader doesn't read"###,
    ),
    (r###"git push origin main"###, true, r###"plain push"###),
    (r###"cd x && git push"###, true, r###"chained after cd"###),
    (
        r###"cd /tmp/a && git -C ../b push -u origin feat"###,
        true,
        r###"git -C"###,
    ),
    (
        r###"git -c user.name=x --no-pager push"###,
        true,
        r###"git global options"###,
    ),
    (r###"bash -c "git push""###, true, r###"bash -c"###),
    (
        r###"sh -c 'cd repo && git push origin HEAD'"###,
        true,
        r###"sh -c, chained inside"###,
    ),
    (
        r###"bash -lc "gh pr merge 12 --squash --delete-branch""###,
        true,
        r###"bash -lc gh pr merge"###,
    ),
    (r###"gh pr merge 5 --squash"###, true, r###"gh pr merge"###),
    (
        r###"gh -R example-org/ad-studio pr merge 5 --squash"###,
        true,
        r###"gh -R before the subcommand"###,
    ),
    (
        r###"gh pr create --fill --base main"###,
        true,
        r###"gh pr create"###,
    ),
    (
        r###"gh api -X PUT repos/example-org/ad-studio/pulls/5/merge"###,
        true,
        r###"a merge through the gh api, -X PUT"###,
    ),
    (
        r###"gh api --method PUT repos/o/r/pulls/5/merge -f merge_method=squash"###,
        true,
        r###"--method PUT"###,
    ),
    (
        r###"gh api -XPUT /repos/o/r/pulls/5/merge"###,
        true,
        r###"-XPUT"###,
    ),
    (
        r###"gh api repos/o/r/merges -f base=main -f head=feat"###,
        true,
        r###"fields make it a POST"###,
    ),
    (
        r###"gh api graphql -f query='mutation { mergePullRequest(input: {pullRequestId: "x"}) { clientMutationId } }'"###,
        true,
        r###"a graphql mutation"###,
    ),
    (
        r###"gh workflow run deploy.yml --ref main"###,
        true,
        r###"gh workflow run"###,
    ),
    (
        r###"echo "$(git push origin main)""###,
        true,
        r###"$(...) inside double quotes runs"###,
    ),
    (r###"x=`git push`"###, true, r###"backticks run"###),
    (
        r###"bash <<'EOF'
git push origin main
EOF"###,
        true,
        r###"a heredoc fed to a shell runs"###,
    ),
    (
        r###"ssh host 'cd ~/app && git push'"###,
        true,
        r###"ssh runs its command"###,
    ),
    (
        r###"tailscale ssh user@workstation "cd ~/app && git pull --ff-only && systemctl --user restart app-runner""###,
        true,
        r###"tailscale ssh, a unit restart"###,
    ),
    (
        r###"sudo systemctl restart nginx"###,
        true,
        r###"sudo systemctl restart"###,
    ),
    (
        r###"systemctl --user restart app"###,
        true,
        r###"a unit restart"###,
    ),
    (
        r###"systemctl --user enable --now app-runner"###,
        true,
        r###"enable --now"###,
    ),
    (
        r###"launchctl kickstart -k gui/501/com.example.runner"###,
        true,
        r###"launchctl kickstart"###,
    ),
    (
        r###"dotnet run ops/deploy.cs"###,
        true,
        r###"the app's deploy"###,
    ),
    (
        r###"cd /srv/app && PATH=$HOME/.dotnet:$PATH dotnet run ops/deploy.cs"###,
        true,
        r###"a deploy after cd, with an assignment"###,
    ),
    (r###"./deploy.sh prod"###, true, r###"a deploy script"###),
    (
        r###"npm publish --access public"###,
        true,
        r###"npm publish"###,
    ),
    (r###"npm run deploy:prod"###, true, r###"npm run deploy"###),
    (r###"npx vercel --prod"###, true, r###"vercel --prod"###),
    (r###"fly deploy -a app"###, true, r###"fly deploy"###),
    (r###"make deploy"###, true, r###"make deploy"###),
    (
        r###"env GIT_SSH_COMMAND='ssh -i k' git push"###,
        true,
        r###"env"###,
    ),
    (r###"timeout 60 git push"###, true, r###"timeout"###),
    (
        r###"false || git push --force-with-lease"###,
        true,
        r###"after ||"###,
    ),
    (r###"echo hi; git push"###, true, r###"after ;"###),
    (
        r###"git status
git push"###,
        true,
        r###"on the next line"###,
    ),
    (r###"(cd sub && git push)"###, true, r###"in a subshell"###),
    (r###"if true; then git push; fi"###, true, r###"in an if"###),
    (r###"eval "git push""###, true, r###"eval"###),
    (
        r###"xargs -n1 git push origin < branches.txt"###,
        true,
        r###"xargs"###,
    ),
    (
        r###"cat <<'EOF' > /tmp/m.md
notes
EOF
git push"###,
        true,
        r###"a push after a heredoc"###,
    ),
    (
        r###"case $x in main) git push;; esac"###,
        true,
        r###"the parser can't read case: a line it can't read is held"###,
    ),
    (
        r###"G=git; $G push"###,
        true,
        r###"a command in a variable"###,
    ),
    (r###"sh -c "$CMD""###, true, r###"sh -c of a variable"###),
    (
        r###"$(echo git) push"###,
        true,
        r###"a command from a substitution"###,
    ),
    (
        r###"echo push | xargs git"###,
        true,
        r###"xargs gives git its words"###,
    ),
    (
        r###"xargs -I{} git {} < words.txt"###,
        true,
        r###"xargs -I"###,
    ),
    (
        r###"git -c alias.s=push s"###,
        true,
        r###"a git alias set with -c"###,
    ),
    (
        r###"git s"###,
        true,
        r###"a git command that isn't git's own: an alias"###,
    ),
    (
        r###"git send-pack origin main"###,
        true,
        r###"git send-pack"###,
    ),
    (
        r###"git -c url.https://example.com/.insteadOf=https://github.com/ fetch"###,
        true,
        r###"git -c url.*.insteadOf"###,
    ),
    (
        r###"GIT_CONFIG_COUNT=1 GIT_CONFIG_KEY_0=alias.s GIT_CONFIG_VALUE_0=push git status"###,
        true,
        r###"git's settings from the environment"###,
    ),
    (
        r###"git submodule foreach 'git push'"###,
        true,
        r###"a command git runs"###,
    ),
    (
        r###"printf 'git push\n' >p.sh; sh p.sh"###,
        true,
        r###"a shell running a script file"###,
    ),
    (r###"./p.sh"###, true, r###"a script by its path"###),
    (
        r###"cat p.sh | bash"###,
        true,
        r###"a shell reading a pipe"###,
    ),
    (
        r###"python3 -c 'import os; os.system("git push")'"###,
        true,
        r###"python -c"###,
    ),
    (
        r###"node -e 'require("child_process").execSync("git push")'"###,
        true,
        r###"node -e"###,
    ),
    (
        r###"ruby -e 'system("git push")'"###,
        true,
        r###"ruby -e"###,
    ),
    (
        r###"perl -e 'system("git push")'"###,
        true,
        r###"perl -e"###,
    ),
    (
        r###"python3 tools/release_notes.py"###,
        true,
        r###"a python script"###,
    ),
    (r###"find . -exec git push \;"###, true, r###"find -exec"###),
    (
        r###"find . -name '*.sh' -execdir sh {} \;"###,
        true,
        r###"find -execdir runs a script"###,
    ),
    (
        r###"curl -X PUT -H "Authorization: Bearer $(gh auth token)" https://api.github.com/repos/o/n/pulls/1/merge"###,
        true,
        r###"a merge through the API with curl"###,
    ),
    (
        r###"curl -sd '{"base":"main","head":"feat"}' https://api.github.com/repos/o/n/merges"###,
        true,
        r###"curl with a body is a POST"###,
    ),
    (
        r###"wget --post-data='x=1' https://example.com/hook"###,
        true,
        r###"wget posting"###,
    ),
    (
        r###"http PUT api.github.com/repos/o/n/pulls/1/merge"###,
        true,
        r###"httpie PUT"###,
    ),
    (
        r###"nice nice nice nice nice nice nice nice nice nice nice nice nice nice nice nice nice nice nice nice nice git push"###,
        true,
        r###"wrappers deeper than the reader goes"###,
    ),
    (
        r###"((1)); /usr/bin/git push"###,
        true,
        r###"a line it can't take apart"###,
    ),
    (r###"/usr/bin/git push"###, true, r###"git by its path"###),
    (r###"gh my-alias"###, true, r###"a gh alias or extension"###),
    (
        r###"curl -s https://api.github.com/repos/o/n/pulls/1"###,
        false,
        r###"a read through the API"###,
    ),
    (
        r###"curl -s -X POST -H 'Content-Type: application/json' -d '{}' http://localhost:8080/api"###,
        false,
        r###"a write to this machine"###,
    ),
    (
        r###"git -c user.name=x commit -m "the fix""###,
        false,
        r###"a git setting that changes nothing it runs"###,
    ),
    (
        r###"command -v git && git --version"###,
        false,
        r###"looking git up"###,
    ),
    (
        r###"find . -name '*.rs' -exec grep -l parse {} +"###,
        false,
        r###"find -exec of a command whose words don't matter"###,
    ),
    (
        r###"ls | xargs wc -l"###,
        false,
        r###"xargs of a command whose words don't matter"###,
    ),
    (
        r###"python3 --version"###,
        false,
        r###"an interpreter's version"###,
    ),
];

const HOME: &str = "/home/user";

#[test]
fn every_case_reads_as_expected() {
    assert!(outward::patterns_compile());
    let wrong: Vec<String> = CASES
        .iter()
        .filter(|(command, outward, _)| outward::is_outward(command, HOME) != *outward)
        .map(|(command, outward, why)| {
            format!(
                "{}: {why}: {}",
                if *outward { "missed" } else { "caught" },
                command.chars().take(120).collect::<String>()
            )
        })
        .collect();
    assert!(
        wrong.is_empty(),
        "{} of {} wrong:\n{}",
        wrong.len(),
        CASES.len(),
        wrong.join("\n")
    );
}

/// A folder whose remote is one repository.
struct OneRepo {
    dir: &'static str,
    repo: &'static str,
}

impl Remotes for OneRepo {
    fn home(&self) -> &str {
        HOME
    }

    fn remotes(&self, dir: &str) -> Option<Vec<String>> {
        (dir == self.dir).then(|| vec![self.repo.to_owned()])
    }
}

#[test]
fn a_repository_always_ok_lets_its_own_steps_through_and_no_other() {
    let here = OneRepo {
        dir: "/srv/app",
        repo: "owner/app",
    };
    let always = |repo: &str| repo == "owner/app";
    assert!(outward::always_ok(
        "cd /srv/app && git push origin main && gh pr merge 5 --squash",
        "/",
        &here,
        &always
    ));
    assert!(outward::always_ok(
        "gh api -X PUT repos/owner/app/pulls/5/merge",
        "/",
        &here,
        &always
    ));
    assert!(
        !outward::always_ok("(cd /srv/app && git status); git push", "/", &here, &always),
        "the push runs in /"
    );
    assert!(
        !outward::always_ok("ssh host 'cd /srv/app && git push'", "/", &here, &always),
        "another machine's folder can't be told"
    );
    assert!(
        !outward::always_ok(
            "gh api -X POST user/repos -f name=x",
            "/srv/app",
            &here,
            &always
        ),
        "a write with no repository"
    );
    assert!(!outward::is_outward("gh api user", HOME), "a gh api read");
}

#[test]
fn a_step_knows_where_it_runs_and_what_it_is() {
    let steps = outward::steps(
        "cd /tmp/a && git -C ../b push -u origin feat",
        Some("/"),
        false,
        &NoRemotes(HOME.to_owned()),
    )
    .unwrap();
    assert_eq!(steps.len(), 1);
    assert_eq!(steps[0].what, "git push");
    assert_eq!(steps[0].dir.as_deref(), Some("/tmp/b"));
    let steps = outward::steps(
        "gh -R example-org/ad-studio pr merge 5 --squash",
        None,
        false,
        &NoRemotes(HOME.to_owned()),
    )
    .unwrap();
    assert_eq!(steps[0].what, "gh pr merge");
    assert_eq!(
        steps[0].repos,
        Some(vec!["example-org/ad-studio".to_owned()])
    );
}

#[test]
fn the_owners_words_name_an_outward_step_unless_they_say_no() {
    assert_eq!(
        outward::named("fix the importer, merge it and deploy").as_deref(),
        Some("merge")
    );
    assert_eq!(
        outward::named("put it live tonight").as_deref(),
        Some("put it live")
    );
    assert_eq!(outward::named("don't merge it yet"), None);
    assert_eq!(outward::named("do it end to end"), None);
    assert_eq!(outward::named(""), None);
    assert_eq!(
        outward::named("Fix the importer. Then merge it.").as_deref(),
        Some("merge")
    );
    assert_eq!(
        outward::named("DEPLOY it tonight").as_deref(),
        Some("deploy")
    );
}

#[test]
fn a_no_a_wait_a_condition_or_a_question_names_no_outward_step() {
    for said in [
        "Don\u{2019}t merge it yet",
        "Don\u{2018}t merge",
        "don\u{2bc}t push",
        "DON'T MERGE",
        "hold off on merging",
        "wait before you push",
        "skip the deploy for now",
        "stop, don't push",
        "not yet, merge it later",
        "merge it if the tests pass",
        "push it once CI is green",
        "why did the deploy fail?",
        "did the merge go through",
        "can it be deployed?",
        "the deploy failed?!",
    ] {
        assert_eq!(outward::named(said), None, "{said}");
    }
}

#[test]
fn every_held_step_it_cannot_be_sure_of_says_why_and_has_no_repository() {
    let read = |command: &str| {
        outward::steps(
            command,
            Some("/srv/app"),
            false,
            &NoRemotes(HOME.to_owned()),
        )
        .unwrap()
    };
    let cases = [
        ("$G push", outward::unsure::VERB),
        ("./p.sh", outward::unsure::SCRIPT),
        ("python3 -c 'print(1)'", outward::unsure::CODE),
        ("echo push | xargs git", outward::unsure::PIPED),
        ("git -c alias.s=push s", outward::unsure::GIT),
        ("git rebase -x 'git push' main", outward::unsure::GIT_RUNS),
        ("gh my-alias", outward::unsure::GH),
        (
            "curl -X DELETE https://example.com/x",
            outward::unsure::HTTP,
        ),
    ];
    for (command, what) in cases {
        let steps = read(command);
        assert_eq!(steps.len(), 1, "{command}");
        assert_eq!(steps[0].what, what, "{command}");
        assert_eq!(
            steps[0].repos, None,
            "{command}: never an always-OK repository"
        );
    }
    let deep = format!("{}git push", "nice ".repeat(outward::WRAPPERS));
    assert!(outward::steps(&deep, None, false, &NoRemotes(HOME.to_owned())).is_err());
    assert!(outward::is_outward(&deep, HOME));
    let here = OneRepo {
        dir: "/srv/app",
        repo: "owner/app",
    };
    assert!(
        !outward::always_ok(
            "curl -X PUT https://api.github.com/repos/other/repo/pulls/1/merge",
            "/srv/app",
            &here,
            &|repo| repo == "owner/app"
        ),
        "an HTTP write's repository is never told from the folder"
    );
    assert_eq!(
        outward::repo_name("git@github.com:owner/app.git").as_deref(),
        Some("owner/app")
    );
    assert_eq!(
        outward::repo_name("https://github.com/owner/app").as_deref(),
        Some("owner/app")
    );
}

/// Whether each line of `lines` is held, as `outward` says.
fn each_reads(lines: &[&str], outward: bool) {
    let wrong: Vec<&str> = lines
        .iter()
        .copied()
        .filter(|line| outward::is_outward(line, HOME) != outward)
        .collect();
    assert!(
        wrong.is_empty(),
        "{} {}:\n{}",
        if outward { "not held" } else { "held" },
        wrong.len(),
        wrong.join("\n")
    );
}

#[test]
fn a_shell_or_a_replaced_word_xargs_runs_is_held() {
    each_reads(
        &[
            r#"echo "'git push'" | xargs sh -c"#,
            "echo 'git push' | xargs -I{} sh -c {}",
            "echo 'git push' | xargs -I % bash -c %",
            "echo git | xargs -I{} {} push",
            "echo git | xargs -i {} push",
            "echo git | xargs --replace=@ @ push",
            "echo push | xargs -0I{} git {}",
            r"find . -name x -exec sh -c 'echo {}' \;",
        ],
        true,
    );
    each_reads(
        &["ls | xargs -I{} wc -l {}", "ls | xargs -n 1 wc -l"],
        false,
    );
}

#[test]
fn a_wrapper_xargs_runs_with_no_command_of_its_own_is_held() {
    each_reads(
        &[
            "echo 'git push' | xargs env",
            "echo 'git push' | xargs env -i",
            "echo 'git push' | xargs nice",
            "echo 'git push' | xargs sudo",
            "echo 'git push' | xargs sudo -u build",
            "echo 'git push' | xargs timeout 60",
            "echo 'git push' | xargs setsid",
            "echo 'git push' | xargs nohup",
            "echo 'git push' | xargs uv run",
            "echo 'git push' | xargs npm exec --",
            "echo 'git push' | xargs npx",
            "echo 'git push' | xargs -0 xargs",
        ],
        true,
    );
    each_reads(
        &[
            "ls | xargs",
            "ls | xargs -0",
            "ls | xargs nice wc -l",
            "ls | xargs env wc -l",
        ],
        false,
    );
}

#[test]
fn env_flock_and_tmux_code_in_any_spelling_is_read_as_shell_code() {
    each_reads(
        &[
            "env -S'git push'",
            "env -iS'git push'",
            "env -iS 'git push'",
            "env --split-string='git push'",
            "env --split='git push'",
            "echo x | xargs env -S'git push'",
            "flock /tmp/l --command 'git push'",
            "flock -w 5 /tmp/l --command 'git push'",
            "tmux display-message -p '#(git push)'",
            "tmux list-windows -F '#(git push)'",
            "tmux new -d 'echo #(git push)'",
        ],
        true,
    );
    each_reads(
        &[
            "env -S'cargo test'",
            "env --split-string='cargo test'",
            "env -uSECRET cargo test",
            "flock /tmp/l --command 'cargo build'",
            "tmux display-message -p '#{session_name}'",
            "tmux list-windows -F '#{window_name}'",
        ],
        false,
    );
}

#[test]
fn a_leading_word_or_a_wrapper_by_its_system_path_is_read_by_its_name() {
    each_reads(
        &[
            "/usr/bin/time git push",
            "/usr/bin/time -o out.txt git push",
            "/usr/bin/nohup git push",
            "/usr/bin/env git push",
            "/usr/bin/sudo -u build git push",
            "/bin/sh -c 'git push'",
            // Anywhere else it is a script that only shares the name.
            "./time ls",
            "./env ls",
        ],
        true,
    );
    each_reads(&["/usr/bin/time cargo test"], false);
}

#[test]
fn every_wrapper_is_read_through_to_the_command_it_runs() {
    each_reads(
        &[
            "setsid git push",
            "setsid -f git push",
            "taskset -c 0 git push",
            "taskset 0x1 git push",
            "chrt -r 10 git push",
            "strace -f git push",
            "strace -f -o trace.txt git push",
            "ltrace -o calls.txt git push",
            "systemd-run --user git push",
            "systemd-run --user --unit push -p Nice=5 git push",
            "unshare -r git push",
            "chroot /srv/root git push",
            "chroot --userspec build /srv/root git push",
            "runuser -u build -- git push",
            "runuser -u build git push",
            "firejail --noprofile git push",
            "nice -n 5 git push",
            "ionice -c 3 git push",
            "stdbuf -oL git push",
            "timeout 60 git push",
            "timeout -s KILL 60 git push",
            "env FOO=1 git push",
            "env -i git push",
            "parallel git push ::: a b",
            "parallel ::: 'git push'",
            "sudo -s 'git push'",
            "sudo -i git push",
            "sudo -s <<'EOF'\ngit push\nEOF",
            "chroot /srv/root <<'EOF'\ngit push\nEOF",
        ],
        true,
    );
    each_reads(
        &[
            "setsid cargo test",
            "taskset -c 0 cargo build",
            "strace -f -o trace.txt ls",
            "timeout 60 cargo test",
            "nice -n 5 cargo build",
            "systemd-run --user --unit build cargo build",
        ],
        false,
    );
}

#[test]
fn code_su_script_tmux_screen_or_trap_runs_is_read_as_shell_code() {
    each_reads(
        &[
            "su -c 'git push'",
            "su build -c 'git push'",
            "su - build -c 'git push'",
            "su --command='git push' build",
            "su -lc 'git push' build",
            "runuser -l build -c 'git push'",
            "su build <<'EOF'\ngit push\nEOF",
            "su build ./push.sh",
            "script -qc 'git push' /dev/null",
            "script -c 'git push' log.txt",
            "script --command 'git push' log.txt",
            "script -q /dev/null git push",
            "tmux new -d 'git push'",
            "tmux new-session -d -s push 'git push origin main'",
            "tmux send-keys -t main 'git push' Enter",
            "tmux if-shell true 'run-shell \"git push\"'",
            "tmux new -d \"$CMD\"",
            "screen -dmS push git push",
            "screen -dmS push bash -c 'git push'",
            "screen -S main -X stuff 'git push\n'",
            "trap 'git push' EXIT",
            "trap -- 'git push' EXIT INT",
            "trap \"$CMD\" EXIT",
        ],
        true,
    );
    each_reads(
        &[
            "su build -c 'cargo test'",
            "script -qc 'cargo test' /dev/null",
            "tmux new -d -s dev 'npm run dev'",
            "tmux ls",
            "screen -ls",
            "trap 'rm -f /tmp/lock' EXIT",
            "trap - EXIT",
            "trap -p",
        ],
        false,
    );
}

#[test]
fn a_package_runner_exec_is_read_like_npx() {
    each_reads(
        &[
            "npm exec -- git push",
            "npm exec git push",
            "npm x -- git push",
            "npm exec -c 'git push'",
            "npx -c 'git push'",
            "npx --call='git push'",
            "pnpm exec git push",
            "pnpm exec -c git push",
            "yarn exec git push",
            "yarn exec 'git push'",
            "bun exec 'git push'",
            "bun x git push",
            "uv run git push",
            "uv run --with requests git push",
            "uv tool run git push",
            "uvx --from tool git push",
            "uv run tools/push.py",
            "uv run push.py",
            "uv run -m pusher",
            "poetry run git push",
            "pipx run --spec tool git push",
        ],
        true,
    );
    each_reads(
        &[
            "npm exec -- eslint .",
            "pnpm exec vitest run",
            "yarn exec 'tsc --noEmit'",
            "uv run pytest -q",
            "poetry run pytest",
        ],
        false,
    );
}

#[test]
fn a_push_or_a_gh_write_in_a_commands_own_words_is_held_whatever_runs_it() {
    each_reads(
        &[
            "some-wrapper --opt value git push",
            "some-wrapper git -C . push origin main",
            "some-wrapper -x gh pr merge 5 --squash",
            "some-wrapper gh pr create --fill",
            "some-wrapper gh release create v1",
            "some-wrapper gh -R owner/app pr merge 5",
            "some-wrapper gh api -X PUT repos/owner/app/pulls/5/merge",
            "some-wrapper npm publish",
            "strace -X verbose git push",
            "echo git push",
        ],
        true,
    );
    // A heredoc's body is never a command's own words, nor is one word that only says it.
    each_reads(
        &[
            "cat <<'EOF' > notes.md\ngit push\ngh pr merge 5\nEOF",
            "some-wrapper 'git push'",
            "some-wrapper git log --grep push",
            "some-wrapper gh pr view 5",
        ],
        false,
    );
    let steps = outward::steps(
        "some-wrapper --opt value git push",
        Some("/srv/app"),
        false,
        &OneRepo {
            dir: "/srv/app",
            repo: "owner/app",
        },
    )
    .unwrap();
    assert_eq!(steps.len(), 1);
    assert_eq!(steps[0].what, "git push");
    assert_eq!(steps[0].repos, None, "never an always-OK repository");
}
