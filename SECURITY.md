# Security policy

## Supported versions

emberhut is pre-release. Only the latest commit on `main` gets security fixes.

## Reporting a vulnerability

Please do not open a public issue. Report it privately through GitHub's
[private vulnerability reporting](https://docs.github.com/en/code-security/security-advisories/guidance-on-reporting-and-writing-information-about-vulnerabilities/privately-reporting-a-security-vulnerability):
the **Security** tab, then **Report a vulnerability**.

Include what you found, how to reproduce it, and the impact you expect. We aim to
acknowledge a report within three working days and to agree a disclosure date with you.

## Scope

- `crates/emberhut-server`: authentication, pairing, sessions, and anything reachable over HTTP.
- `crates/emberhut-client` and `apps/desktop`: how credentials are stored and sent.

Secrets never belong in the repository. CI's scan fails on credentials gitleaks recognises,
on private and CGNAT IP addresses and email addresses (`.gitleaks.toml`), and on tracked `.env`
or key files.
