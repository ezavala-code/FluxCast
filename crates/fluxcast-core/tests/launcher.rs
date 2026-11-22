use fluxcast_core::{
    catalog::{Catalog, Entry, Kind},
    utilities::evaluate,
};
use std::path::PathBuf;
use uuid::Uuid;

fn value(query: &str) -> String {
    evaluate(query).unwrap().unwrap().value
}

#[test]
fn offline_tools_preserve_text_and_convert_in_both_directions() {
    assert_eq!(value("(1920 * 0.75) + 48"), "1488");
    assert_eq!(value("= 2 ^ 3"), "8");
    let text = "  ¡Hola! ";
    assert_eq!(
        value(&format!("unb64 {}", value(&format!("b64 {text}")))),
        text
    );
    assert_eq!(value("hex 255"), "0xFF");
    assert_eq!(value("bin -0xA"), "-0b1010");
    assert_eq!(value("dec 0b1010"), "10");
    assert_eq!(value("ts -1"), "1969-12-31T23:59:59Z");
    assert_eq!(value("ts 1970-01-01T01:00:00+01:00"), "0");
    assert!(value("ts now").parse::<i64>().unwrap() > 1_700_000_000);
    assert_eq!(value("HEX 255"), "0xFF");
    for version in [4, 7] {
        let uuid = Uuid::parse_str(&value(&format!("uuid{version}"))).unwrap();
        assert_eq!(uuid.get_version_num(), version);
    }
    let jwt: serde_json::Value =
        serde_json::from_str(&value("jwt eyJhbGciOiJub25lIn0.eyJzdWIiOiJhIn0.")).unwrap();
    assert_eq!(jwt["header"]["alg"], "none");
    assert_eq!(jwt["payload"]["sub"], "a");
}

#[test]
fn invalid_input_is_reported_and_application_names_remain_searchable() {
    for bad in [
        "= 1/0",
        "= 2+(",
        "= print(1)",
        "ts 9223372036854775807",
        "ts someday",
        "unb64 ",
        "unb64 /w==",
        "unb64 *",
        "dec 0b21",
        "jwt x.y",
        "jwt e30.W10.",
    ] {
        assert!(evaluate(bad).unwrap().is_err(), "{bad}");
    }
    for query in [
        "Firefox",
        "1Password",
        "org.kde.Kate",
        "uuid4 project",
        "hex",
        "bin",
        "dec",
        "ts",
        "jwt",
        "b64",
        "unb64",
        "2024",
    ] {
        assert!(evaluate(query).is_none(), "{query}");
    }
}

#[test]
fn keyword_search_ranks_name_matches_ahead_of_keyword_matches() {
    let cat = Catalog::new(vec![
        Entry::with_keywords(
            "Konsole".into(),
            Kind::App,
            "konsole.desktop".into(),
            "terminal shell console".into(),
        ),
        Entry::new("Terminal".into(), Kind::Project, "Terminal".into()),
    ]);
    let names: Vec<_> = cat
        .search("terminal", None, 10)
        .into_iter()
        .map(|i| cat.get(i).unwrap().name.as_str())
        .collect();
    assert_eq!(names, ["Terminal", "Konsole"]);
    let app_only: Vec<_> = cat
        .search("terminal", Some(Kind::App), 10)
        .into_iter()
        .map(|i| cat.get(i).unwrap().name.as_str())
        .collect();
    assert_eq!(app_only, ["Konsole"]);
}

#[test]
fn fuzzy_search_is_accent_insensitive_filtered_bounded_and_stable() {
    let cat = Catalog::new(
        ["Terminal", "Término", "Term", "Telegram"]
            .map(|s| {
                Entry::new(
                    s.into(),
                    if s == "Terminal" {
                        Kind::Project
                    } else {
                        Kind::App
                    },
                    s.into(),
                )
            })
            .into(),
    );
    let names = |ids: Vec<usize>| {
        ids.iter()
            .map(|&i| cat.get(i).unwrap().name.as_str())
            .collect::<Vec<_>>()
    };
    assert_eq!(names(cat.search("term", None, 1)), ["Term"]);
    assert_eq!(
        names(cat.search("tmnl", Some(Kind::Project), 12)),
        ["Terminal"]
    );
    assert!(names(cat.search("term", Some(Kind::App), 12)).contains(&"Término"));
    assert!(cat.search("", None, 0).is_empty());
    assert!(cat.search("xyz", None, 12).is_empty());
}

#[cfg(target_os = "linux")]
#[test]
fn discover_respects_xdg_masks_flatpaks_and_worktree_files() {
    use fluxcast_core::desktop::discover;
    use std::fs;
    let temp = tempfile::tempdir().unwrap();
    let dir = temp.path();
    let user = dir.join("user");
    let flatpak = dir.join("flatpak/exports/share/applications");
    let projects = dir.join("projects");
    for p in [&user, &flatpak, &projects] {
        fs::create_dir_all(p).unwrap();
    }
    let app = |name, extra| {
        format!("[Desktop Entry]\nType=Application\nName={name}\nExec=/bin/true\n{extra}\n")
    };
    fs::write(flatpak.join("app.desktop"), app("Old", "")).unwrap();
    fs::write(user.join("app.desktop"), app("Override", "")).unwrap();
    fs::create_dir(flatpak.join("sub")).unwrap();
    fs::write(flatpak.join("sub/masked.desktop"), app("Masked", "")).unwrap();
    fs::write(
        user.join("sub-masked.desktop"),
        "[Desktop Entry]\nHidden=true\n",
    )
    .unwrap();
    fs::write(
        flatpak.join("hidden.desktop"),
        app("Hidden", "NoDisplay=true"),
    )
    .unwrap();
    fs::write(flatpak.join("org.test.Flat.desktop"), app("Flat", "")).unwrap();
    fs::create_dir_all(projects.join("nested/repo/.git")).unwrap();
    fs::create_dir_all(projects.join("node_modules/vendor/.git")).unwrap();
    fs::create_dir_all(projects.join("work tree")).unwrap();
    fs::write(projects.join("work tree/.git"), "gitdir: ../store").unwrap();
    std::os::unix::fs::symlink(&projects, projects.join("cycle")).unwrap();
    let cat = discover(
        &[user, flatpak],
        &[projects.clone(), projects.join("nested/repo")],
    );
    let names: Vec<_> = cat
        .search("", None, 32)
        .into_iter()
        .map(|i| &cat.get(i).unwrap().name)
        .collect();
    assert_eq!(names, ["Flat", "Override", "repo", "work tree"]);
}

#[test]
#[ignore = "Medición local de latencia en modo release"]
fn search_latency() {
    use std::{
        hint::black_box,
        time::{Duration, Instant},
    };
    let catalog = Catalog::new(
        (0..5000)
            .map(|i| {
                Entry::new(
                    format!("Proyecto FluxCast {i}"),
                    Kind::Project,
                    PathBuf::from(i.to_string()),
                )
            })
            .collect(),
    );
    let mut samples = Vec::new();
    for query in ["flxc", "prj 42", "cast", "proyecto", "xyz"]
        .iter()
        .cycle()
        .take(250)
    {
        let start = Instant::now();
        black_box(catalog.search(black_box(query), None, 32));
        samples.push(start.elapsed());
    }
    samples.sort();
    let p95 = samples[237];
    println!(
        "5000 entradas, 250 consultas, búsqueda en memoria: p50 {:?}, p95 {p95:?}",
        samples[124]
    );
    assert!(p95 < Duration::from_millis(15));
}
