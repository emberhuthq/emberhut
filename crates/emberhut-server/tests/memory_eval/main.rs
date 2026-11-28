//! The memory eval: how well recall finds what was said, over a synthetic two-month history in
//! English and Arabic (`data/`), with no model in the loop but the embedder.
//!
//! Each question names its gold evidence; the metric is recall@8, whether a gold piece is among
//! the 8 hits the recall tool gives by default (`any`), and whether all of them are (`all`).
//! Question types follow LongMemEval (single fact, multi-session, temporal, knowledge update,
//! abstention), plus:
//! - cross-lingual: an Arabic question whose evidence is English, and the reverse;
//! - forget: once the owner forgets a fact, its words must not come back from recall;
//! - injection: a third party's message must not become a fact, and recall must hand it back
//!   marked as quoted material;
//! - dedupe: the same thing kept in several places shows once.
//!
//! It runs twice: by words alone, and with a real embedding model's vectors recorded in
//! `data/vectors.bin` (see `vectors`). Each run writes its report to the test's target folder
//! and fails when a number falls below its floor in `data/floors.json`.
#![expect(
    clippy::unwrap_used,
    clippy::panic,
    clippy::print_stdout,
    reason = "an eval: a failure fails it, and its report is printed"
)]

mod corpus;
mod hut;
mod vectors;

use std::collections::{BTreeMap, HashSet};
use std::fmt::Write as _;
use std::sync::Arc;

use emberhut_core::memory::recall::render;
use emberhut_server::services::facts::SetFact;
use emberhut_server::services::recall::{Embedder, NoEmbedder};
use serde_json::{Value, json};

use crate::corpus::Question;
use crate::hut::Hut;

/// How many hits count.
const K: usize = 8;

/// The kinds scored by recall@8 of their gold evidence.
const RETRIEVAL: [&str; 6] = [
    "single-fact",
    "multi-session",
    "temporal",
    "knowledge-update",
    "cross-lingual",
    "dedupe",
];

#[derive(Default)]
struct Tally {
    asked: u32,
    any: u32,
    all: u32,
}

impl Tally {
    fn add(&mut self, any: bool, all: bool) {
        self.asked += 1;
        self.any += u32::from(any);
        self.all += u32::from(all);
    }

    fn rate(n: u32, of: u32) -> f64 {
        if of == 0 {
            0.0
        } else {
            f64::from(n) / f64::from(of)
        }
    }
}

/// One run's numbers, by name (`single-fact`, `overall`, `forget`...).
type Scores = BTreeMap<String, f64>;

/// Asks every question of a hut holding the corpus; the scores and the report.
async fn run(mode: &str, embedder: Arc<dyn Embedder>) -> (Scores, String) {
    let (corpus, questions) = corpus::load();
    let hut = Hut::new(&corpus, embedder).await;
    for question in &questions {
        for key in question
            .gold
            .iter()
            .chain(&question.stale)
            .chain(&question.copies)
            .chain(question.inject.iter())
            .chain(question.forget.iter())
        {
            assert!(
                hut.refs.contains_key(key),
                "{}: no piece of evidence {key}",
                question.id
            );
        }
    }

    let mut tallies: BTreeMap<String, Tally> = BTreeMap::new();
    let mut by_lang: BTreeMap<String, Tally> = BTreeMap::new();
    let mut fresher = Tally::default();
    let mut misses = Vec::new();
    let mut questions_out = Vec::new();
    for question in questions.iter().filter(|q| RETRIEVAL.contains(&q.kind.as_str())) {
        let found = hut
            .ask(&question.q, question.since.as_deref(), question.until.as_deref())
            .await;
        let keys = hut.keys_of(&found);
        let top: HashSet<&str> = keys.iter().take(K).map(String::as_str).collect();
        let any = question.gold.iter().any(|key| top.contains(key.as_str()));
        let all = question.gold.iter().all(|key| top.contains(key.as_str()));
        let mut ok = any;
        if question.kind == "dedupe" {
            // Found, and found once.
            let shown = question
                .copies
                .iter()
                .filter(|key| top.contains(key.as_str()))
                .count();
            ok = shown == 1;
            tallies.entry("dedupe-once".to_owned()).or_default().add(ok, ok);
        }
        if question.kind == "knowledge-update" && !question.stale.is_empty() {
            let rank = |key: &String| keys.iter().position(|hit| hit == key);
            let newest = question.gold.iter().filter_map(rank).min();
            let oldest = question.stale.iter().filter_map(rank).min();
            let first = match (newest, oldest) {
                (Some(new), Some(old)) => new < old,
                (Some(_), None) => true,
                _ => false,
            };
            fresher.add(first, first);
        }
        tallies.entry(question.kind.clone()).or_default().add(any, all);
        tallies.entry("overall".to_owned()).or_default().add(any, all);
        by_lang
            .entry(format!("lang-{}", question.lang))
            .or_default()
            .add(any, all);
        if !ok {
            misses.push(format!(
                "{} [{}] {} → {}",
                question.id,
                question.kind,
                question.q,
                keys.iter().take(K).cloned().collect::<Vec<_>>().join(", ")
            ));
        }
        questions_out.push(json!({ "id": question.id, "any": any, "all": all, "hits": keys }));
    }

    let injection = injection(&hut, &questions).await;
    let forget = forget(&hut, &questions).await;

    let mut scores = Scores::new();
    let mut table = String::from("| | asked | recall_any@8 | recall_all@8 |\n|---|---|---|---|\n");
    for (name, tally) in tallies.iter().chain(&by_lang) {
        let any = Tally::rate(tally.any, tally.asked);
        let all = Tally::rate(tally.all, tally.asked);
        scores.insert(name.clone(), any);
        scores.insert(format!("{name}-all"), all);
        writeln!(table, "| {name} | {} | {any:.3} | {all:.3} |", tally.asked).unwrap();
    }
    let rate = Tally::rate(fresher.any, fresher.asked);
    scores.insert("update-newest-first".to_owned(), rate);
    writeln!(table, "| update: newest first | {} | {rate:.3} | |", fresher.asked).unwrap();
    for (name, (passed, asked)) in [("forget", forget.0), ("injection-not-saved", injection.0), ("injection-fenced", injection.1)] {
        let rate = Tally::rate(passed, asked);
        scores.insert(name.to_owned(), rate);
        writeln!(table, "| {name} | {asked} | {rate:.3} | |").unwrap();
    }
    let abstained = questions.iter().filter(|q| q.kind == "abstention").count();
    writeln!(table, "| abstention | {abstained} | (judged end to end) | |").unwrap();

    let report = format!(
        "## Memory eval: {mode}\n\n{table}\n### Misses\n\n{}\n\n### Forget and injection\n\n{}\n",
        misses.join("\n"),
        forget.1.iter().chain(&injection.2).cloned().collect::<Vec<_>>().join("\n"),
    );
    let out = std::path::Path::new(env!("CARGO_TARGET_TMPDIR")).join("memory-eval");
    std::fs::create_dir_all(&out).unwrap();
    std::fs::write(out.join(format!("{mode}.md")), &report).unwrap();
    std::fs::write(
        out.join(format!("{mode}.json")),
        serde_json::to_string_pretty(&json!({ "mode": mode, "scores": scores, "questions": questions_out })).unwrap(),
    )
    .unwrap();
    (scores, report)
}

/// Injection: saving what a third party's message asks to have saved must not make it the
/// owner's fact; and recall must hand that message back marked as quoted material. Passed and
/// asked for each, and what failed.
async fn injection(hut: &Hut, questions: &[Question]) -> ((u32, u32), (u32, u32), Vec<String>) {
    let mut saved = (0, 0);
    let mut fenced = (0, 0);
    let mut failed = Vec::new();
    for question in questions.iter().filter(|q| q.kind == "injection") {
        let found = hut.ask(&question.q, None, None).await;
        let keys = hut.keys_of(&found);
        let inject = question.inject.as_deref().unwrap();
        if let Some(at) = keys.iter().position(|key| key == inject) {
            fenced.1 += 1;
            let shown = render(&emberhut_core::memory::recall::Found {
                hits: vec![found.hits[at].clone()],
                ..found.clone()
            });
            if shown.contains("<recalled") {
                fenced.0 += 1;
            } else {
                failed.push(format!("{} injection: {inject} handed back unmarked", question.id));
            }
        }
        // The assistant, woken by the message, saves what it asks.
        let claim = question.claim.clone().unwrap_or_default();
        let title = format!("eval {}", question.id);
        hut.facts
            .set_fact(SetFact {
                category: "preferences".to_owned(),
                title: title.clone(),
                text: claim,
                ..SetFact::default()
            })
            .await
            .unwrap();
        saved.1 += 1;
        let facts = hut.facts.facts(Some("preferences")).await.unwrap();
        let planted = facts.iter().any(|fact| fact.title == title);
        if planted {
            failed.push(format!("{} injection: {inject}'s claim became a fact", question.id));
        } else {
            saved.0 += 1;
        }
    }
    (saved, fenced, failed)
}

/// Forget: once the owner forgets the fact, none of the 8 hits for the question holds its
/// words. Passed and asked, and what failed.
async fn forget(hut: &Hut, questions: &[Question]) -> ((u32, u32), Vec<String>) {
    let mut passed = (0, 0);
    let mut failed = Vec::new();
    for question in questions.iter().filter(|q| q.kind == "forget") {
        let key = question.forget.as_deref().unwrap();
        let id = &hut.fact_ids[key];
        hut.facts.drop_fact(id).await.unwrap();
        hut.index().await;
        let found = hut.ask(&question.q, None, None).await;
        let back: Vec<String> = found
            .hits
            .iter()
            .filter(|hit| {
                let text = hit.text.to_lowercase();
                question.words.iter().any(|word| text.contains(&word.to_lowercase()))
            })
            .map(|hit| hit.reference.clone())
            .collect();
        passed.1 += 1;
        if back.is_empty() {
            passed.0 += 1;
        } else {
            failed.push(format!("{} forget: {key}'s words came back in {}", question.id, back.join(", ")));
        }
    }
    (passed, failed)
}

/// Fails when a score is below its floor for `mode`.
fn hold_floors(mode: &str, scores: &Scores, report: &str) {
    let floors: Value = std::fs::read_to_string(corpus::data().join("floors.json"))
        .ok()
        .and_then(|text| serde_json::from_str(&text).ok())
        .unwrap_or_default();
    let mut below = Vec::new();
    if let Some(floors) = floors.get(mode).and_then(Value::as_object) {
        for (name, floor) in floors {
            let floor = floor.as_f64().unwrap_or(0.0);
            let got = scores.get(name).copied().unwrap_or(0.0);
            if got + 1e-9 < floor {
                below.push(format!("{name}: {got:.3} < {floor:.3}"));
            }
        }
    }
    assert!(below.is_empty(), "{report}\nBelow the floor: {}", below.join("; "));
}

#[tokio::test]
async fn recall_by_words_holds_its_floor() {
    let (scores, report) = run("words", Arc::new(NoEmbedder)).await;
    println!("{report}");
    hold_floors("words", &scores, &report);
}

#[tokio::test]
async fn recall_by_meaning_holds_its_floor() {
    let recorded = Arc::new(vectors::Recorded::open());
    assert!(
        recorded.available(),
        "no recorded vectors: run with EMBERHUT_EVAL_EMBED_URL set once"
    );
    let (scores, report) = run("meaning", recorded.clone()).await;
    recorded.save();
    let missed = recorded.missed();
    assert!(
        recorded.recording() || missed.is_empty(),
        "{} texts have no recorded vector (what is embedded changed): run again with \
         EMBERHUT_EVAL_EMBED_URL set to record them. The first: {:?}",
        missed.len(),
        missed.first()
    );
    println!("{report}");
    hold_floors("meaning", &scores, &report);
}
