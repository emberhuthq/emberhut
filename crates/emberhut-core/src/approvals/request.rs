//! Approval requests: a new one as an agent's step asks for it, one imported from an earlier
//! installation's history, and one as it is stored.

use jiff::Timestamp;

use super::approver::Approver;
use super::kind::ActionKind;
use super::payload::{FileDigest, Payload, PayloadHash, Sha256Digest, count, put};
use super::state::{self, DecidedVia, Decision, RequestState};
use crate::agents::AgentId;
use crate::task::TaskId;

/// The longest target, in characters.
pub const TARGET_MAX: usize = 500;
/// The longest summary, in characters.
pub const SUMMARY_MAX: usize = 300;
/// The longest text, in characters.
pub const TEXT_MAX: usize = 40_000;
/// The most files one request carries.
pub const FILES_MAX: usize = 20;
/// The longest file name, in characters.
pub const FILE_NAME_MAX: usize = 255;
/// The longest reason or source reference, in characters.
pub const NOTE_MAX: usize = 500;

/// The version tag of an imported request's hash: never a live payload's, so an imported row
/// can never match a step that uses an approval.
pub const IMPORT_TAG: &str = "emberhut-approval-import-v1";

/// Why a request can't be made (or imported) as given.
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
#[non_exhaustive]
pub enum RequestError {
    /// A field that must say something is blank.
    #[error("{0} must not be blank")]
    Blank(&'static str),
    /// A field is longer than it may be.
    #[error("{field} is longer than {max} characters")]
    TooLong {
        /// The field.
        field: &'static str,
        /// Its longest.
        max: usize,
    },
    /// More than [`FILES_MAX`] files.
    #[error("a request carries at most {FILES_MAX} files")]
    TooManyFiles,
    /// A file name that is blank, too long, a path, or holds a control or invisible character.
    #[error(
        "files[{0}]: a file's name is 1 to {FILE_NAME_MAX} characters, without '/', control or \
         invisible characters"
    )]
    FileName(usize),
    /// A request whose approver is nobody: such a step needs no request.
    #[error("nobody approves this kind: the step needs no request")]
    NoApprover,
    /// A fixed-core request for anyone but the owner.
    #[error("{0} is the fixed core's: only the owner approves it")]
    FixedCore(ActionKind),
    /// A payment without its amount, or an amount on anything but a payment.
    #[error("a payment carries its amount, and only a payment does")]
    Amount,
    /// An imported request in a state it could still move on from.
    #[error("an imported request is used, expired, rejected or withdrawn, never {0}")]
    ImportedState(RequestState),
    /// An imported request that says it was used, without its use.
    #[error("an imported used request needs when and by what it was used")]
    ImportedUse,
}

fn text_field(text: &str, field: &'static str, max: usize) -> Result<(), RequestError> {
    if text.trim().is_empty() {
        return Err(RequestError::Blank(field));
    }
    if text.chars().count() > max {
        return Err(RequestError::TooLong { field, max });
    }
    Ok(())
}

fn note_field(text: Option<&str>, field: &'static str) -> Result<(), RequestError> {
    text.map_or(Ok(()), |text| text_field(text, field, NOTE_MAX))
}

fn file_name(index: usize, name: &str) -> Result<(), RequestError> {
    let fits = !name.trim().is_empty()
        && name.chars().count() <= FILE_NAME_MAX
        && !name.contains('/')
        && !name
            .chars()
            .any(|c| c.is_control() || super::shown::is_invisible(c));
    if fits {
        Ok(())
    } else {
        Err(RequestError::FileName(index))
    }
}

/// The checks every request shares, new or imported.
fn check_common(
    target: &str,
    summary: &str,
    text: Option<&str>,
    kind: ActionKind,
    approver: &Approver,
    amount_cents: Option<u64>,
    file_names: &[&str],
) -> Result<(), RequestError> {
    text_field(target, "target", TARGET_MAX)?;
    text_field(summary, "summary", SUMMARY_MAX)?;
    if let Some(text) = text
        && text.chars().count() > TEXT_MAX
    {
        return Err(RequestError::TooLong {
            field: "text",
            max: TEXT_MAX,
        });
    }
    if file_names.len() > FILES_MAX {
        return Err(RequestError::TooManyFiles);
    }
    for (index, name) in file_names.iter().enumerate() {
        file_name(index, name)?;
    }
    if *approver == Approver::Nobody {
        return Err(RequestError::NoApprover);
    }
    if kind.is_fixed_core() && *approver != Approver::Owner {
        return Err(RequestError::FixedCore(kind));
    }
    if (kind == ActionKind::Payment) != amount_cents.is_some() {
        return Err(RequestError::Amount);
    }
    Ok(())
}

/// A request as an agent's step asks for it, before it is stored.
///
/// Its kind is the one after [`super::fixed_core::raise`], its approver the one
/// [`super::fixed_core::effective_approver`] names now; its files were read by the server.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct NewApproval {
    /// The agent that acts.
    pub agent: AgentId,
    /// The kind of action.
    pub kind: ActionKind,
    /// Where it goes.
    pub target: String,
    /// One line of the agent's.
    pub summary: String,
    /// The exact words, if any.
    pub text: Option<String>,
    /// Its files.
    pub files: Vec<FileDigest>,
    /// Who approves it, as the policy named them when it was made.
    pub approver: Approver,
    /// A payment's amount, in cents.
    pub amount_cents: Option<u64>,
    /// The task whose step asked, if one did.
    pub task: Option<TaskId>,
}

impl NewApproval {
    /// What its approval covers.
    #[must_use]
    pub fn payload(&self) -> Payload<'_> {
        Payload {
            agent: &self.agent,
            kind: self.kind,
            target: &self.target,
            text: self.text.as_deref(),
            amount_cents: self.amount_cents,
            files: &self.files,
        }
    }

    /// Its payload's hash.
    #[must_use]
    pub fn payload_hash(&self) -> PayloadHash {
        self.payload().hash()
    }

    /// Checks the request: a target and a summary that say something, nothing too long, file
    /// names that are names, an approver (never nobody, and the owner for the fixed core), and
    /// an amount exactly when it is a payment.
    ///
    /// # Errors
    ///
    /// [`RequestError`].
    pub fn validate(&self) -> Result<(), RequestError> {
        let names: Vec<&str> = self.files.iter().map(|f| f.name.as_str()).collect();
        check_common(
            &self.target,
            &self.summary,
            self.text.as_deref(),
            self.kind,
            &self.approver,
            self.amount_cents,
            &names,
        )
    }
}

/// A file of a stored request. An imported one may not know its size.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ApprovalFile {
    /// Its name.
    pub name: String,
    /// Its size in bytes, when known.
    pub size: Option<u64>,
    /// The SHA-256 of its bytes.
    pub sha256: Sha256Digest,
}

impl From<FileDigest> for ApprovalFile {
    fn from(file: FileDigest) -> Self {
        Self {
            name: file.name,
            size: Some(file.size),
            sha256: file.sha256,
        }
    }
}

/// A decision, as it was made.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DecisionRecord {
    /// Yes or no.
    pub decision: Decision,
    /// When.
    pub at: Timestamp,
    /// Who, as the channel proved it: the device, the Slack user, the link, or what the
    /// approver's app reported.
    pub by: String,
    /// Through which channel.
    pub via: DecidedVia,
}

/// An approval's one use.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct UseRecord {
    /// When.
    pub at: Timestamp,
    /// The task or tool that used it.
    pub by: String,
}

/// Where a request was posted to its approver in Slack, so the message can be updated.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SlackPost {
    /// The DM's channel id.
    pub channel: String,
    /// The message's timestamp, Slack's id for it.
    pub ts: String,
}

/// A request from an earlier installation's history: already over, and never usable.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ImportedApproval {
    /// The agent that acted.
    pub agent: AgentId,
    /// The kind of action.
    pub kind: ActionKind,
    /// Where it went.
    pub target: String,
    /// Its one line.
    pub summary: String,
    /// The exact words, if any.
    pub text: Option<String>,
    /// Its files.
    pub files: Vec<ApprovalFile>,
    /// Who approved it.
    pub approver: Approver,
    /// A payment's amount, in cents.
    pub amount_cents: Option<u64>,
    /// The task whose step asked, if known.
    pub task: Option<TaskId>,
    /// Where it ended: used, expired, rejected or withdrawn.
    pub state: RequestState,
    /// Why it ended there (`imported: never answered`).
    pub state_reason: Option<String>,
    /// Where it came from (`import:<table>#<id>`).
    pub source_ref: String,
    /// When it was made.
    pub created_at: Timestamp,
    /// Its decision, if it had one.
    pub decided: Option<DecisionRecord>,
    /// Its use, if it was used.
    pub used: Option<UseRecord>,
    /// When it expired or was withdrawn, if known.
    pub closed_at: Option<Timestamp>,
}

impl ImportedApproval {
    /// Checks it as [`NewApproval::validate`] does (an amount aside: old payments may lack
    /// one), and that it is over: used (with its use), expired, rejected or withdrawn.
    ///
    /// # Errors
    ///
    /// [`RequestError`].
    pub fn validate(&self) -> Result<(), RequestError> {
        let names: Vec<&str> = self.files.iter().map(|f| f.name.as_str()).collect();
        let amount = if self.kind == ActionKind::Payment {
            Some(self.amount_cents.unwrap_or_default())
        } else {
            self.amount_cents
        };
        check_common(
            &self.target,
            &self.summary,
            self.text.as_deref(),
            self.kind,
            &self.approver,
            amount,
            &names,
        )?;
        note_field(self.state_reason.as_deref(), "state_reason")?;
        text_field(&self.source_ref, "source_ref", NOTE_MAX)?;
        if !self.state.is_final() {
            return Err(RequestError::ImportedState(self.state));
        }
        if self.state == RequestState::Used && self.used.is_none() {
            return Err(RequestError::ImportedUse);
        }
        Ok(())
    }

    /// Its hash: the payload's parts as a live request's are encoded, under
    /// [`IMPORT_TAG`], with each file's size marked known or not. The tag keeps it apart from
    /// every live payload's hash.
    #[must_use]
    pub fn payload_hash(&self) -> PayloadHash {
        let mut out = Vec::new();
        put(&mut out, IMPORT_TAG.as_bytes());
        put(&mut out, self.agent.as_str().as_bytes());
        put(&mut out, self.kind.as_str().as_bytes());
        put(&mut out, self.target.as_bytes());
        optional(&mut out, self.text.as_deref().map(str::as_bytes));
        optional(
            &mut out,
            self.amount_cents
                .map(u64::to_be_bytes)
                .as_ref()
                .map(|b| &b[..]),
        );
        let mut files: Vec<&ApprovalFile> = self.files.iter().collect();
        files.sort_by(|a, b| (a.sha256, &a.name, a.size).cmp(&(b.sha256, &b.name, b.size)));
        out.extend_from_slice(&count(files.len()).to_be_bytes());
        for file in files {
            out.extend_from_slice(file.sha256.as_bytes());
            optional(
                &mut out,
                file.size.map(u64::to_be_bytes).as_ref().map(|b| &b[..]),
            );
            put(&mut out, file.name.as_bytes());
        }
        Sha256Digest::of(&out)
    }
}

fn optional(out: &mut Vec<u8>, bytes: Option<&[u8]>) {
    match bytes {
        Some(bytes) => {
            out.push(1);
            put(out, bytes);
        }
        None => out.push(0),
    }
}

/// A request as it is stored.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ApprovalRequest {
    /// Its id.
    pub id: crate::ApprovalId,
    /// The agent that acts.
    pub agent: AgentId,
    /// The kind of action.
    pub kind: ActionKind,
    /// Where it goes.
    pub target: String,
    /// One line of the agent's.
    pub summary: String,
    /// The exact words, if any.
    pub text: Option<String>,
    /// Its files, in the order they were given.
    pub files: Vec<ApprovalFile>,
    /// What its approval covers.
    pub payload_hash: PayloadHash,
    /// Who approves it, as the policy named them when it was made.
    pub approver: Approver,
    /// A payment's amount, in cents.
    pub amount_cents: Option<u64>,
    /// The task whose step asked, if one did.
    pub task: Option<TaskId>,
    /// Where it is in its life.
    pub state: RequestState,
    /// Why it was expired, withdrawn or rejected, when that was said.
    pub state_reason: Option<String>,
    /// Where an imported request came from; none for a live one.
    pub source_ref: Option<String>,
    /// When it was made.
    pub created_at: Timestamp,
    /// Its decision.
    pub decided: Option<DecisionRecord>,
    /// Its use.
    pub used: Option<UseRecord>,
    /// When it expired or was withdrawn.
    pub closed_at: Option<Timestamp>,
    /// Where it was posted to its approver in Slack.
    pub shown: Option<SlackPost>,
}

impl ApprovalRequest {
    /// When it expires; none once it can't.
    #[must_use]
    pub fn expires_at(&self) -> Option<Timestamp> {
        state::expires_at(
            self.state,
            self.created_at,
            self.decided.as_ref().map(|d| d.at),
        )
    }

    /// Whether it still waits for its answer at `now`.
    #[must_use]
    pub fn is_open(&self, now: Timestamp) -> bool {
        self.state == RequestState::Pending && self.expires_at().is_some_and(|at| now < at)
    }

    /// Whether it may be used at `now` for a step with this hash, by an approver the agent's
    /// policy names now (`approver_now`): approved, unused, unexpired, imported never.
    #[must_use]
    pub fn is_usable(
        &self,
        now: Timestamp,
        payload_hash: &PayloadHash,
        approver_now: &Approver,
    ) -> bool {
        self.state == RequestState::Approved
            && self.source_ref.is_none()
            && self.used.is_none()
            && self.expires_at().is_some_and(|at| now < at)
            && self.payload_hash == *payload_hash
            && self.approver == *approver_now
    }
}

#[cfg(test)]
mod tests {
    use jiff::SignedDuration;

    use super::*;
    use crate::approvals::ApproverId;

    fn new() -> NewApproval {
        NewApproval {
            agent: AgentId::parse("client-a").unwrap(),
            kind: ActionKind::Post,
            target: "example blog".to_owned(),
            summary: "The launch post".to_owned(),
            text: Some("We shipped it.".to_owned()),
            files: vec![FileDigest::of("cover.png", b"png")],
            approver: Approver::External(ApproverId::parse("example-lead").unwrap()),
            amount_cents: None,
            task: Some(TaskId::new(12)),
        }
    }

    fn at(hours: i64) -> Timestamp {
        Timestamp::from_second(1_800_000_000).unwrap() + SignedDuration::from_hours(hours)
    }

    fn imported() -> ImportedApproval {
        let new = new();
        ImportedApproval {
            agent: new.agent,
            kind: new.kind,
            target: new.target,
            summary: new.summary,
            text: new.text,
            files: vec![ApprovalFile {
                name: "cover.png".to_owned(),
                size: None,
                sha256: Sha256Digest::of(b"png"),
            }],
            approver: new.approver,
            amount_cents: None,
            task: None,
            state: RequestState::Expired,
            state_reason: Some("imported: never answered".to_owned()),
            source_ref: "import:approvals#4".to_owned(),
            created_at: at(0),
            decided: None,
            used: None,
            closed_at: None,
        }
    }

    #[test]
    fn a_good_request() {
        assert_eq!(new().validate(), Ok(()));
        assert_eq!(new().payload_hash(), new().payload().hash());
    }

    /// A change to a good request, and the error it makes.
    type Refused = (fn(&mut NewApproval), RequestError);

    #[test]
    fn bad_requests_are_refused() {
        let cases: [Refused; 8] = [
            (|r| r.target = " ".to_owned(), RequestError::Blank("target")),
            (
                |r| r.summary = String::new(),
                RequestError::Blank("summary"),
            ),
            (|r| r.approver = Approver::Nobody, RequestError::NoApprover),
            (
                |r| r.kind = ActionKind::Payment,
                RequestError::FixedCore(ActionKind::Payment),
            ),
            (|r| r.amount_cents = Some(5), RequestError::Amount),
            (
                |r| r.files.push(FileDigest::of("../a", b"")),
                RequestError::FileName(1),
            ),
            (
                |r| r.files.push(FileDigest::of("invoice\u{202E}fdp.exe", b"")),
                RequestError::FileName(1),
            ),
            (
                |r| {
                    r.files = (0..=FILES_MAX)
                        .map(|i| FileDigest::of(format!("{i}"), b""))
                        .collect();
                },
                RequestError::TooManyFiles,
            ),
        ];
        for (change, error) in cases {
            let mut request = new();
            change(&mut request);
            assert_eq!(request.validate(), Err(error));
        }
        let mut payment = new();
        payment.kind = ActionKind::Payment;
        payment.approver = Approver::Owner;
        assert_eq!(payment.validate(), Err(RequestError::Amount));
        payment.amount_cents = Some(1_000);
        assert_eq!(payment.validate(), Ok(()));
    }

    #[test]
    fn imports_are_over_and_never_match_a_live_hash() {
        assert_eq!(imported().validate(), Ok(()));
        let mut pending = imported();
        pending.state = RequestState::Approved;
        assert_eq!(
            pending.validate(),
            Err(RequestError::ImportedState(RequestState::Approved))
        );
        let mut used = imported();
        used.state = RequestState::Used;
        assert_eq!(used.validate(), Err(RequestError::ImportedUse));
        used.used = Some(UseRecord {
            at: at(1),
            by: "task 12".to_owned(),
        });
        assert_eq!(used.validate(), Ok(()));
        let mut sized = imported();
        sized.files[0].size = Some(3);
        assert_ne!(imported().payload_hash(), sized.payload_hash());
        assert_ne!(imported().payload_hash(), new().payload_hash());
    }

    fn stored(state: RequestState) -> ApprovalRequest {
        let new = new();
        ApprovalRequest {
            id: crate::ApprovalId::new(1),
            payload_hash: new.payload_hash(),
            agent: new.agent,
            kind: new.kind,
            target: new.target,
            summary: new.summary,
            text: new.text,
            files: new.files.into_iter().map(ApprovalFile::from).collect(),
            approver: new.approver,
            amount_cents: None,
            task: new.task,
            state,
            state_reason: None,
            source_ref: None,
            created_at: at(0),
            decided: Some(DecisionRecord {
                decision: Decision::Approve,
                at: at(2),
                by: "U0APPROVER".to_owned(),
                via: DecidedVia::Slack,
            }),
            used: None,
            closed_at: None,
            shown: None,
        }
    }

    #[test]
    fn usable_only_approved_unused_unexpired_same_hash_same_approver() {
        let request = stored(RequestState::Approved);
        let hash = request.payload_hash;
        let approver = request.approver.clone();
        assert!(request.is_usable(at(3), &hash, &approver));
        assert!(!request.is_usable(at(26), &hash, &approver), "expired");
        assert!(!request.is_usable(at(3), &Sha256Digest::of(b"x"), &approver));
        assert!(
            !request.is_usable(at(3), &hash, &Approver::Owner),
            "policy moved"
        );
        assert!(!stored(RequestState::Used).is_usable(at(3), &hash, &approver));
        let mut imported = stored(RequestState::Approved);
        imported.source_ref = Some("import:x#1".to_owned());
        assert!(!imported.is_usable(at(3), &hash, &approver));
    }

    #[test]
    fn open_while_pending_and_unexpired() {
        let mut request = stored(RequestState::Pending);
        request.decided = None;
        assert!(request.is_open(at(71)));
        assert!(!request.is_open(at(72)));
    }
}
