use anamnesis_core::{Id, ThreadKind, ThreadStatus};
use anamnesis_mind::threads::{extract_rules, extraction_prompt, find_verbatim, validate_extraction, validate_extraction_str};
use chrono::Utc;

fn kinds(text: &str) -> Vec<(ThreadKind, ThreadStatus, String)> {
    extract_rules(&Id("i".into()), text, Utc::now()).into_iter().map(|t| (t.kind, t.status, t.quote)).collect()
}

#[test]
fn english_rules() {
    let text = "Long day. I decided to move the archive to sqlite. I won't add social login.\n\
                How should the review queue weigh old notes?\n\
                - [ ] write the readme\n- [x] fix the importer\n\
                I'll sketch the timeline tomorrow. The weather was fine.\n\
                ```\nlet q = a?b;\nI decided nothing here\n```\n";
    let k = kinds(text);
    assert!(k.contains(&(ThreadKind::Decision, ThreadStatus::Open, "I decided to move the archive to sqlite.".into())), "{k:?}");
    assert!(k.contains(&(ThreadKind::Rejection, ThreadStatus::Open, "I won't add social login.".into())));
    assert!(k.contains(&(ThreadKind::OpenQuestion, ThreadStatus::Open, "How should the review queue weigh old notes?".into())));
    assert!(k.contains(&(ThreadKind::Plan, ThreadStatus::Open, "- [ ] write the readme".into())));
    assert!(k.contains(&(ThreadKind::Plan, ThreadStatus::Resolved, "- [x] fix the importer".into())));
    assert!(k.contains(&(ThreadKind::Plan, ThreadStatus::Open, "I'll sketch the timeline tomorrow.".into())));
    assert!(!k.iter().any(|x| x.2.contains("weather") || x.2.contains("nothing here") || x.2.contains("a?b")));
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
    assert!(k.contains(&(ThreadKind::Rejection, ThreadStatus::Open, "Hoje decidi não publicar o ensaio ainda.".into())), "{k:?}");
    assert!(k.contains(&(ThreadKind::Decision, ThreadStatus::Open, "Decidi usar o modelo local.".into())));
    assert!(k.contains(&(ThreadKind::OpenQuestion, ThreadStatus::Open, "Será que a memória é mesmo reconhecimento".into())));
    assert!(k.contains(&(ThreadKind::Plan, ThreadStatus::Open, "Vou reler o Mênon amanhã.".into())));
    assert!(k.contains(&(ThreadKind::Rejection, ThreadStatus::Open, "Não vou mais usar o Notion.".into())));
    assert!(k.contains(&(ThreadKind::Plan, ThreadStatus::Open, "TODO: revisar as notas de março".into())));
    // "todo dia" (every day) is not a task
    assert!(!k.iter().any(|x| x.2.starts_with("Todo dia")));
}

#[test]
fn model_answers_are_validated() {
    let source = "Fiquei pensando.  Decidi usar sqlite\nem vez de postgres. I wonder if offline is enough?";
    let answer = serde_json::json!({"items": [
        {"type": "decision", "statement": "usar sqlite", "quote": "Decidi usar sqlite em vez de postgres.", "status": "open", "confidence": 0.9},
        {"type": "open_question", "statement": "is offline enough?", "quote": "I wonder if offline is enough?", "status": "open", "confidence": 0.8},
        {"type": "plan", "statement": "migrate", "quote": "Vou migrar tudo amanhã.", "status": "open", "confidence": 0.9},
        {"type": "claim", "statement": "x", "quote": "Fiquei pensando.", "status": "open"},
        {"type": "decision", "statement": "weak", "quote": "Fiquei pensando.", "status": "open", "confidence": 0.1}
    ]});
    let rep = validate_extraction(source, &answer, &Id("i".into()), Utc::now(), "local:qwen3", 0.5);
    assert_eq!(rep.accepted.len(), 2, "{:?}", rep.rejected);
    // the whitespace-differing quote is mapped back to the exact source span
    assert_eq!(rep.accepted[0].quote, "Decidi usar sqlite\nem vez de postgres.");
    assert!(rep.accepted.iter().all(|t| source.contains(t.quote.as_str()) && t.extractor == "local:qwen3"));
    let reasons: Vec<&str> = rep.rejected.iter().map(|r| r.reason.as_str()).collect();
    assert!(reasons.contains(&"quote is not verbatim in the source"));
    assert!(reasons.contains(&"unknown type"));
    assert!(reasons.contains(&"low confidence"));

    let fenced = "```json\n{\"items\": []}\n```";
    assert!(validate_extraction_str(source, fenced, &Id("i".into()), Utc::now(), "x", 0.0).unwrap().accepted.is_empty());
    assert!(validate_extraction_str(source, "nope", &Id("i".into()), Utc::now(), "x", 0.0).is_err());

    assert_eq!(find_verbatim("a  b c", "a b"), Some((0, 4)));
    assert_eq!(find_verbatim("ação  é", "ação é"), Some((0, "ação  é".len())));
    let p = extraction_prompt("t", "texto");
    assert!(p.user.contains("texto") && p.schema["properties"]["items"].is_object());
}
