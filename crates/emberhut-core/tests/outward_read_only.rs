//! Read-only commands run without an ask; pushes, merges, deploys, writes to other machines
//! and paid API calls are still held. The first commands here are ones the reader once held as
//! outward though they only read: a disk-usage scan, an HTML file read and printed, an API's
//! models listed with a key from a file, a JSON file read and printed, a secret scan's report
//! read and printed.
#![expect(
    clippy::needless_raw_string_hashes,
    reason = "the commands are written one way, whatever each holds"
)]

use emberhut_core::guard::outward::{self, Reading};

const HOME: &str = "/home/user";

/// Commands that only read (and write only in their own folder): they run with no ask.
const MUST_PASS: &[&str] = &[
    // Sizes of every folder in the home, twelve at a time, into the task's own folder.
    r###"cd /home/user; ls -A | wc -l; cd /home/user/work/task-1; (cd /home/user; ls -A | xargs -P 12 -I{} sh -c 'timeout 1500 du -sx --block-size=1M "{}" 2>/dev/null || echo "TIMEOUT {}"' > /home/user/work/task-1/top.txt) >/dev/null 2>&1 &
echo started"###,
    // Notes and a listing, then an HTML file's text.
    r###"cd ~/work; cat notes/numbers-watch.md; ls -R plan/ask plan/check plan/lw | head -40; python3 -I -c "
import re,html,sys
t=open('plan/ad-plan.html').read()
t=re.sub(r'<style.*?</style>|<script.*?</script>','',t,flags=re.S)
t=re.sub(r'<[^>]+>',' ',t); t=html.unescape(re.sub(r'\s+',' ',t))
print(t[:6000])""###,
    // An API's models and plan, its key read from a file (never printed).
    r###"K=$(cat ~/.config/voice/api.key); curl -s https://api.voice.example/v1/models -H "xi-api-key: $K" | python3 -c "import json,sys;[print(m['model_id'],m.get('name'),m.get('can_do_text_to_speech')) for m in json.load(sys.stdin)]"; curl -s https://api.voice.example/v1/user/subscription -H "xi-api-key: $K" | python3 -c "import json,sys;d=json.load(sys.stdin);print(d.get('tier'),d.get('character_count'),d.get('character_limit'),d.get('next_character_count_reset_unix'))""###,
    // A JSON file's entries since a date.
    r###"F=/home/user/.cache/tool-results/corrections.txt; python3 -I -c "
import json,sys
d=json.load(open(sys.argv[1]))
for c in d['corrections']:
  if c['at']>='2026-11-21':
    print('#',c['id'],c['at'][:16],c['source'],'| lesson:',c['lesson'])
    print('  SAID:',c['said'][:400].replace('\n',' '))
    print('  WRONG:',(c['wrong'] or '')[:500].replace('\n',' '))
print('LESSONS',len(d['lessons']))
" $F"###,
    // A secret scan over a checkout, its report read back.
    r###"cd /home/user/work/task-2/src; gitleaks dir . --no-banner -r ../gitleaks.json --redact=40 2>&1 | tail -5; python3 -I -c "
import json;d=json.load(open('../gitleaks.json'))
for f in d: print(f['RuleID'],f['File'],f['StartLine'],f['Match'][:90])""###,
    r###"grep -rn TODO src | head -20; find . -name '*.rs' | wc -l; du -sh * | sort -h"###,
    r###"cat a.txt b.txt > /home/user/work/task-1/both.txt"###,
    r###"curl -s https://api.github.com/repos/owner/app/pulls | python3 -c "import json,sys; print(len(json.load(sys.stdin)))""###,
    r###"gh api repos/owner/app/pulls --jq '.[].title'"###,
    r###"python3 -c "import json,sys; print(json.dumps(json.load(open(sys.argv[1])), indent=2))" data.json"###,
    r###"find . -name '*.log' -exec sh -c 'tail -n 5 "$1"' _ {} \;"###,
    r###"ssh prod 'journalctl -u app -n 50 --no-pager'"###,
    r###"ssh prod 'systemctl status app; docker ps; df -h'"###,
    r###"scp prod:/var/log/app.log ./logs/"###,
    r###"ssh prod 'cd /srv/app && git log --oneline -5 && sort -u .env.example | head'"###,
];

/// Outward steps the reader knows: the owner's words must cover them.
const MUST_ASK_OUTWARD: &[&str] = &[
    "git push origin main",
    "cd /srv/app && git push --force",
    "gh pr merge 12 --squash",
    "gh pr create --fill",
    "gh api -X PUT repos/owner/app/pulls/1/merge",
    "gh release create v1.2.0",
    "npm publish",
    "cargo publish",
    "twine upload dist/*",
    "docker push registry.example.com/app:1",
    "docker buildx build --push -t registry.example.com/app:1 .",
    "fly deploy",
    "vercel --prod",
    "wrangler deploy",
    "kubectl apply -f prod.yaml",
    "terraform apply -auto-approve",
    "systemctl --user restart app",
    "ssh prod 'sudo systemctl restart app'",
    "ssh prod 'cd /srv/app && git push'",
    "scp build.tar prod:/srv/app/",
    "rsync -az dist/ deploy@prod:/srv/www/",
    "ls | xargs -I{} sh -c 'cd \"{}\" && git push'",
];

/// Commands the reader can't be sure of: held, the assistant asked before the owner.
const MUST_ASK_UNSURE: &[&str] = &[
    "ssh prod 'rm -rf /srv/app/releases/old'",
    r#"ssh prod 'psql -c "delete from users"'"#,
    "ssh prod 'echo x > /etc/app.conf'",
    "ssh prod 'sort -o /etc/hosts list.txt'",
    "ssh prod 'cd /srv/app && git pull'",
    "ssh prod < migrate.sh",
    "cat migrate.sh | ssh prod",
    r#"curl -X POST https://api.voice.example/v1/text-to-speech/abc -d '{"text":"hi"}'"#,
    r#"curl -s https://api.llm.example/v1/chat/completions -H "Authorization: Bearer $K" -d @req.json"#,
    r#"python3 -c "import requests; requests.post('https://api.example.com/v1/charge')""#,
    r#"python3 -c "import subprocess; subprocess.run(['git', 'push'])""#,
    r#"python3 -c "open('/srv/app/config.json', 'w').write('{}')""#,
    r#"python3 -c "import os; os.system('make deploy')""#,
    "python3 tools/sync.py",
    "./deploy.sh",
    "bash scripts/release.sh",
    "ls | xargs -I{} sh -c '{}'",
    "ls | xargs -I{} sh -c 'du {}'",
    r#"ls | xargs -I{} sh -c "du \"$(echo {})\"""#,
    r#"eval "$CMD""#,
    r#"for f in *.json; do python3 -c "import json;print(json.load(open('$f')))"; done"#,
];

#[test]
fn read_only_commands_run_with_no_ask() {
    let held: Vec<String> = MUST_PASS
        .iter()
        .filter(|command| outward::reading(command, HOME) != Reading::Clear)
        .map(|command| format!("{:?}: {}", outward::reading(command, HOME), command))
        .collect();
    assert!(held.is_empty(), "held:\n{}", held.join("\n\n"));
}

#[test]
fn pushes_merges_and_deploys_still_ask_the_owner() {
    let missed: Vec<String> = MUST_ASK_OUTWARD
        .iter()
        .filter(|command| !matches!(outward::reading(command, HOME), Reading::Outward(_)))
        .map(|command| format!("{:?}: {command}", outward::reading(command, HOME)))
        .collect();
    assert!(missed.is_empty(), "not outward:\n{}", missed.join("\n"));
    assert!(
        MUST_ASK_OUTWARD
            .iter()
            .all(|c| outward::is_outward(c, HOME))
    );
}

#[test]
fn what_it_cannot_be_sure_of_is_held() {
    let missed: Vec<String> = MUST_ASK_UNSURE
        .iter()
        .filter(|command| !matches!(outward::reading(command, HOME), Reading::Unsure(_)))
        .map(|command| format!("{:?}: {command}", outward::reading(command, HOME)))
        .collect();
    assert!(
        missed.is_empty(),
        "not held as unsure:\n{}",
        missed.join("\n")
    );
    assert!(MUST_ASK_UNSURE.iter().all(|c| outward::is_outward(c, HOME)));
}

#[test]
fn a_line_it_cannot_take_apart_is_unsure() {
    assert!(matches!(
        outward::reading("echo $((1 + 2)); case x in y) ;; esac", HOME),
        Reading::Unsure(_)
    ));
}
