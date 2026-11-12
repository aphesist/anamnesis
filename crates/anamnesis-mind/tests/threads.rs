use anamnesis_core::{Id, ThreadKind, ThreadStatus};
use anamnesis_mind::threads::{
    RULE_EXTRACTOR, extract_rules, extraction_prompt, find_verbatim, is_rule_extractor,
    validate_extraction, validate_extraction_str,
};
use chrono::Utc;

fn kinds(text: &str) -> Vec<(ThreadKind, ThreadStatus, String)> {
    extract_rules(&Id("i".into()), text, Utc::now())
        .into_iter()
        .map(|t| (t.kind, t.status, t.quote))
        .collect()
}

#[test]
fn english_rules() {
    let text = "Long day. I decided to move the archive to sqlite. I won't add social login.\n\
                How should I weigh old notes in the review queue?\n\
                - [ ] write the readme\n- [x] fix the importer\n\
                I'll sketch the timeline tomorrow. The weather was fine.\n\
                ```\nlet q = a?b;\nI decided nothing here\n```\n";
    let k = kinds(text);
    assert!(
        k.contains(&(
            ThreadKind::Decision,
            ThreadStatus::Open,
            "I decided to move the archive to sqlite.".into()
        )),
        "{k:?}"
    );
    assert!(k.contains(&(
        ThreadKind::Rejection,
        ThreadStatus::Open,
        "I won't add social login.".into()
    )));
    assert!(k.contains(&(
        ThreadKind::OpenQuestion,
        ThreadStatus::Open,
        "How should I weigh old notes in the review queue?".into()
    )));
    assert!(k.contains(&(
        ThreadKind::Plan,
        ThreadStatus::Open,
        "- [ ] write the readme".into()
    )));
    assert!(k.contains(&(
        ThreadKind::Plan,
        ThreadStatus::Resolved,
        "- [x] fix the importer".into()
    )));
    assert!(k.contains(&(
        ThreadKind::Plan,
        ThreadStatus::Open,
        "I'll sketch the timeline tomorrow.".into()
    )));
    assert!(
        !k.iter().any(|x| x.2.contains("weather")
            || x.2.contains("nothing here")
            || x.2.contains("a?b"))
    );
    // every quote is verbatim
    for (_, _, q) in &k {
        assert!(text.contains(q.as_str()));
    }
}

#[test]
fn portuguese_rules() {
    let text = "Hoje decidi não publicar o ensaio ainda. Decidi usar o modelo local.\n\
                Será que a memória é mesmo reconhecimento\n\
                Vou reler o Mênon amanhã. Não vou mais usar o Notion.\n\
                Todo dia a mesma coisa.\n\
                TODO: revisar as notas de março";
    let k = kinds(text);
    assert!(
        k.contains(&(
            ThreadKind::Rejection,
            ThreadStatus::Open,
            "Hoje decidi não publicar o ensaio ainda.".into()
        )),
        "{k:?}"
    );
    assert!(k.contains(&(
        ThreadKind::Decision,
        ThreadStatus::Open,
        "Decidi usar o modelo local.".into()
    )));
    assert!(k.contains(&(
        ThreadKind::OpenQuestion,
        ThreadStatus::Open,
        "Será que a memória é mesmo reconhecimento".into()
    )));
    assert!(k.contains(&(
        ThreadKind::Plan,
        ThreadStatus::Open,
        "Vou reler o Mênon amanhã.".into()
    )));
    assert!(k.contains(&(
        ThreadKind::Rejection,
        ThreadStatus::Open,
        "Não vou mais usar o Notion.".into()
    )));
    assert!(k.contains(&(
        ThreadKind::Plan,
        ThreadStatus::Open,
        "TODO: revisar as notas de março".into()
    )));
    // "todo dia" (every day) is not a task
    assert!(!k.iter().any(|x| x.2.starts_with("Todo dia")));
}

#[test]
fn model_answers_are_validated() {
    let source =
        "Fiquei pensando.  Decidi usar sqlite\nem vez de postgres. I wonder if offline is enough?";
    let answer = serde_json::json!({"items": [
        {"type": "decision", "statement": "usar sqlite", "quote": "Decidi usar sqlite em vez de postgres.", "status": "open", "confidence": 0.9},
        {"type": "open_question", "statement": "is offline enough?", "quote": "I wonder if offline is enough?", "status": "open", "confidence": 0.8},
        {"type": "plan", "statement": "migrate", "quote": "Vou migrar tudo amanhã.", "status": "open", "confidence": 0.9},
        {"type": "claim", "statement": "x", "quote": "Fiquei pensando.", "status": "open"},
        {"type": "decision", "statement": "weak", "quote": "Fiquei pensando.", "status": "open", "confidence": 0.1}
    ]});
    let rep = validate_extraction(
        source,
        &answer,
        &Id("i".into()),
        Utc::now(),
        "local:qwen3",
        0.5,
    );
    assert_eq!(rep.accepted.len(), 2, "{:?}", rep.rejected);
    // the whitespace-differing quote is mapped back to the exact source span
    assert_eq!(
        rep.accepted[0].quote,
        "Decidi usar sqlite\nem vez de postgres."
    );
    assert!(
        rep.accepted
            .iter()
            .all(|t| source.contains(t.quote.as_str()) && t.extractor == "local:qwen3")
    );
    let reasons: Vec<&str> = rep.rejected.iter().map(|r| r.reason.as_str()).collect();
    assert!(reasons.contains(&"quote is not verbatim in the source"));
    assert!(reasons.contains(&"unknown type"));
    assert!(reasons.contains(&"low confidence"));

    let fenced = "```json\n{\"items\": []}\n```";
    assert!(
        validate_extraction_str(source, fenced, &Id("i".into()), Utc::now(), "x", 0.0)
            .unwrap()
            .accepted
            .is_empty()
    );
    assert!(
        validate_extraction_str(source, "nope", &Id("i".into()), Utc::now(), "x", 0.0).is_err()
    );

    assert_eq!(find_verbatim("a  b c", "a b"), Some((0, 4)));
    assert_eq!(
        find_verbatim("ação  é", "ação é"),
        Some((0, "ação  é".len()))
    );
    let p = extraction_prompt("t", "texto");
    assert!(p.user.contains("texto") && p.schema["properties"]["items"].is_object());
}

fn found(text: &str) -> Vec<(ThreadKind, String)> {
    extract_rules(&Id("i".into()), text, Utc::now())
        .into_iter()
        .map(|t| (t.kind, t.quote))
        .collect()
}

fn assert_kind(text: &str, kind: ThreadKind) {
    let f = found(text);
    assert!(
        f.iter()
            .any(|(k, q)| *k == kind && text.contains(q.as_str())),
        "expected {kind:?} in {text:?}, got {f:?}"
    );
}

fn assert_none(text: &str) {
    let f = found(text);
    assert!(f.is_empty(), "expected nothing in {text:?}, got {f:?}");
}

#[test]
fn open_questions_need_first_person_deliberation_or_a_marker() {
    // true positives, english
    for t in [
        "Should I move the archive to sqlite before the trip?",
        "I wonder whether the review queue needs a random arm.",
        "How do I keep the drafts from piling up?",
        "What if I wrote one page a day instead?",
        "Question: does spaced repetition help with essays",
        "Q: is the latent share stable across months",
        "- [ ] is the importer fast enough?",
        "I haven't decided between the two editions.",
        "## Open questions\n\n- can the layout stay stable when regions merge?\n- is a week enough data?",
        "## open questions\nhow stable is the clustering?",
    ] {
        assert_kind(t, ThreadKind::OpenQuestion);
    }
    // true positives, portuguese
    for t in [
        "Será que vale a pena reler o Mênon agora?",
        "Não sei se mantenho o capítulo dois.",
        "Devo publicar o ensaio antes de março?",
        "Preciso decidir entre as duas editoras.",
        "Pergunta: a memória é reconhecimento",
        "## Perguntas\n\n- a revisão diária funciona?",
        "Me pergunto se o modo offline basta.",
    ] {
        assert_kind(t, ThreadKind::OpenQuestion);
    }
}

#[test]
fn rhetorical_and_quoted_questions_are_not_threads() {
    for t in [
        // a bare question mark, no first person
        "What is the use of a house if you haven't got a tolerable planet to put it on?",
        "How many a man has dated a new era in his life from the reading of a book?",
        // addressed to the reader
        "Should you not rather go to the woods?",
        "Why should we be in such desperate haste to succeed?",
        // negative, rhetorical
        "Shall I not have intelligence with the earth?",
        "Am I not partly leaves and vegetable mould myself?",
        // inside quotation marks or a block quote
        "He asked me, \"Should I sell the farm?\" and I said nothing.",
        "\"Should I go on?\" she said.",
        "> Should I stay or should I go?",
        // a paragraph dense with questions is rhetoric
        "What is nature? What is a man? Should I ask what a river is? Who knows?",
        // portuguese
        "O que é a memória senão reconhecimento?",
        "Você não acha que devo parar?",
        "Não é verdade que a vida é curta?",
        "Ele perguntou: \"Devo ficar?\"",
        // too short
        "ok?",
    ] {
        assert_none(t);
    }
}

#[test]
fn decisions_rejections_and_plans_need_a_first_person_commitment() {
    for (t, k) in [
        (
            "I decided to keep the journal in plain text.",
            ThreadKind::Decision,
        ),
        ("We settled on a weekly review.", ThreadKind::Decision),
        (
            "Decision: the demo ships without login.",
            ThreadKind::Decision,
        ),
        ("Optei pelo modelo local.", ThreadKind::Decision),
        ("Resolvi publicar só em abril.", ThreadKind::Decision),
        (
            "I decided not to publish the second volume.",
            ThreadKind::Rejection,
        ),
        (
            "I'm not going to rewrite the importer.",
            ThreadKind::Rejection,
        ),
        (
            "Desisti de traduzir o livro inteiro.",
            ThreadKind::Rejection,
        ),
        (
            "Não vou fazer login social por enquanto.",
            ThreadKind::Rejection,
        ),
        ("I'll call the printer tomorrow.", ThreadKind::Plan),
        (
            "I plan to walk to the cliffs every morning.",
            ThreadKind::Plan,
        ),
        ("TODO: back up the vault", ThreadKind::Plan),
        ("Vou reler o capítulo amanhã.", ThreadKind::Plan),
        ("Pretendo terminar o rascunho em maio.", ThreadKind::Plan),
    ] {
        assert_kind(t, k);
    }
    for t in [
        // third person, or no commitment
        "He decided to go to Europe.",
        "The town decided to build a new road.",
        "The decision is not mine to make.",
        "I will tell you what I think of the railroad.",
        "I should like to keep some book of natural history.",
        "We need to be awake to the morning.",
        "I want to live deliberately.",
        "Vou te contar uma coisa.",
        "Eu não vou dizer que foi fácil, mas foi bom.",
        // "resolvi" as "i solved"
        "Resolvi o problema da importação.",
        // conditional
        "If I decided to stay, the house would be enough.",
        // quoted
        "\"I decided to leave,\" he said.",
        // "todo dia" is "every day"
        "Todo dia a mesma coisa.",
    ] {
        assert_none(t);
    }
}

#[test]
fn rule_threads_carry_the_extractor_version() {
    let t = extract_rules(&Id("i".into()), "I decided to stay.", Utc::now());
    assert_eq!(t.len(), 1);
    assert_eq!(t[0].extractor, RULE_EXTRACTOR);
    assert!(is_rule_extractor(RULE_EXTRACTOR));
    assert!(is_rule_extractor("rule"));
    assert!(!is_rule_extractor("local:qwen3"));
}

#[test]
fn thread_ids_are_the_same_on_every_device() {
    use anamnesis_mind::threads::thread_id;
    let item = anamnesis_core::Id("01JX0000000000000000000000".into());
    let at = chrono::DateTime::parse_from_rfc3339("2026-03-01T10:00:00Z")
        .unwrap()
        .with_timezone(&chrono::Utc);
    let text = "I decided to walk every morning. Should I keep a separate journal?";
    let a = anamnesis_mind::extract_rules(&item, text, at);
    let b = anamnesis_mind::extract_rules(&item, text, at);
    assert_eq!(a.len(), 2);
    assert_eq!(
        a.iter().map(|t| &t.id).collect::<Vec<_>>(),
        b.iter().map(|t| &t.id).collect::<Vec<_>>()
    );
    assert_eq!(a[0].id, thread_id(&item, a[0].kind, &a[0].quote, at));
    assert_ne!(a[0].id, a[1].id);
    // a ulid whose time is the item's
    let u: ulid::Ulid = a[0].id.0.parse().unwrap();
    assert_eq!(u.timestamp_ms() as i64, at.timestamp_millis());
}
