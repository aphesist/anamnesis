//! PDF extraction against a PDF generated in-test with lopdf (offline).

use anamnesis_connectors::connectors::pdf::extract_pdf;
use anamnesis_connectors::{builtin, validated_list};
use anamnesis_core::{ItemKind, Origin};
use lopdf::content::{Content, Operation};
use lopdf::{Document, Object, Stream, dictionary};
use serde_json::json;

/// A minimal PDF: one Helvetica line per page, plus an Info dictionary.
fn make_pdf(pages: &[&str], title: &str) -> Vec<u8> {
    let mut doc = Document::with_version("1.5");
    let pages_id = doc.new_object_id();
    let font_id = doc.add_object(dictionary! {
        "Type" => "Font", "Subtype" => "Type1", "BaseFont" => "Helvetica", "Encoding" => "WinAnsiEncoding",
    });
    let resources_id = doc.add_object(dictionary! { "Font" => dictionary! { "F1" => font_id } });
    let mut kids = Vec::new();
    for text in pages {
        let content = Content {
            operations: vec![
                Operation::new("BT", vec![]),
                Operation::new("Tf", vec!["F1".into(), 14.into()]),
                Operation::new("Td", vec![72.into(), 720.into()]),
                Operation::new("Tj", vec![Object::string_literal(*text)]),
                Operation::new("ET", vec![]),
            ],
        };
        let content_id = doc.add_object(Stream::new(dictionary! {}, content.encode().unwrap()));
        let page_id = doc.add_object(dictionary! {
            "Type" => "Page", "Parent" => pages_id, "Contents" => content_id,
        });
        kids.push(page_id.into());
    }
    let count = kids.len() as i64;
    doc.objects.insert(
        pages_id,
        Object::Dictionary(dictionary! {
            "Type" => "Pages", "Kids" => kids, "Count" => count, "Resources" => resources_id,
            "MediaBox" => vec![0.into(), 0.into(), 612.into(), 792.into()],
        }),
    );
    let catalog_id = doc.add_object(dictionary! { "Type" => "Catalog", "Pages" => pages_id });
    doc.trailer.set("Root", catalog_id);
    let info_id = doc.add_object(dictionary! {
        "Title" => Object::string_literal(title), "Author" => Object::string_literal("Plato"),
    });
    doc.trailer.set("Info", info_id);
    let mut out = Vec::new();
    doc.save_to(&mut out).unwrap();
    out
}

#[test]
fn extracts_pages_with_offsets() {
    let bytes = make_pdf(&["Virtue cannot be taught.", "Then it is recollection."], "Meno");
    assert!(bytes.len() < 20_000);
    let t = extract_pdf(&bytes).unwrap();
    assert_eq!(t.page_count, 2);
    assert_eq!(t.title.as_deref(), Some("Meno"));
    assert_eq!(t.author.as_deref(), Some("Plato"));
    let (n1, s1, e1) = t.pages[0];
    let (n2, s2, e2) = t.pages[1];
    assert_eq!((n1, n2), (1, 2));
    assert!(t.body[s1..e1].contains("Virtue cannot be taught."), "{:?}", t.body);
    assert!(t.body[s2..e2].contains("Then it is recollection."), "{:?}", t.body);
    assert!(e1 <= s2);
}

#[test]
fn garbage_is_an_error_not_a_panic() {
    assert!(extract_pdf(b"%PDF-1.4 this is not really a pdf").is_err());
}

#[tokio::test]
async fn pdf_connector_folder_and_single_file() {
    let tmp = tempfile::tempdir().unwrap();
    std::fs::write(tmp.path().join("meno.pdf"), make_pdf(&["page one", "page two"], "Meno")).unwrap();
    std::fs::write(tmp.path().join("broken.pdf"), b"not a pdf").unwrap();
    std::fs::write(tmp.path().join("notes.md"), "not a pdf either").unwrap();
    let c = builtin("pdf").unwrap();

    let b = validated_list(c.as_ref(), &json!({"path": tmp.path().to_str().unwrap()}), None).await.unwrap();
    assert_eq!(b.upserts.len(), 2);
    let d = b.upserts.iter().find(|d| d.external_id == "meno.pdf").unwrap();
    assert_eq!(d.kind, ItemKind::Pdf);
    assert_eq!(d.origin, Origin::External);
    assert_eq!(d.title, "Meno");
    assert_eq!(d.meta["page_count"], 2);
    assert_eq!(d.meta["pages"][1]["page"], 2);
    let start = d.meta["pages"][1]["start"].as_u64().unwrap() as usize;
    assert!(d.body[start..].contains("page two"));
    let broken = b.upserts.iter().find(|d| d.external_id == "broken.pdf").unwrap();
    assert!(broken.meta["extract_error"].is_string());
    assert!(broken.body.is_empty());

    let again = validated_list(c.as_ref(), &json!({"path": tmp.path().to_str().unwrap()}), b.next_cursor.as_deref()).await.unwrap();
    assert!(again.is_empty());

    let single = tmp.path().join("meno.pdf");
    let b = validated_list(c.as_ref(), &json!({"path": single.to_str().unwrap()}), None).await.unwrap();
    assert_eq!(b.upserts.iter().map(|d| d.external_id.as_str()).collect::<Vec<_>>(), vec!["meno.pdf"]);
}
