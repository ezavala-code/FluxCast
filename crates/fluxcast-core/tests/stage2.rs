use chrono::{TimeZone, Utc};
use fluxcast_core::{
    classifier::{classify, plain_text, ContentType},
    store::{SnippetInput, Store},
    templates::expand_at,
};
use uuid::Uuid;

#[test]
fn classifier_detects_json_url_code_color_jwt_and_text() {
    let json = classify(r#"{"service":"fluxcast","port":8080}"#);
    assert_eq!(json.content_type, ContentType::Json);
    assert!(json
        .formatted
        .as_deref()
        .unwrap()
        .contains("  \"service\": \"fluxcast\""));

    let url = classify("https://fedoraproject.org/kde/");
    assert_eq!(url.content_type, ContentType::Url);

    let hex_color = classify("#3DAEE9");
    assert_eq!(hex_color.content_type, ContentType::Color);
    assert_eq!(hex_color.color_swatch.as_deref(), Some("#3daee9"));

    let rgb_color = classify("rgb(61, 174, 233)");
    assert_eq!(rgb_color.content_type, ContentType::Color);
    assert_eq!(rgb_color.color_swatch.as_deref(), Some("rgb(61, 174, 233)"));

    let jwt = classify("eyJhbGciOiJub25lIn0.eyJzdWIiOiJ6YXZhbGEifQ.");
    assert_eq!(jwt.content_type, ContentType::Jwt);
    assert!(jwt.formatted.as_deref().unwrap().contains("\"zavala\""));

    let rust_code = classify("pub fn main() {\n    let mut x = 42;\n}");
    assert_eq!(rust_code.content_type, ContentType::Code);
    assert_eq!(rust_code.language, Some("rust"));

    let plain = classify("Nota rápida de reunión con el equipo");
    assert_eq!(plain.content_type, ContentType::Text);

    assert_eq!(
        plain_text("  \u{1b}[32mHola\u{1b}[0m\r\nMundo  "),
        "Hola\nMundo"
    );
}

#[test]
fn sqlite_fts5_clipboard_store_deduplicates_pins_searches_and_queues_outbox() {
    let store = Store::open_in_memory().unwrap();
    let first = store
        .record_clipboard("sudo dnf install webkit2gtk4.1-devel")
        .unwrap()
        .unwrap();
    let second = store
        .record_clipboard(r#"{"wayland":"ext-data-control-v1"}"#)
        .unwrap()
        .unwrap();
    assert_eq!(second.content_type, ContentType::Json);

    // Copiar de nuevo el primer texto actualiza el registro existente sin duplicar filas.
    let deduped = store
        .record_clipboard("sudo dnf install webkit2gtk4.1-devel")
        .unwrap()
        .unwrap();
    assert_eq!(deduped.id, first.id);

    let all = store.list_clipboard("", false, 20).unwrap();
    assert_eq!(all.len(), 2);
    assert_eq!(all[0].id, first.id);

    // Búsqueda FTS5 encuentra por término parcial.
    let fts_hits = store.list_clipboard("webkit2gtk", false, 20).unwrap();
    assert_eq!(fts_hits.len(), 1);
    assert_eq!(fts_hits[0].id, first.id);

    // Fijar el segundo ítem lo coloca primero.
    assert!(store.toggle_pin_clipboard(&second.id).unwrap());
    let ordered = store.list_clipboard("", false, 20).unwrap();
    assert_eq!(ordered[0].id, second.id);
    assert!(ordered[0].is_pinned);

    // Limpiar no fijados conserva los fijados.
    assert_eq!(store.clear_unpinned_clipboard().unwrap(), 1);
    let remaining = store.list_clipboard("", false, 20).unwrap();
    assert_eq!(remaining.len(), 1);
    assert_eq!(remaining[0].id, second.id);

    assert!(store.outbox_count().unwrap() >= 2);
}

#[test]
fn snippets_crud_templates_tags_and_frecency_ranking_work() {
    let store = Store::open_in_memory().unwrap();
    let s1 = store
        .save_snippet(SnippetInput {
            id: None,
            title: "Limpiar Docker".into(),
            keyword: Some("dprune".into()),
            body: "docker system prune -af # {{date:YYYY-MM-DD}} {{clipboard}} {{uuid}}".into(),
            language: Some("shell".into()),
            tags: vec!["Docker".into(), "Fedora".into()],
        })
        .unwrap();
    assert_eq!(s1.keyword.as_deref(), Some("!dprune"));
    assert_eq!(s1.tags, vec!["docker", "fedora"]);

    let s2 = store
        .save_snippet(SnippetInput {
            id: None,
            title: "Logs Docker Compose".into(),
            keyword: Some("!dlogs".into()),
            body: "docker compose logs -f {{clipboard}}".into(),
            language: Some("shell".into()),
            tags: vec!["docker".into()],
        })
        .unwrap();

    // Búsqueda directa por alias (!dprune).
    let by_kw = store.find_snippet_by_keyword("!dprune").unwrap().unwrap();
    assert_eq!(by_kw.id, s1.id);

    // Usar s2 varias veces incrementa su frecuencia (use_count) y lo posiciona arriba.
    for _ in 0..3 {
        let expanded = store.use_snippet(&s2.id, "api-service").unwrap();
        assert_eq!(expanded, "docker compose logs -f api-service");
    }

    let ranked = store.list_snippets("docker", Some("docker")).unwrap();
    assert_eq!(ranked.len(), 2);
    assert_eq!(ranked[0].id, s2.id);
    assert_eq!(ranked[0].use_count, 3);

    // Verificar expansión determinística de plantillas.
    let fixed_date = Utc.with_ymd_and_hms(2026, 11, 21, 23, 30, 0).unwrap();
    let rendered = expand_at(
        "fecha={{date:YYYY-MM-DD HH:mm}} clip={{clipboard}} id={{uuid7}}",
        "kwin_wayland",
        fixed_date,
    );
    assert!(rendered.starts_with("fecha=2026-11-21 23:30 clip=kwin_wayland id="));
    let uuid_part = rendered.split("id=").nth(1).unwrap();
    assert_eq!(Uuid::parse_str(uuid_part).unwrap().get_version_num(), 7);

    store.delete_snippet(&s1.id).unwrap();
    assert!(store.find_snippet_by_keyword("!dprune").unwrap().is_none());
}
